use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::messenger::{IncomingMessage, Messenger};

const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// Decode a bech32-encoded string (npub1...) into its raw bytes.
fn bech32_decode_payload(s: &str) -> Option<Vec<u8>> {
    let pos = s.rfind('1')?;
    let data_part = &s[pos + 1..];
    if data_part.len() < 6 {
        return None;
    }
    let values: Vec<u8> = data_part
        .chars()
        .map(|c| BECH32_CHARSET.find(c).map(|i| i as u8))
        .collect::<Option<Vec<_>>>()?;
    let data_values = &values[..values.len() - 6];
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut result = Vec::new();
    for &v in data_values {
        acc = (acc << 5) | (v as u32);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            result.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(result)
}

/// Convert an npub (bech32) or hex pubkey string to hex.
/// Accepts both "npub1..." and raw 64-char hex.
pub fn normalize_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let bytes =
            bech32_decode_payload(trimmed).ok_or_else(|| anyhow!("invalid npub: {}", trimmed))?;
        if bytes.len() != 32 {
            return Err(anyhow!(
                "npub decoded to {} bytes, expected 32",
                bytes.len()
            ));
        }
        Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
    } else if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_lowercase())
    } else {
        Err(anyhow!(
            "invalid pubkey (expected npub1... or 64-char hex): {}",
            trimmed
        ))
    }
}

#[derive(Debug, Clone)]
pub struct MarmotConfig {
    pub binary_path: String,
    pub relays: Vec<String>,
    pub state_dir: String,
    pub allowed_pubkeys: Vec<String>,
    pub auto_accept_welcomes: bool,
}

pub struct MarmotClient {
    writer: Arc<Mutex<BufWriter<std::process::ChildStdin>>>,
    request_id: AtomicU64,
    /// Maps sender pubkey -> latest nostr_group_id for routing replies.
    /// Currently treats each pubkey as a single identity (like Signal UUID),
    /// collapsing all groups from the same sender into one agent context.
    /// TODO: When multi-agent/subagent support lands, this could be extended
    /// to route per-group (each group ID = separate agent thread) while still
    /// sharing a parent identity for cross-thread memory.
    group_routes: Arc<Mutex<HashMap<String, String>>>,
    child: Mutex<Child>,
}

impl Drop for MarmotClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl MarmotClient {
    fn send_cmd(&self, cmd: serde_json::Value) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| anyhow!("Lock error: {}", e))?;
        let cmd_str = serde_json::to_string(&cmd)? + "\n";
        writer.write_all(cmd_str.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    fn next_request_id(&self) -> String {
        self.request_id.fetch_add(1, Ordering::SeqCst).to_string()
    }
}

impl MarmotClient {
    fn resolve_group(&self, pubkey: &str) -> Result<String> {
        let routes = self
            .group_routes
            .lock()
            .map_err(|e| anyhow!("Lock error: {}", e))?;
        routes
            .get(pubkey)
            .cloned()
            .ok_or_else(|| anyhow!("No group route for pubkey {}", pubkey))
    }
}

impl Messenger for MarmotClient {
    fn send_message(&self, recipient: &str, message: &str) -> Result<()> {
        let group_id = self.resolve_group(recipient)?;
        let id = self.next_request_id();
        let preview_end = {
            let max_len = 50.min(message.len());
            let mut end = max_len;
            while end > 0 && !message.is_char_boundary(end) {
                end -= 1;
            }
            end
        };
        info!(
            "Sending marmot message (req #{}) to {} via group {}: {}...",
            id,
            recipient,
            group_id,
            &message[..preview_end]
        );
        self.send_cmd(json!({
            "cmd": "send_message",
            "request_id": id,
            "nostr_group_id": group_id,
            "content": message
        }))
    }

    fn send_typing(&self, recipient: &str, stop: bool) -> Result<()> {
        if stop {
            return Ok(());
        }
        let group_id = match self.resolve_group(recipient) {
            Ok(gid) => gid,
            Err(_) => return Ok(()),
        };
        let id = self.next_request_id();
        self.send_cmd(json!({
            "cmd": "send_typing",
            "request_id": id,
            "nostr_group_id": group_id
        }))
    }
}

