//! Signal interface via signal-cli JSON-RPC
//!
//! Supports two modes:
//! 1. TCP mode: Connect to signal-cli daemon running in separate container (Docker)
//! 2. Subprocess mode: Start signal-cli as subprocess (native/dev)

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// A message received from Signal
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub source: String,
    pub source_name: Option<String>,
    pub message: String,
    #[allow(dead_code)]
    pub timestamp: u64,
}

/// Connection mode for signal-cli
#[allow(dead_code)]
enum ConnectionMode {
    /// TCP connection to signal-cli daemon
    Tcp {
        reader: BufReader<TcpStream>,
        writer: BufWriter<TcpStream>,
    },
    /// Subprocess running signal-cli
    Subprocess {
        process: Child,
        writer: BufWriter<std::process::ChildStdin>,
    },
}

/// Signal client using signal-cli JSON-RPC
pub struct SignalClient {
    mode: Mutex<ConnectionMode>,
    request_id: AtomicU64,
    account: String,
    /// TCP connection parameters for reconnection
    tcp_host: Option<String>,
    tcp_port: u16,
}

impl SignalClient {
    /// Create a new Signal client connecting to a TCP daemon
    pub fn connect_tcp(account: &str, host: &str, port: u16) -> Result<Self> {
        info!("Connecting to signal-cli daemon at {}:{}", host, port);

        let stream =
            TcpStream::connect((host, port)).context("Failed to connect to signal-cli daemon")?;

        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);

        info!("Connected to signal-cli daemon");

