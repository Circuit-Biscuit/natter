//! laklak-server: HTTP API, WebSocket gateway, SQLite storage and the call SFU, in one process.
//! `main.rs` only reads the environment and calls [`serve`]. See `docs/ARCHITECTURE.md`.

mod auth;
mod db;
mod embeds;
mod games;
mod gateway;
mod mail;
mod messages;
mod nextcloud;
mod sfu;
mod spaces;
mod voice;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use laklak_proto::{ApiError, EPOCH_MS, Id, Millis, Registration};
use sqlx::SqlitePool;
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;
use tower_http::trace::TraceLayer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub data: PathBuf,
    pub name: String,
    pub registration: Registration,
    pub max_upload: u64,
    pub media_port: u16,
    /// Most a camera may send in a call, kbit/s (`LAKLAK_CAMERA_KBPS`, 0 = no cap).
    pub camera_kbps: u32,
    /// Daily Word's time zone: minutes ahead of UTC (`LAKLAK_GAME_UTC_OFFSET`, default 60 = Central Europe).
    pub game_utc_offset_min: i64,
    pub public_ip: Option<IpAddr>,
    pub web_dir: Option<PathBuf>,
    pub nextcloud_url: Option<String>,
    pub nextcloud_group: Option<String>,
    pub nextcloud_folder: String,
    pub autojoin_space: Option<Id>,
    /// KLIPY app key for the GIF picker (`LAKLAK_KLIPY_KEY`).
    pub klipy_key: Option<String>,
    /// Where password reset e-mails go out (`LAKLAK_SMTP_URL`, see `mail.rs`), and from whom (`LAKLAK_SMTP_FROM`).
    pub smtp_url: Option<String>,
    pub smtp_from: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Config> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Ok(Config {
            bind: var("LAKLAK_BIND")
                .unwrap_or("0.0.0.0:8080".into())
                .parse()?,
            data: var("LAKLAK_DATA").unwrap_or("./laklak-data".into()).into(),
            name: var("LAKLAK_NAME").unwrap_or("Laklak".into()),
            registration: match var("LAKLAK_REGISTRATION").as_deref() {
                None | Some("invite") => Registration::Invite,
                Some("open") => Registration::Open,
                Some("closed") => Registration::Closed,
                Some(v) => {
                    anyhow::bail!("LAKLAK_REGISTRATION must be open, invite or closed, not {v:?}")
                }
            },
            max_upload: var("LAKLAK_MAX_UPLOAD_MB")
                .map(|v| v.parse::<u64>())
                .transpose()?
                .unwrap_or(100)
                * 1024
                * 1024,
            media_port: var("LAKLAK_MEDIA_PORT")
                .map(|v| v.parse())
                .transpose()?
                .unwrap_or(50000),
            // ponytail: ~half the browser default (2.5 Mbit/s at 720p); simulcast if big calls ever need more.
            camera_kbps: var("LAKLAK_CAMERA_KBPS")
                .map(|v| v.parse())
                .transpose()?
                .unwrap_or(1000),
            game_utc_offset_min: var("LAKLAK_GAME_UTC_OFFSET")
                .map(|v| v.parse())
                .transpose()?
                .unwrap_or(60),
            public_ip: var("LAKLAK_PUBLIC_IP").map(|v| v.parse()).transpose()?,
            web_dir: var("LAKLAK_WEB_DIR").map(PathBuf::from),
            nextcloud_url: var("LAKLAK_NEXTCLOUD_URL"),
            nextcloud_group: var("LAKLAK_NEXTCLOUD_GROUP"),
            nextcloud_folder: var("LAKLAK_NEXTCLOUD_FOLDER").unwrap_or("Laklak".into()),
            autojoin_space: var("LAKLAK_AUTOJOIN_SPACE")
                .map(|v| v.parse())
                .transpose()?,
            klipy_key: var("LAKLAK_KLIPY_KEY"),
            smtp_url: var("LAKLAK_SMTP_URL"),
            smtp_from: var("LAKLAK_SMTP_FROM"),
        })
    }
}

pub struct AppState {
    pub cfg: Config,
    pub db: SqlitePool,
    pub hub: gateway::Hub,
    pub voice: voice::Voice,
    pub nc: Option<nextcloud::Nc>,
    pub mailer: Option<mail::Mailer>,
    ids: Mutex<(i64, i64)>,
    login_fails: Mutex<HashMap<String, Vec<Instant>>>,
    /// Reset e-mails sent per account lately (a few per quarter hour).
    reset_sends: Mutex<HashMap<Id, Vec<Instant>>>,
    send_budget: Mutex<HashMap<Id, (f64, Instant)>>,
}

pub type St = Arc<AppState>;

