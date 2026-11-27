//! Two-factor sign-in with an authenticator app (TOTP, RFC 6238: HMAC-SHA1, 30-second steps, 6 digits) and single-use
//! recovery codes. Each code works once (the last step used is kept). Accounts that sign in with Nextcloud use
//! Nextcloud's own two-factor sign-in.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use laklak_proto::{Id, TotpCode, TotpEnabled, TotpSetup};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::auth::{Auth, hash_token};
use crate::{ApiErr, R, St, bad, conflict, now_ms};

const STEP_SECS: u64 = 30;

/// The 6-digit code for one time step (RFC 4226's dynamic truncation).
fn code_at(secret: &[u8], step: u64) -> u32 {
    use hmac::{KeyInit, Mac};
    let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(secret).expect("HMAC takes any key");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]]) % 1_000_000
}

/// The step a typed code belongs to (now, or one step either side for a phone clock that's a little off), if it
/// comes after `after`: an older or the same step was already used.
fn matching_step(secret: &[u8], typed: &str, now_secs: u64, after: u64) -> Option<u64> {
    let digits: String = typed.chars().filter(char::is_ascii_digit).collect();
    let code: u32 = (digits.len() == 6).then(|| digits.parse().ok()).flatten()?;
    let now = now_secs / STEP_SECS;
    [now.saturating_sub(1), now, now + 1].into_iter().find(|&s| s > after && code_at(secret, s) == code)
}

/// Recovery codes are compared without dashes, spaces or case.
fn recovery_key(typed: &str) -> String {
    typed.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase()
}

fn recovery_code() -> String {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..10)
        .map(|i| {
            let c = CHARS[rand::random::<u32>() as usize % CHARS.len()] as char;
            if i == 5 { format!("-{c}") } else { c.to_string() }
        })
        .collect()
}

pub enum Second {
    /// No second factor on, or the right code.
    Passed,
    /// On, and no code sent: ask for one.
    Needed,
    Wrong,
}

pub fn needed() -> ApiErr {
    ApiErr {
        code: Some("totp_required"),
        ..ApiErr::new(StatusCode::UNAUTHORIZED, "enter the code from your authenticator app")
    }
}

pub fn wrong() -> ApiErr {
    ApiErr { code: Some("totp_wrong"), ..ApiErr::new(StatusCode::UNAUTHORIZED, "that code is wrong") }
}

/// The second factor of an account, after its password (or reset code) was right: an authenticator code, used up
/// for its time step, or a recovery code, used up for good.
pub async fn check(st: &St, user: Id, typed: Option<&str>) -> R<Second> {
    let (secret, enabled, last): (Option<String>, bool, i64) =
        sqlx::query_as("SELECT totp_secret, totp_enabled, totp_last_step FROM users WHERE id = ?")
            .bind(user)
            .fetch_one(&st.db)
            .await?;
    let (true, Some(secret)) = (enabled, secret) else {
        return Ok(Second::Passed);
    };
    let Some(typed) = typed.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(Second::Needed);
    };
    let secret = data_encoding::BASE32_NOPAD.decode(secret.as_bytes()).unwrap_or_default();
    if let Some(step) = matching_step(&secret, typed, (now_ms() / 1000) as u64, last as u64) {
        // Two sign-ins racing with the same code: only one moves the step on.
        let moved = sqlx::query("UPDATE users SET totp_last_step = ? WHERE id = ? AND totp_last_step < ?")
            .bind(step as i64)
            .bind(user)
            .bind(step as i64)
            .execute(&st.db)
            .await?
            .rows_affected();
        return Ok(if moved == 1 { Second::Passed } else { Second::Wrong });
    }
    let used = sqlx::query("DELETE FROM totp_recovery WHERE user_id = ? AND code_hash = ?")
        .bind(user)
        .bind(hash_token(&recovery_key(typed)))
        .execute(&st.db)
        .await?
        .rows_affected();
    if used == 1 {
        tracing::info!("user {user} used a two-factor recovery code");
        return Ok(Second::Passed);
    }
    Ok(Second::Wrong)
}

/// Turns it off and forgets the secret and recovery codes (also when an admin resets the password).
pub async fn clear(st: &St, user: Id) -> R<()> {
    sqlx::query("UPDATE users SET totp_secret = NULL, totp_enabled = 0, totp_last_step = 0 WHERE id = ?")
        .bind(user)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM totp_recovery WHERE user_id = ?").bind(user).execute(&st.db).await?;
    Ok(())
}