        Ok(Self {
            mode: Mutex::new(ConnectionMode::Tcp { reader, writer }),
            request_id: AtomicU64::new(1),
            account: account.to_string(),
            tcp_host: Some(host.to_string()),
            tcp_port: port,
        })
    }

    /// Reconnect TCP connection (for recovery from broken pipe)
    pub fn reconnect(&self) -> Result<()> {
        let host = self
            .tcp_host
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Cannot reconnect: not in TCP mode"))?;

        warn!(
            "Reconnecting to signal-cli daemon at {}:{}...",
            host, self.tcp_port
        );

        let stream = TcpStream::connect((host.as_str(), self.tcp_port))
            .context("Failed to reconnect to signal-cli daemon")?;

        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);

        let mut mode = self
            .mode
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *mode = ConnectionMode::Tcp { reader, writer };

        info!("Reconnected to signal-cli daemon successfully");
        Ok(())
    }

    /// Create a new Signal client spawning a subprocess
    pub fn spawn_subprocess(account: &str) -> Result<Self> {
        info!("Starting signal-cli for account: {}", account);

        let mut process = Command::new("signal-cli")
            .args(["-a", account, "jsonRpc", "--send-read-receipts"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn signal-cli. Is it installed and in PATH?")?;

        let stdin = process.stdin.take().context("Failed to get stdin")?;
        let writer = BufWriter::new(stdin);

        info!("signal-cli started successfully");

        Ok(Self {
            mode: Mutex::new(ConnectionMode::Subprocess { process, writer }),
            request_id: AtomicU64::new(1),
            account: account.to_string(),
            tcp_host: None,
            tcp_port: 0,
        })
    }

    /// Subscribe to receive messages (required for TCP mode)
    #[allow(dead_code)]
    pub fn subscribe_receive(&self) -> Result<()> {
        info!("Subscribing to messages...");
        self.send_request("subscribeReceive", json!({}))?;
        Ok(())
    }

    /// Send a JSON-RPC request (fire and forget for now)
    fn send_request(&self, method: &str, mut params: Value) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        // Add account parameter for TCP mode
        let mut mode = self
            .mode
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        if matches!(*mode, ConnectionMode::Tcp { .. }) {
            if let Value::Object(ref mut map) = params {
                map.insert("account".to_string(), json!(self.account));
            }
        }

        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id
        });

        let request_str = serde_json::to_string(&request)? + "\n";
        debug!("Sending request: {}", request_str.trim());

        match &mut *mode {
            ConnectionMode::Tcp { writer, .. } => {
                writer.write_all(request_str.as_bytes())?;
                writer.flush()?;
            }
            ConnectionMode::Subprocess { writer, .. } => {
                writer.write_all(request_str.as_bytes())?;
                writer.flush()?;
            }
        }

        Ok(json!({"status": "sent", "id": id}))
    }

    /// Send a message to a recipient with retry on connection failure
    pub fn send_message(&self, recipient: &str, message: &str) -> Result<()> {
        // Find valid UTF-8 boundary for preview
        let preview_end = {
            let max_len = 50.min(message.len());
            let mut end = max_len;
            while end > 0 && !message.is_char_boundary(end) {
                end -= 1;
            }
            end
        };

        // Retry logic: try up to 3 times with reconnection on failure
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 1..=max_retries {
            let result = self.send_request(
                "send",
                json!({
                    "recipient": [recipient],
                    "message": message
                }),
            );

            match result {
                Ok(res) => {
                    let request_id = res.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    info!(
                        "Sent message (req #{}) to {}: {}...",
                        request_id,
                        recipient,
                        &message[..preview_end]
                    );
                    return Ok(());
                }
                Err(e) => {
                    let error_str = e.to_string();
                    warn!(
                        "Send attempt {}/{} failed: {}",
                        attempt, max_retries, error_str
                    );
                    last_error = Some(e);

                    // If it's a broken pipe or connection error, try to reconnect
                    if error_str.contains("Broken pipe")
                        || error_str.contains("Connection reset")
                        || error_str.contains("os error 32")
                        || error_str.contains("os error 104")
                    {
                        if attempt < max_retries {
                            if let Err(reconnect_err) = self.reconnect() {
                                warn!("Reconnection failed: {}", reconnect_err);
                                // Small delay before retry
                                std::thread::sleep(std::time::Duration::from_millis(500));
                            }
                        }
                    } else {
                        // Non-connection error, don't retry
                        break;
                    }
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| anyhow::anyhow!("Send failed after {} retries", max_retries)))
    }

    /// Send typing indicator to a recipient
    pub fn send_typing(&self, recipient: &str, stop: bool) -> Result<()> {
        debug!("Sending typing indicator (stop={}) to {}", stop, recipient);

        self.send_request(
            "sendTyping",
            json!({
                "recipient": [recipient],
                "stop": stop
            }),
        )?;

        Ok(())
    }

    /// Send read receipt for a message
    #[allow(dead_code)]
    pub fn send_read_receipt(&self, recipient: &str, timestamp: u64) -> Result<()> {
        debug!(
            "Sending read receipt to {} for timestamp {}",
            recipient, timestamp
        );

        self.send_request(
            "sendReceipt",
            json!({
                "recipient": [recipient],
                "targetTimestamp": [timestamp],
                "type": "read"
            }),
        )?;

        Ok(())
    }

    /// Refresh account/prekeys to prevent silent send failures
    /// Call this periodically (e.g., every 4-8 hours) as a health check
    pub fn refresh_account(&self) -> Result<()> {
        info!("Refreshing Signal account (prekey health check)...");

        self.send_request("updateAccount", json!({}))?;

        info!("Signal account refreshed successfully");
        Ok(())
    }

    /// Take the reader for the receive loop (consumes self partially)
    /// Returns a reader that can be used in run_receive_loop
    pub fn take_reader(&self) -> Result<SignalReader> {
        let mut mode = self
            .mode
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        match &mut *mode {
            ConnectionMode::Tcp { .. } => {
                // For TCP, we need to clone the underlying stream
                // This is a limitation - we'll need a different approach
                Err(anyhow::anyhow!(
                    "TCP reader extraction not yet supported - use run_receive_loop_tcp"
                ))
            }
            ConnectionMode::Subprocess { process, .. } => {
                let stdout = process.stdout.take().context("stdout already taken")?;
                Ok(SignalReader::Subprocess(BufReader::new(stdout)))
            }
        }
    }

    /// Check if the subprocess is still running (only for subprocess mode)
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        let mut mode = match self.mode.lock() {
            Ok(m) => m,
            Err(_) => return false,
        };

        match &mut *mode {
            ConnectionMode::Tcp { .. } => true, // Assume TCP is always "running"
            ConnectionMode::Subprocess { process, .. } => match process.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    warn!("signal-cli exited with status: {}", status);
                    false
                }
                Err(e) => {
                    error!("Error checking signal-cli status: {}", e);
                    false
                }
            },
        }
    }
}

impl Drop for SignalClient {
    fn drop(&mut self) {
        if let Ok(mut mode) = self.mode.lock() {
            match &mut *mode {
                ConnectionMode::Tcp { .. } => {
                    info!("Disconnecting from signal-cli daemon");
                }
                ConnectionMode::Subprocess { process, .. } => {
                    info!("Shutting down signal-cli subprocess");
                    let _ = process.kill();
                }
            }
        }
    }
}

