//! Outgoing e-mail: password reset codes, through the server's own mail account.
//!
//! `NATTER_SMTP_URL` says where: `smtps://user:password@mail.example.com:465` (TLS from the start) or
//! `smtp://user:password@mail.example.com:587?tls=required` (STARTTLS); write an `@` in the user name as `%40`.
//! `NATTER_SMTP_FROM` is the sender, like `Natter <chat@example.com>`. `file:///some/folder` writes each mail there
//! as an `.eml` file instead of sending it (for trying things out). Without these the apps offer no e-mail reset;
//! a server admin can still give someone a temporary password.

use anyhow::Context;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::{AsyncFileTransport, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub struct Mailer {
    from: Mailbox,
    via: Via,
}

enum Via {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    File(AsyncFileTransport<Tokio1Executor>),
}

impl Mailer {
    /// From the config; `None` when no SMTP is set up. Fails on a URL or sender that can't work.
    pub fn new(url: Option<&str>, from: Option<&str>) -> anyhow::Result<Option<Mailer>> {
        let Some(url) = url else { return Ok(None) };
        let from: Mailbox = from
            .context("NATTER_SMTP_URL needs NATTER_SMTP_FROM, like \"Natter <chat@example.com>\"")?
            .parse()
            .context("NATTER_SMTP_FROM isn't an e-mail address, like \"Natter <chat@example.com>\"")?;
        let via = match url.strip_prefix("file://") {
            Some(dir) => {
                std::fs::create_dir_all(dir).with_context(|| format!("NATTER_SMTP_URL: can't create {dir}"))?;
                Via::File(AsyncFileTransport::new(dir))
            }
            None => Via::Smtp(
                AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                    .context("NATTER_SMTP_URL isn't an SMTP URL like smtps://user:password@mail.example.com:465")?
                    .build(),
            ),
        };
        Ok(Some(Mailer { from, via }))
    }

    /// Whether the SMTP server answers (logged at startup, so a wrong host shows up right away).
    pub async fn check(&self) -> anyhow::Result<()> {
        if let Via::Smtp(t) = &self.via {
            anyhow::ensure!(t.test_connection().await?, "the SMTP server didn't answer");
        }
        Ok(())
    }

    pub async fn send(&self, to: &str, subject: &str, body: String) -> anyhow::Result<()> {
        let msg = Message::builder()
            .from(self.from.clone())
            .to(to.parse()?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;
        match &self.via {
            Via::Smtp(t) => drop(t.send(msg).await?),
            Via::File(t) => drop(t.send(msg).await?),
        }
        Ok(())
    }
}
