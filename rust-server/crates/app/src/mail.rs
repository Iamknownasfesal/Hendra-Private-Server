//! Sending what a verification or reset link needs to reach.
//!
//! # Why this is a trait
//!
//! Delivery is the one part of the flow that leaves the process, and it is the part that differs
//! between a laptop, a test and a deployment. Everything else, the tokens, the expiry, the single
//! use, is the same everywhere and is tested without sending anything.
//!
//! A server with nothing configured logs what it would have sent. That is honest for development
//! and wrong for production, which is why it says so at startup.

use std::sync::{Arc, Mutex};

/// Somewhere to send a message.
pub trait Mail: Send + Sync {
    /// Sends one message, or reports that it could not.
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;

    /// Whether anything actually leaves the process.
    fn delivers(&self) -> bool {
        true
    }
}

/// Writes what it would have sent to the log.
///
/// For development, where a link in a terminal is more useful than one in an inbox nobody is
/// watching, and for a deployment that has not been configured yet, where it is at least visible.
pub struct Logged;

impl Mail for Logged {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        tracing::warn!(%to, %subject, %body, "no mail sender configured; logging instead");
        Ok(())
    }

    fn delivers(&self) -> bool {
        false
    }
}

/// Keeps what it was given, so a test can read it.
#[derive(Default)]
pub struct Captured {
    sent: Mutex<Vec<(String, String, String)>>,
}

impl Captured {
    /// Everything sent so far, as `(to, subject, body)`.
    pub fn sent(&self) -> Vec<(String, String, String)> {
        self.sent
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

impl Mail for Captured {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push((to.to_string(), subject.to_string(), body.to_string()));
        Ok(())
    }
}

/// What the app server holds.
pub type Sender = Arc<dyn Mail>;