/// Spawn marmotd daemon and return the client, stdout reader, and child process handle.
pub fn spawn_marmot(config: &MarmotConfig) -> Result<(MarmotClient, std::process::ChildStdout)> {
    let mut cmd = Command::new(&config.binary_path);
    cmd.arg("daemon");

    for relay in &config.relays {
        cmd.arg("--relay").arg(relay);
    }

    cmd.arg("--state-dir").arg(&config.state_dir);

    let is_wildcard = config.allowed_pubkeys.iter().any(|p| p == "*");
    if !is_wildcard {
        for pk in &config.allowed_pubkeys {
            let hex_pk = normalize_pubkey(pk)
                .with_context(|| format!("invalid MARMOT_ALLOWED_PUBKEYS entry: {}", pk))?;
            cmd.arg("--allow-pubkey").arg(&hex_pk);
        }
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!(
        "Spawning marmotd: {} daemon --relay {} --state-dir {}",
        config.binary_path,
        config.relays.join(","),
        config.state_dir
    );

    let mut child = cmd.spawn().context("Failed to spawn marmotd")?;

    let stdin = child.stdin.take().context("Failed to get marmotd stdin")?;
    let stdout = child
        .stdout
        .take()
        .context("Failed to get marmotd stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("Failed to get marmotd stderr")?;

    // Forward stderr to tracing
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            info!(target: "marmotd", "{}", line);
        }
    });

    let writer = Arc::new(Mutex::new(BufWriter::new(stdin)));

    let group_routes = Arc::new(Mutex::new(HashMap::new()));
    let client = MarmotClient {
        writer: writer.clone(),
        request_id: AtomicU64::new(1),
        group_routes,
        child: Mutex::new(child),
    };

    Ok((client, stdout))
}

