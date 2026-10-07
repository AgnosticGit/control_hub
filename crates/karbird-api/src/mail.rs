use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct InviteEmail {
    pub recipient: String,
    pub organization: String,
    pub inviter: String,
    pub accept_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("email delivery is not configured")]
    Disabled,
    #[error("invalid mail configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("email provider rejected the request")]
    Provider,
}

pub trait Mailer: std::fmt::Debug + Send + Sync {
    fn send_invite<'a>(
        &'a self,
        message: InviteEmail,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;
}

#[derive(Debug, Default)]
pub struct DisabledMailer;

impl Mailer for DisabledMailer {
    fn send_invite<'a>(
        &'a self,
        _message: InviteEmail,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>> {
        Box::pin(async { Err(MailError::Disabled) })
    }
}

#[derive(Debug, Clone)]
pub struct ResendMailer {
    client: reqwest::Client,
    api_key: String,
    from: String,
}

impl ResendMailer {
    pub fn new(api_key: String, from: String) -> Result<Self, MailError> {
        if api_key.trim().is_empty() {
            return Err(MailError::InvalidConfiguration("KARBIRD_RESEND_API_KEY is empty"));
        }
        if !from.contains('@') || from.chars().any(|character| character == '\r' || character == '\n') {
            return Err(MailError::InvalidConfiguration("KARBIRD_MAIL_FROM must be an email address"));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| MailError::InvalidConfiguration("HTTP client"))?;
        Ok(Self { client, api_key, from })
    }
}

#[derive(Serialize)]
struct ResendRequest<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: String,
    text: String,
}

impl Mailer for ResendMailer {
    fn send_invite<'a>(
        &'a self,
        message: InviteEmail,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>> {
        Box::pin(async move {
            let body = ResendRequest {
                from: &self.from,
                to: [&message.recipient],
                subject: format!("Join {} on karbird", message.organization),
                text: format!(
                    "{} invited you to {} on karbird. Accept the invitation: {}",
                    message.inviter, message.organization, message.accept_url
                ),
            };
            let response = self
                .client
                .post("https://api.resend.com/emails")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|_| MailError::Provider)?;
            if response.status().is_success() { Ok(()) } else { Err(MailError::Provider) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_configuration_rejects_empty_or_header_injected_values() {
        assert!(ResendMailer::new(String::new(), "hello@karbird.com".into()).is_err());
        assert!(ResendMailer::new("key".into(), "hello@karbird.com\r\nBcc:x@y.test".into()).is_err());
        assert!(ResendMailer::new("key".into(), "Karbird <hello@karbird.com>".into()).is_ok());
    }

    #[tokio::test]
    async fn disabled_mailer_reports_a_visible_delivery_error() {
        let result = DisabledMailer
            .send_invite(InviteEmail {
                recipient: "person@example.test".into(),
                organization: "Example".into(),
                inviter: "Owner".into(),
                accept_url: "https://karbird.com/app/invite#secret".into(),
            })
            .await;
        assert!(matches!(result, Err(MailError::Disabled)));
    }
}
