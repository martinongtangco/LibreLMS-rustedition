//! Mail-sending interface + development outbox (spec FR-010).
//!
//! Real delivery is out of scope for spec 001; the outbox is visible via a dev
//! page in later slices.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

/// An outbound mail (dev outbox entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Mail failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    Other(String),
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailError::Other(m) => write!(f, "mail: {m}"),
        }
    }
}

impl std::error::Error for MailError {}

/// The sending interface; later slices may add real senders behind it.
#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError>;
}

/// Development outbox: messages accumulate in-process and are drainable.
#[derive(Default, Clone)]
pub struct DevOutbox {
    queue: std::sync::Arc<Mutex<VecDeque<OutboundMail>>>,
}

impl DevOutbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain and return all queued mails (dev page, tests).
    pub fn drain(&self) -> Vec<OutboundMail> {
        self.queue.lock().expect("poisoned").drain(..).collect()
    }
}

#[async_trait]
impl Mailer for DevOutbox {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError> {
        self.queue
            .lock()
            .expect("poisoned")
            .push_back(OutboundMail {
                to: to.to_string(),
                subject: subject.to_string(),
                body: body.to_string(),
            });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outbox_collects_and_drains() {
        let m = DevOutbox::new();
        m.send("a@b.c", "hi", "body").await.unwrap();
        let msgs = m.drain();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].subject, "hi");
        assert!(m.drain().is_empty(), "drained");
    }
}