/// Run the marmot receive loop: waits for daemon ready, publishes keypackage,
/// then listens for incoming messages and auto-accepts welcomes.
pub async fn run_marmot_receive_loop(
    stdout: std::process::ChildStdout,
    writer: Arc<Mutex<BufWriter<std::process::ChildStdin>>>,
    tx: mpsc::Sender<IncomingMessage>,
    config: MarmotConfig,
    group_routes: Arc<Mutex<HashMap<String, String>>>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        let send_cmd = |cmd: serde_json::Value| -> Result<()> {
            let mut w = writer.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let s = serde_json::to_string(&cmd)? + "\n";
            w.write_all(s.as_bytes())?;
            w.flush()?;
            Ok(())
        };

        // Phase 1: Wait for ready
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Err(anyhow!("marmotd closed stdout before ready"));
            }
            let event: serde_json::Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => {
                    debug!("marmotd non-json output (startup): {}", line.trim());
                    continue;
                }
            };

            if event.get("type").and_then(|t| t.as_str()) == Some("ready") {
                let pubkey = event
                    .get("pubkey")
                    .and_then(|p| p.as_str())
                    .unwrap_or("unknown");
                let npub = event
                    .get("npub")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                info!("marmotd ready: pubkey={} npub={}", pubkey, npub);
                break;
            }
        }

        // Phase 2: Publish keypackage
        let relays: Vec<&str> = config.relays.iter().map(|s| s.as_str()).collect();
        send_cmd(json!({
            "cmd": "publish_keypackage",
            "request_id": "init_kp",
            "relays": relays
        }))?;

        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Err(anyhow!("marmotd closed stdout during keypackage publish"));
            }
            let event: serde_json::Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => {
                    debug!("marmotd non-json output (keypackage): {}", line.trim());
                    continue;
                }
            };

            let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if event_type == "ok"
                && event.get("request_id").and_then(|id| id.as_str()) == Some("init_kp")
            {
                info!("marmotd keypackage published");
                break;
            }
            if event_type == "error"
                && event.get("request_id").and_then(|id| id.as_str()) == Some("init_kp")
            {
                let msg = event
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                warn!("marmotd keypackage publish failed: {}", msg);
                break;
            }
        }

        info!("Marmot receive loop started, listening for messages...");

        // Phase 3: Main receive loop
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    warn!("marmotd closed stdout");
                    break;
                }
                Ok(_) => {
                    let event: serde_json::Value = match serde_json::from_str(line.trim()) {
                        Ok(v) => v,
                        Err(e) => {
                            debug!("marmotd non-json output: {} ({})", line.trim(), e);
                            continue;
                        }
                    };
                    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    match event_type {
                        "welcome_received" => {
                            let wrapper_id = event
                                .get("wrapper_event_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            let from = event
                                .get("from_pubkey")
                                .and_then(|x| x.as_str())
                                .unwrap_or("unknown");
                            let group_name = event
                                .get("group_name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            info!(
                                "Marmot welcome received from {} (group: {}, wrapper: {})",
                                from, group_name, wrapper_id
                            );

                            if config.auto_accept_welcomes && !wrapper_id.is_empty() {
                                let req_id = format!("auto_{}", wrapper_id);
                                if let Err(e) = send_cmd(json!({
                                    "cmd": "accept_welcome",
                                    "request_id": req_id,
                                    "wrapper_event_id": wrapper_id
                                })) {
                                    warn!("Failed to auto-accept welcome: {}", e);
                                }
                            }
                        }
                        "group_joined" => {
                            let group_id = event
                                .get("nostr_group_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("unknown");
                            info!("Marmot joined group: {}", group_id);
                        }
                        "message_received" => {
                            let from_pubkey = event
                                .get("from_pubkey")
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            let content =
                                event.get("content").and_then(|x| x.as_str()).unwrap_or("");
                            let group_id = event
                                .get("nostr_group_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            let created_at = event
                                .get("created_at")
                                .and_then(|x| x.as_u64())
                                .unwrap_or(0);

                            if content.is_empty() {
                                continue;
                            }

                            let preview_end = {
                                let max_len = 100.min(content.len());
                                let mut end = max_len;
                                while end > 0 && !content.is_char_boundary(end) {
                                    end -= 1;
                                }
                                end
                            };
                            info!(
                                "Marmot message from {} in group {}: {}",
                                from_pubkey,
                                group_id,
                                &content[..preview_end]
                            );

                            // Track pubkey -> latest group for reply routing.
                            // This means the most recent group a user messages from
                            // becomes the reply target. When we add multi-agent support,
                            // each group could maintain its own agent thread instead.
                            if !from_pubkey.is_empty() && !group_id.is_empty() {
                                if let Ok(mut routes) = group_routes.lock() {
                                    routes.insert(from_pubkey.to_string(), group_id.to_string());
                                }
                            }

                            let msg = IncomingMessage {
                                source: from_pubkey.to_string(),
                                source_name: None,
                                message: content.to_string(),
                                attachments: vec![],
                                timestamp: created_at,
                                reply_to: from_pubkey.to_string(),
                                reply_context: Some(group_id.to_string()),
                            };

                            if tx.blocking_send(msg).is_err() {
                                error!("Failed to send marmot message to channel");
                                break;
                            }
                        }
                        "ok" | "keypackage_published" => {
                            debug!("marmotd: {}", line.trim());
                        }
                        "error" => {
                            let msg = event
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown");
                            warn!("marmotd error: {}", msg);
                        }
                        _ => {
                            debug!("marmotd event: {}", line.trim());
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading from marmotd: {}", e);
                    break;
                }
            }
        }

        warn!("Marmot receive loop ended");
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}

/// Get the shared writer handle from a MarmotClient (for the receive loop).
pub fn writer_handle(client: &MarmotClient) -> Arc<Mutex<BufWriter<std::process::ChildStdin>>> {
    client.writer.clone()
}

/// Get the shared group routes handle from a MarmotClient (for the receive loop).
pub fn group_routes_handle(client: &MarmotClient) -> Arc<Mutex<HashMap<String, String>>> {
    client.group_routes.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_npub() {
        let hex =
            normalize_pubkey("npub1gx8my906z8urmgzpcynjlj43ehwc5jket0mc70pkvzkg6k636hmqnwunq7")
                .unwrap();
        assert_eq!(
            hex,
            "418fb215fa11f83da041c1272fcab1cddd8a4ad95bf78f3c3660ac8d5b51d5f6"
        );
    }

    #[test]
    fn test_normalize_hex_passthrough() {
        let hex =
            normalize_pubkey("418fb215fa11f83da041c1272fcab1cddd8a4ad95bf78f3c3660ac8d5b51d5f6")
                .unwrap();
        assert_eq!(
            hex,
            "418fb215fa11f83da041c1272fcab1cddd8a4ad95bf78f3c3660ac8d5b51d5f6"
        );
    }

    #[test]
    fn test_normalize_invalid() {
        assert!(normalize_pubkey("not_a_valid_key").is_err());
        assert!(normalize_pubkey("npub1invalid").is_err());
    }
}
