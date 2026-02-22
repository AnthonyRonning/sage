use anyhow::Result;

/// An attachment received from a messaging provider
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IncomingAttachment {
    pub file: String,
    pub content_type: String,
    pub size: Option<u64>,
}

/// A message received from a messaging provider
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique identifier of the sender (Signal UUID, Nostr pubkey, etc.)
    pub source: String,
    pub source_name: Option<String>,
    pub message: String,
    pub attachments: Vec<IncomingAttachment>,
    #[allow(dead_code)]
    pub timestamp: u64,
    /// Identity key for agent lookup and reply routing (Signal UUID or Marmot pubkey)
    pub reply_to: String,
}

/// Trait for sending messages via a messaging provider
pub trait Messenger: Send + Sync {
    fn send_message(&self, recipient: &str, message: &str) -> Result<()>;
    fn send_typing(&self, recipient: &str, stop: bool) -> Result<()>;

    /// Periodic health/refresh check (no-op by default)
    fn refresh(&self) -> Result<()> {
        Ok(())
    }
}
