//! Transactional e-mail delivery via Brevo.
//!
//! When `EMAIL_ENABLED` is false or no API key is configured the rendered
//! message is logged at INFO instead of being sent, so local development stays
//! fully functional without an SMTP provider.

#![allow(dead_code)]

use std::time::Duration;

use serde::Serialize;

use crate::config::Config;

const BREVO_ENDPOINT: &str = "https://api.brevo.com/v3/smtp/email";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A rendered transactional e-mail ready to be delivered.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
}

#[derive(Serialize)]
struct BrevoSender<'a> {
    email: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct BrevoRecipient<'a> {
    email: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct BrevoPayload<'a> {
    sender: BrevoSender<'a>,
    to: Vec<BrevoRecipient<'a>>,
    subject: &'a str,
    #[serde(rename = "htmlContent")]
    html_content: &'a str,
    #[serde(rename = "textContent")]
    text_content: &'a str,
}

/// Renders the verification e-mail.
pub fn verification_message(
    config: &Config,
    to: &str,
    username: &str,
    token: &str,
) -> EmailMessage {
    let link = format!("{}/verify-email?token={}", config.base_url, token);
    EmailMessage {
        to: to.to_string(),
        subject: format!("Verify your {} account", config.title),
        html: layout(
            config,
            "Confirm your e-mail address",
            &format!(
                "Hi {},<br><br>confirm this address to activate your container registry account.",
                escape(username)
            ),
            "Verify e-mail",
            &link,
        ),
        text: format!(
            "Hi {username},\n\nConfirm your e-mail address to activate your account:\n{link}\n"
        ),
    }
}

/// Renders the password-reset e-mail.
pub fn password_reset_message(
    config: &Config,
    to: &str,
    username: &str,
    token: &str,
) -> EmailMessage {
    let link = format!("{}/reset-password?token={}", config.base_url, token);
    EmailMessage {
        to: to.to_string(),
        subject: format!("Reset your {} password", config.title),
        html: layout(
            config,
            "Reset your password",
            &format!(
                "Hi {},<br><br>we received a request to reset your password. This link expires shortly.",
                escape(username)
            ),
            "Reset password",
            &link,
        ),
        text: format!(
            "Hi {username},\n\nReset your password with this link:\n{link}\n\nIf you did not request this, ignore this email.\n"
        ),
    }
}

/// Renders and delivers the verification e-mail, logging failures.
pub async fn send_verification_email(config: &Config, to: &str, username: &str, token: &str) {
    let message = verification_message(config, to, username, token);
    if let Err(err) = send(config, &message).await {
        tracing::warn!(error = %err, to = %message.to, "failed to send verification e-mail");
    }
}

/// Renders and delivers the password-reset e-mail, logging failures.
pub async fn send_password_reset_email(config: &Config, to: &str, username: &str, token: &str) {
    let message = password_reset_message(config, to, username, token);
    if let Err(err) = send(config, &message).await {
        tracing::warn!(error = %err, to = %message.to, "failed to send password-reset e-mail");
    }
}

/// Sends a rendered message through Brevo, or logs it when e-mail is disabled.
pub async fn send(config: &Config, message: &EmailMessage) -> anyhow::Result<()> {
    let api_key = config
        .brevo_api_key
        .as_deref()
        .filter(|key| !key.is_empty());

    let (Some(api_key), true) = (api_key, config.email_enabled) else {
        tracing::info!(
            to = %message.to,
            subject = %message.subject,
            body = %message.text,
            "e-mail delivery disabled; rendered message follows"
        );
        return Ok(());
    };

    let payload = BrevoPayload {
        sender: BrevoSender {
            email: &config.brevo_sender_email,
            name: &config.brevo_sender_name,
        },
        to: vec![BrevoRecipient {
            email: &message.to,
            name: &message.to,
        }],
        subject: &message.subject,
        html_content: &message.html,
        text_content: &message.text,
    };

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|err| anyhow::anyhow!("failed to build HTTP client: {err}"))?;

    let response = client
        .post(BREVO_ENDPOINT)
        .header("api-key", api_key)
        .header("accept", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("Brevo request failed: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Brevo returned {status}: {detail}"));
    }
    Ok(())
}

fn layout(config: &Config, heading: &str, body_html: &str, button: &str, link: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"color-scheme\" content=\"light dark\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<style>\
body{{margin:0;padding:24px;background:#f6f8fa;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;color:#1f2328;}}\
.card{{max-width:560px;margin:0 auto;background:#ffffff;border:1px solid #d0d7de;border-radius:6px;padding:32px;}}\
h1{{font-size:20px;margin:0 0 16px;}}\
p{{font-size:14px;line-height:1.5;}}\
a.button{{display:inline-block;margin:16px 0;padding:10px 16px;background:#0969da;color:#ffffff;border-radius:6px;text-decoration:none;font-weight:600;}}\
.muted{{color:#636c76;font-size:12px;word-break:break-all;}}\
@media (prefers-color-scheme: dark){{\
body{{background:#0d1117;color:#e6edf3;}}\
.card{{background:#161b22;border-color:#30363d;}}\
a.button{{background:#2f81f7;}}\
.muted{{color:#848d97;}}\
}}</style></head><body><div class=\"card\">\
<h1>{heading}</h1><p>{body_html}</p>\
<p><a class=\"button\" href=\"{link}\">{button}</a></p>\
<p class=\"muted\">Or open this link: {link}</p>\
<p class=\"muted\">{title}</p>\
</div></body></html>",
        heading = escape(heading),
        body_html = body_html,
        button = escape(button),
        link = escape(link),
        title = escape(&config.title),
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::test_config;

    #[test]
    fn verification_message_contains_link_and_primer_palette() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = test_config(dir.path());
        let message = verification_message(&config, "a@example.com", "alice", "abc123");
        assert!(message.html.contains("#0969da"));
        assert!(message.html.contains("#1f2328"));
        assert!(message.html.contains("#d0d7de"));
        assert!(message.html.contains("prefers-color-scheme: dark"));
        assert!(message.html.contains("/verify-email?token=abc123"));
        assert!(message.text.contains("abc123"));
    }

    #[test]
    fn html_escapes_user_supplied_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = test_config(dir.path());
        let message = password_reset_message(&config, "a@example.com", "<script>", "t");
        assert!(!message.html.contains("<script>"));
        assert!(message.html.contains("&lt;script&gt;"));
    }

    #[tokio::test]
    async fn disabled_transport_is_a_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = test_config(dir.path());
        let message = verification_message(&config, "a@example.com", "alice", "t");
        assert!(send(&config, &message).await.is_ok());
    }
}