impl AppState {
    /// Next time-ordered id: `(ms since EPOCH_MS) << 10 | seq`, strictly increasing.
    pub fn id(&self) -> Id {
        let mut g = self.ids.lock().unwrap();
        let ms = now_ms() - EPOCH_MS;
        if ms > g.0 {
            *g = (ms, 0);
        } else {
            g.1 += 1;
            if g.1 >= 1024 {
                // 1024 ids in one millisecond: borrow the next one.
                *g = (g.0 + 1, 0);
            }
        }
        (g.0 << 10) | g.1
    }

    /// Message rate limit: 5 per second, bursts of 10. Returns the wait in ms when over.
    pub fn take_send_budget(&self, user: Id) -> Result<(), u64> {
        let mut map = self.send_budget.lock().unwrap();
        let now = Instant::now();
        let (tokens, at) = map.entry(user).or_insert((10.0, now));
        *tokens = (*tokens + at.elapsed().as_secs_f64() * 5.0).min(10.0);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - *tokens) / 5.0 * 1000.0).ceil() as u64)
        }
    }
}

pub fn now_ms() -> Millis {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as Millis
}

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub struct ApiErr {
    pub status: StatusCode,
    pub msg: String,
    pub retry_after_ms: Option<u64>,
}

impl ApiErr {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> ApiErr {
        ApiErr {
            status,
            msg: msg.into(),
            retry_after_ms: None,
        }
    }
}

pub type R<T> = Result<T, ApiErr>;