/// A new secret for the authenticator app, as text, an `otpauth://` link and a QR code. Not on until confirmed.
pub async fn setup(State(st): State<St>, auth: Auth) -> R<Json<TotpSetup>> {
    let (username, nextcloud, enabled): (String, bool, bool) =
        sqlx::query_as("SELECT username, nc_user_id IS NOT NULL, totp_enabled FROM users WHERE id = ?")
            .bind(auth.user)
            .fetch_one(&st.db)
            .await?;
    if nextcloud {
        return Err(bad("accounts that sign in with Nextcloud use Nextcloud's two-factor sign-in"));
    }
    if enabled {
        return Err(conflict("two-factor sign-in is already on"));
    }
    let secret = data_encoding::BASE32_NOPAD.encode(&rand::random::<[u8; 20]>());
    sqlx::query("UPDATE users SET totp_secret = ?, totp_last_step = 0 WHERE id = ?")
        .bind(&secret)
        .bind(auth.user)
        .execute(&st.db)
        .await?;
    let enc = |s: &str| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string();
    let issuer = enc(&st.cfg.name);
    let uri = format!(
        "otpauth://totp/{issuer}:{}?secret={secret}&issuer={issuer}&algorithm=SHA1&digits=6&period=30",
        enc(&username)
    );
    let qr_svg = qrcode::QrCode::new(uri.as_bytes())
        .map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).build())
        .unwrap_or_default();
    Ok(Json(TotpSetup { secret, uri, qr_svg }))
}

/// Confirms the setup with a code from the app: on, plus 8 recovery codes, shown this once.
pub async fn enable(State(st): State<St>, auth: Auth, Json(req): Json<TotpCode>) -> R<Json<TotpEnabled>> {
    let (username, secret, enabled): (String, Option<String>, bool) =
        sqlx::query_as("SELECT username, totp_secret, totp_enabled FROM users WHERE id = ?")
            .bind(auth.user)
            .fetch_one(&st.db)
            .await?;
    if enabled {
        return Err(conflict("two-factor sign-in is already on"));
    }
    let secret = secret.ok_or_else(|| bad("set up two-factor sign-in first"))?;
    let secret = data_encoding::BASE32_NOPAD.decode(secret.as_bytes()).unwrap_or_default();
    let step = matching_step(&secret, &req.code, (now_ms() / 1000) as u64, 0)
        .ok_or_else(|| bad("that code is wrong: check the time on your phone and try the next one"))?;
    let codes: Vec<String> = (0..8).map(|_| recovery_code()).collect();
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE users SET totp_enabled = 1, totp_last_step = ? WHERE id = ?")
        .bind(step as i64)
        .bind(auth.user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM totp_recovery WHERE user_id = ?").bind(auth.user).execute(&mut *tx).await?;
    for c in &codes {
        sqlx::query("INSERT INTO totp_recovery (user_id, code_hash) VALUES (?, ?)")
            .bind(auth.user)
            .bind(hash_token(&recovery_key(c)))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    tracing::info!("{username} turned on two-factor sign-in");
    Ok(Json(TotpEnabled { recovery_codes: codes }))
}

/// Turns it off: needs a current code (or a recovery code), so a stolen session alone can't. Wrong codes count like
/// failed sign-ins.
pub async fn disable(State(st): State<St>, auth: Auth, Json(req): Json<TotpCode>) -> R<StatusCode> {
    let username: String =
        sqlx::query_scalar("SELECT username FROM users WHERE id = ?").bind(auth.user).fetch_one(&st.db).await?;
    {
        let mut fails = st.login_fails.lock().unwrap();
        let list = fails.entry(username.clone()).or_default();
        list.retain(|t| t.elapsed().as_secs() < 600);
        if list.len() >= 10 {
            return Err(ApiErr::new(StatusCode::TOO_MANY_REQUESTS, "too many attempts, try again in a few minutes"));
        }
    }
    match check(&st, auth.user, Some(&req.code)).await? {
        Second::Passed | Second::Needed => {}
        Second::Wrong => {
            st.login_fails.lock().unwrap().entry(username).or_default().push(std::time::Instant::now());
            return Err(wrong());
        }
    }
    clear(&st, auth.user).await?;
    tracing::info!("{username} turned off two-factor sign-in");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_6238_codes() {
        // RFC 6238 appendix B, SHA-1 secret, last 6 of its 8 digits.
        let secret = b"12345678901234567890";
        assert_eq!(code_at(secret, 59 / 30), 287_082);
        assert_eq!(code_at(secret, 1_111_111_109 / 30), 81_804);
        assert_eq!(code_at(secret, 1_234_567_890 / 30), 5_924);
        // A code matches its step or a neighbour, once: not a step already used.
        assert_eq!(matching_step(secret, "287 082", 59, 0), Some(1));
        assert_eq!(matching_step(secret, "287082", 89, 0), Some(1), "one step late is fine");
        assert_eq!(matching_step(secret, "287082", 59, 1), None, "step 1 was used");
        assert_eq!(matching_step(secret, "287083", 59, 0), None);
        assert_eq!(recovery_key(" AB2cd-EF3gh "), "ab2cdef3gh");
        assert_eq!(recovery_code().len(), 11);
    }
}