/// Reader for incoming messages
pub enum SignalReader {
    Subprocess(BufReader<std::process::ChildStdout>),
}

/// Parse incoming JSON-RPC notifications for messages
pub fn parse_incoming_message(line: &str) -> Option<IncomingMessage> {
    let value: Value = serde_json::from_str(line).ok()?;

    // Check if this is a receive notification
    if value.get("method")?.as_str()? != "receive" {
        return None;
    }

    let params = value.get("params")?;
    let envelope = params.get("envelope")?;

    // Get the message content
    let data_message = envelope.get("dataMessage")?;
    let message = data_message.get("message")?.as_str()?;

    // Skip empty messages
    if message.is_empty() {
        return None;
    }

    // Try sourceUuid first (preferred), fall back to sourceNumber
    let source = envelope
        .get("sourceUuid")
        .and_then(|v| v.as_str())
        .or_else(|| envelope.get("sourceNumber").and_then(|v| v.as_str()))
        .or_else(|| envelope.get("source").and_then(|v| v.as_str()))?
        .to_string();

    let source_name = envelope
        .get("sourceName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let timestamp = data_message.get("timestamp")?.as_u64()?;

    Some(IncomingMessage {
        source,
        source_name,
        message: message.to_string(),
        timestamp,
    })
}

/// Run the message receive loop for subprocess mode
pub async fn run_receive_loop(
    reader: SignalReader,
    tx: mpsc::Sender<IncomingMessage>,
) -> Result<()> {
    match reader {
        SignalReader::Subprocess(reader) => {
            tokio::task::spawn_blocking(move || {
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            debug!("Received from signal-cli: {}", line);

                            if let Some(msg) = parse_incoming_message(&line) {
                                // Find valid UTF-8 boundary for preview
                                let preview_end = {
                                    let max_len = 100.min(msg.message.len());
                                    let mut end = max_len;
                                    while end > 0 && !msg.message.is_char_boundary(end) {
                                        end -= 1;
                                    }
                                    end
                                };
                                info!(
                                    "📨 Message from {}: {}",
                                    msg.source_name.as_deref().unwrap_or(&msg.source),
                                    &msg.message[..preview_end]
                                );

                                if tx.blocking_send(msg).is_err() {
                                    error!("Failed to send message to channel");
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error reading from signal-cli: {}", e);
                            break;
                        }
                    }
                }
                warn!("Signal receive loop ended");
            })
            .await?;
        }
    }

    Ok(())
}

/// Run the message receive loop for TCP mode
/// This needs the TcpStream directly since we can't easily share the BufReader
pub async fn run_receive_loop_tcp(
    host: &str,
    port: u16,
    account: &str,
    tx: mpsc::Sender<IncomingMessage>,
) -> Result<()> {
    let host = host.to_string();
    let account = account.to_string();

    tokio::task::spawn_blocking(move || {
        // Create a separate connection for receiving
        let stream = TcpStream::connect((&host[..], port))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = BufWriter::new(stream);

        // Subscribe to receive messages
        let subscribe_request = json!({
            "jsonrpc": "2.0",
            "method": "subscribeReceive",
            "params": {"account": account},
            "id": 1
        });
        let request_str = serde_json::to_string(&subscribe_request)? + "\n";
        writer.write_all(request_str.as_bytes())?;
        writer.flush()?;

        info!("Subscribed to messages on TCP connection");

        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    warn!("signal-cli daemon closed connection");
                    break;
                }
                Ok(_) => {
                    debug!("Received from signal-cli: {}", line.trim());

                    if let Some(msg) = parse_incoming_message(&line) {
                        // Find valid UTF-8 boundary for preview
                        let preview_end = {
                            let max_len = 100.min(msg.message.len());
                            let mut end = max_len;
                            while end > 0 && !msg.message.is_char_boundary(end) {
                                end -= 1;
                            }
                            end
                        };
                        info!(
                            "📨 Message from {}: {}",
                            msg.source_name.as_deref().unwrap_or(&msg.source),
                            &msg.message[..preview_end]
                        );

                        if tx.blocking_send(msg).is_err() {
                            error!("Failed to send message to channel");
                            break;
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading from signal-cli: {}", e);
                    break;
                }
            }
        }
        warn!("Signal TCP receive loop ended");
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}
