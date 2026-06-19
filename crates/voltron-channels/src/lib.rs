//! voltron-channels — ChannelAdapter implementation for CLI (stdin/stdout).

use async_trait::async_trait;
use futures::stream::Stream;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use voltron_core::{ChannelAdapter, Message, VoltronError};

/// A `ChannelAdapter` backed by stdin (read) and stdout (write).
///
/// Messages received via stdin are expected as JSON lines. Outgoing messages
/// are written as JSON lines to stdout.
///
/// For testing, use `CliChannel::with_io()` to supply custom reader/writer.
pub struct CliChannel {
    /// Shared receiver for the stdin reader task. Mutex-protected so `recv()`
    /// can extract it under `&self`.
    rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<Message>>>>,
    _handle: tokio::task::JoinHandle<()>,
    /// Shared writer for outgoing messages. Mutex-protected for &self access.
    writer: Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>,
}

impl CliChannel {
    /// Create a new CLI channel reading from real stdin / writing to real stdout.
    ///
    /// Spawns a background tokio task that reads stdin line-by-line and
    /// forwards each non-empty line as a user message through the internal channel.
    pub fn new() -> Self {
        Self::with_io(
            tokio::io::BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
        )
    }

    /// Create a CLI channel with explicit reader and writer (useful for testing).
    ///
    /// `reader` should be an async reader yielding lines. `writer` receives
    /// serialized messages.
    pub fn with_io<R, W>(reader: R, writer: W) -> Self
    where
        R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        let (tx, rx) = mpsc::channel::<Message>(256);

        let handle = tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                // Try to parse as JSON first (structured input), else treat as raw text
                let msg = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&trimmed) {
                    Message {
                        role: "user".into(),
                        content: val.to_string(),
                        name: None,
                        tool_call_id: None,
                        tool_calls: vec![],
                    }
                } else {
                    Message::user(trimmed)
                };

                if tx.send(msg).await.is_err() {
                    break; // receiver dropped
                }
            }
        });

        Self {
            rx: Arc::new(tokio::sync::Mutex::new(Some(rx))),
            _handle: handle,
            writer: Arc::new(tokio::sync::Mutex::new(Box::new(writer))),
        }
    }
}

impl Default for CliChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelAdapter for CliChannel {
    async fn recv(&self) -> Box<dyn Stream<Item = Message> + Unpin + Send> {
        let rx_opt = self.rx.lock().await.take();
        match rx_opt {
            Some(rx) => Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
            None => {
                // If recv() is called a second time, return an empty stream
                let (_, rx) = mpsc::channel::<Message>(1);
                Box::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
    }

    async fn send(&self, message: Message) -> Result<(), VoltronError> {
        let json = serde_json::to_string(&message)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;
        let mut writer = self.writer.lock().await;
        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| VoltronError::ChannelIO(e.to_string()))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| VoltronError::ChannelIO(e.to_string()))?;
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    /// Helper: create a duplex pair for the channel reader.
    /// Returns (channel, writer) so the test can write data to feed the channel.
    fn make_duplex_channel() -> (CliChannel, tokio::io::DuplexStream) {
        let (writer, reader) = tokio::io::duplex(1024);
        let channel = CliChannel::with_io(tokio::io::BufReader::new(reader), tokio::io::sink());
        (channel, writer)
    }

    #[tokio::test]
    async fn test_recv_text_message() {
        let (channel, mut writer) = make_duplex_channel();

        use tokio::io::AsyncWriteExt;
        writer.write_all(b"Hello, Voltron!\n").await.unwrap();
        writer.shutdown().await.unwrap();

        let mut stream = channel.recv().await;
        let msg = stream.next().await.expect("should receive a message");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Hello, Voltron!");
    }

    #[tokio::test]
    async fn test_recv_json_message() {
        let (channel, mut writer) = make_duplex_channel();

        use tokio::io::AsyncWriteExt;
        let json_msg = r#"{"text": "structured"}"#;
        writer
            .write_all(format!("{json_msg}\n").as_bytes())
            .await
            .unwrap();
        writer.shutdown().await.unwrap();

        let mut stream = channel.recv().await;
        let msg = stream.next().await.expect("should receive a message");
        assert_eq!(msg.role, "user");
        // JSON input gets stringified
        assert!(msg.content.contains("structured"));
    }

    #[tokio::test]
    async fn test_recv_skips_empty_lines() {
        let (channel, mut writer) = make_duplex_channel();

        use tokio::io::AsyncWriteExt;
        writer.write_all(b"\n\nHello\n\nWorld\n").await.unwrap();
        writer.shutdown().await.unwrap();

        let mut stream = channel.recv().await;
        let msg1 = stream.next().await.unwrap();
        assert_eq!(msg1.content, "Hello");
        let msg2 = stream.next().await.unwrap();
        assert_eq!(msg2.content, "World");
    }

    #[tokio::test]
    async fn test_recv_skips_comments() {
        let (channel, mut writer) = make_duplex_channel();

        use tokio::io::AsyncWriteExt;
        writer
            .write_all(b"# this is a comment\nReal message\n")
            .await
            .unwrap();
        writer.shutdown().await.unwrap();

        let mut stream = channel.recv().await;
        let msg = stream.next().await.unwrap();
        assert_eq!(msg.content, "Real message");
    }

    #[tokio::test]
    async fn test_send_message() {
        let (reader_writer, reader) = tokio::io::duplex(1024);
        let (chan_writer, mut chan_reader) = tokio::io::duplex(1024);
        let channel = CliChannel::with_io(tokio::io::BufReader::new(reader), chan_writer);

        // Drop the writer side of the reader so the stdin task stops cleanly
        drop(reader_writer);

        let msg = Message::assistant("Response from agent");
        channel.send(msg).await.unwrap();

        // Read a fixed buffer from the channel reader (don't wait for EOF)
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 1024];
        let n = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            chan_reader.read(&mut buf),
        )
        .await
        .expect("read timed out")
        .expect("read failed");
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("Response from agent"), "got: {output}");
    }
}