pub fn bad(msg: impl Into<String>) -> ApiErr {
    ApiErr::new(StatusCode::BAD_REQUEST, msg)
}
pub fn forbidden(msg: impl Into<String>) -> ApiErr {
    ApiErr::new(StatusCode::FORBIDDEN, msg)
}
pub fn not_found(what: &str) -> ApiErr {
    ApiErr::new(StatusCode::NOT_FOUND, format!("{what} not found"))
}
pub fn conflict(msg: impl Into<String>) -> ApiErr {
    ApiErr::new(StatusCode::CONFLICT, msg)
}
pub fn internal(e: impl std::fmt::Display) -> ApiErr {
    tracing::error!("{e}");
    ApiErr::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

impl From<sqlx::Error> for ApiErr {
    fn from(e: sqlx::Error) -> Self {
        internal(format!("database: {e}"))
    }
}

impl From<std::io::Error> for ApiErr {
    fn from(e: std::io::Error) -> Self {
        internal(format!("io: {e}"))
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiError {
                error: self.msg,
                retry_after_ms: self.retry_after_ms,
            }),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------- app

/// Open the database, start the SFU and background jobs, and build the router.
pub async fn build(cfg: Config) -> anyhow::Result<(Router, St)> {
    std::fs::create_dir_all(cfg.data.join("files"))?;
    let db = db::open(&cfg.data.join("laklak.db")).await?;
    let (voice, sfu_events) = voice::Voice::start(&cfg)?;
    // Outbound HTTPS (Nextcloud) uses rustls with ring; installing twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let nc = match &cfg.nextcloud_url {
        Some(url) => Some(nextcloud::Nc::new(
            url,
            cfg.nextcloud_group.clone(),
            cfg.nextcloud_folder.clone(),
            cfg.autojoin_space,
        )?),
        None => None,
    };
    let mailer = mail::Mailer::new(cfg.smtp_url.as_deref(), cfg.smtp_from.as_deref())?;
    if let Some(m) = &mailer {
        match m.check().await {
            Ok(()) => tracing::info!("password reset e-mail is on"),
            Err(e) => tracing::warn!("LAKLAK_SMTP_URL: {e:#} (reset e-mails will fail until it works)"),
        }
    }
    let st: St = Arc::new(AppState {
        cfg: cfg.clone(),
        db,
        hub: gateway::Hub::default(),
        voice,
        nc,
        mailer,
        ids: Mutex::new((0, 0)),
        login_fails: Mutex::default(),
        reset_sends: Mutex::default(),
        send_budget: Mutex::default(),
    });
    db::upgrade_roles(&st.db, || st.id())
        .await
        .map_err(|e| anyhow::anyhow!("upgrading roles: {}", e.msg))?;
    tokio::spawn(voice::watch_sfu(st.clone(), sfu_events));
    tokio::spawn(messages::gc_uploads(st.clone()));
    tokio::spawn(spaces::archive_idle_threads(st.clone()));
    tokio::spawn(embeds::backfill(st.clone()));
    games::init(&st).await?;
    tokio::spawn(games::summaries(st.clone()));

    let mut app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/gateway", get(gateway::ws))
        .route("/api/info", get(auth::info))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/forgot", post(auth::forgot_password))
        .route("/api/auth/reset", post(auth::reset_with_code))
        .route("/api/me/email", put(auth::set_email))
        .route("/api/auth/nextcloud", post(nextcloud::login_start))
        .route("/api/auth/nextcloud/poll", post(nextcloud::login_poll))
        .route("/api/nextcloud/files", get(nextcloud::files))
        .route("/api/nextcloud/share", post(nextcloud::share))
        .route("/api/me", patch(auth::update_me).delete(auth::delete_me))
        .route("/api/me/sessions", get(auth::sessions))
        .route("/api/me/sessions/{id}", delete(auth::revoke_session))
        .route("/api/me/password", put(auth::change_password))
        .route("/api/users/{id}/password", post(auth::reset_password))
        .route("/api/me/settings", put(auth::put_settings))
        .route(
            "/api/uploads",
            post(messages::upload).layer(DefaultBodyLimit::max(cfg.max_upload as usize)),
        )
        .route("/files/{token}/{name}", get(messages::file))
        .route("/api/spaces", post(spaces::create_space))
        .route(
            "/api/spaces/{id}",
            patch(spaces::update_space).delete(spaces::delete_space),
        )
        .route(
            "/api/spaces/{id}/invites",
            get(spaces::list_invites).post(spaces::create_invite),
        )
        .route(
            "/api/invites/{code}",
            get(spaces::invite_preview)
                .post(spaces::join)
                .delete(spaces::revoke_invite),
        )
        .route(
            "/api/spaces/{id}/members/{user_id}",
            patch(spaces::update_member).delete(spaces::remove_member),
        )
        .route("/api/spaces/{id}/roles", post(spaces::create_role).put(spaces::reorder_roles))
        .route(
            "/api/spaces/{id}/roles/{role_id}",
            patch(spaces::update_role).delete(spaces::delete_role),
        )
        .route(
            "/api/spaces/{id}/members/{user_id}/roles/{role_id}",
            put(spaces::add_member_role).delete(spaces::remove_member_role),
        )
        .route(
            "/api/channels/{id}/permissions/{target_id}",
            put(spaces::set_overwrite).delete(spaces::delete_overwrite),
        )
        .route("/api/games/word", get(games::get))
        .route("/api/games/word/guess", post(games::guess))
        .route("/api/spaces/{id}/bans", get(spaces::list_bans))
        .route(
            "/api/spaces/{id}/bans/{user_id}",
            put(spaces::ban).delete(spaces::unban),
        )
        .route("/api/spaces/{id}/channels", post(spaces::create_channel))
        .route(
            "/api/channels/{id}",
            patch(spaces::update_channel).delete(spaces::delete_channel),
        )
        .route("/api/dms", post(spaces::create_dm))
        .route(
            "/api/channels/{id}/threads",
            get(spaces::list_threads).post(spaces::create_thread),
        )
        .route(
            "/api/channels/{id}/messages",
            get(messages::list).post(messages::send),
        )
        .route(
            "/api/channels/{id}/messages/{mid}",
            patch(messages::edit).delete(messages::delete),
        )
        .route(
            "/api/channels/{id}/messages/{mid}/reactions/{emoji}",
            put(messages::react).delete(messages::unreact),
        )
        .route("/api/channels/{id}/pins", get(messages::pins))
        .route(
            "/api/channels/{id}/pins/{mid}",
            put(messages::pin).delete(messages::unpin),
        )
        .route("/api/channels/{id}/ack", post(messages::ack))
        .route("/api/search", get(messages::search))
        .route("/api/channels/{id}/voice", post(voice::join))
        .route("/api/voice", patch(voice::update).delete(voice::leave))
        .route("/api/channels/{id}/ring", post(voice::ring));

    app = match &cfg.web_dir {
        // The app is a single page: unknown paths get index.html.
        Some(dir) => app.fallback_service(
            // `fallback` (not `not_found_service`) so app routes like /invite/abc answer 200.
            // `no-cache` = ask the server every time (a cheap 304 when nothing changed). Without it browsers cache
            // index.html by guesswork and keep running the old app after an update, while the desktop app, which
            // carries its own copy, already has the new one.
            SetResponseHeader::overriding(
                ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html"))),
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-cache"),
            ),
        ),
        None => app.fallback(|| async { not_found("route") }),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(AllowMethods::mirror_request())
        // `*` does not cover Authorization, so mirror what the browser asks for.
        .allow_headers(AllowHeaders::mirror_request())
        .max_age(std::time::Duration::from_secs(3600));
    let app = app
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(st.clone());
    Ok((app, st))
}

/// Run until Ctrl+C / SIGTERM.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let bind = cfg.bind;
    let (app, st) = build(cfg).await?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(
        "laklak {VERSION} listening on http://{bind}, call media on udp {}",
        st.voice.candidate
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    st.db.close().await;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    tracing::info!("shutting down");
}
