 // Voltron Claw — composite agent binary
// License: Apache-2.0
//
// Wires all Phase 1 crates together through the AgentRuntime and
// enters the agent run loop.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use voltron_audit::{FileAuditSink, InMemoryAuditSink};
use voltron_channels::CliChannel;
use voltron_core::{AuditSink, LLMProvider, MemoryStore, SkillExecutor};
use voltron_memory::{InMemoryStore, SqliteStore};
use voltron_providers::{DeepSeekProvider, OpenAIProvider};
use voltron_runtime::{AgentConfig, AgentRuntime};
use voltron_skills::LocalSkillExecutor;

// ── CLI ───────────────────────────────────────────────────────────

/// Voltron Claw — Rust-native composite agent.
#[derive(Parser, Debug)]
#[command(name = "voltron", version, about)]
struct Cli {
    /// Path to TOML configuration file.
    #[arg(short, long, default_value = "voltron.toml")]
    config: PathBuf,

    /// LLM provider: "deepseek" (default) or "openai".
    #[arg(short, long, default_value = "deepseek")]
    provider: String,

    /// Override the model name (e.g. "deepseek-reasoner" or "gpt-4o-mini").
    #[arg(short = 'M', long)]
    model: Option<String>,

    /// SQLite database path for persistent memory. Defaults to in-memory.
    #[arg(short = 'D', long)]
    database: Option<PathBuf>,

    /// File path for JSONL audit trail. Defaults to in-memory audit.
    #[arg(short = 'A', long)]
    audit_log: Option<PathBuf>,

    /// Maximum number of conversation turns before exit (0 = unlimited).
    #[arg(short = 'n', long, default_value_t = 0)]
    max_turns: u32,
}

// ─── Config file (optional TOML) ──────────────────────────────────

#[derive(serde::Deserialize)]
struct Config {
    provider: Option<String>,
    model: Option<String>,
    database: Option<String>,
    audit_log: Option<String>,
    max_turns: Option<u32>,
}

fn load_config(path: &PathBuf) -> Option<Config> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

// ─── Entry point ──────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // ── Logging ────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // ── CLI + config merge ─────────────────────────────────────
    let cli = Cli::parse();
    let cfg = load_config(&cli.config);

    let provider_name = cfg
        .as_ref()
        .and_then(|c| c.provider.clone())
        .unwrap_or(cli.provider);
    let model_override = cli
        .model
        .clone()
        .or_else(|| cfg.as_ref().and_then(|c| c.model.clone()));

    // ── Construct LLM provider ─────────────────────────────────
    let llm_provider: Arc<dyn LLMProvider> = match provider_name.as_str() {
        "openai" => {
            let p = match OpenAIProvider::from_env() {
                Ok(provider) => provider,
                Err(e) => {
                    tracing::error!("{e}");
                    eprintln!("OPENAI_API_KEY env var required for OpenAI provider");
                    std::process::exit(1);
                }
            };
            Arc::new(p)
        }
        _ => {
            let p = match DeepSeekProvider::from_env() {
                Ok(provider) => provider,
                Err(e) => {
                    tracing::error!("{e}");
                    eprintln!("DEEPSEEK_API_KEY env var required for DeepSeek provider");
                    std::process::exit(1);
                }
            };
            Arc::new(p)
        }
    };

    tracing::info!(
        provider = %llm_provider.provider_name(),
        "LLM provider initialised",
    );

    // ── Construct memory store ─────────────────────────────────
    let db_path = cli
        .database
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| cfg.as_ref().and_then(|c| c.database.clone()));

    let memory: Arc<dyn MemoryStore> = if let Some(path) = db_path {
        match SqliteStore::connect(&path).await {
            Ok(store) => Arc::new(store),
            Err(e) => {
                tracing::error!("Failed to connect to SQLite: {e}");
                std::process::exit(1);
            }
        }
    } else {
        Arc::new(InMemoryStore::new())
    };

    tracing::info!("Memory store initialised");

    // ── Construct skill executor ───────────────────────────────
    let skills = Arc::new(LocalSkillExecutor::with_defaults());
    tracing::info!(
        count = skills.manifests().len(),
        "Skill executor initialised",
    );

    // ── Construct channel ──────────────────────────────────────
    let channel = Arc::new(CliChannel::new());
    tracing::info!("CLI channel initialised");

    // ── Construct audit sink ───────────────────────────────────
    let audit_path = cli
        .audit_log
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| cfg.as_ref().and_then(|c| c.audit_log.clone()));

    let audit: Arc<dyn AuditSink> = if let Some(path) = audit_path {
        match FileAuditSink::new(&path) {
            Ok(sink) => Arc::new(sink),
            Err(e) => {
                tracing::error!("Failed to open audit log: {e}");
                std::process::exit(1);
            }
        }
    } else {
        Arc::new(InMemoryAuditSink::new())
    };

    tracing::info!("Audit sink initialised");

    // ── Resolve max_turns config ────────────────────────────────
    let max_turns = cfg
        .as_ref()
        .and_then(|c| c.max_turns)
        .unwrap_or(cli.max_turns);

    // ── Build AgentRuntime ──────────────────────────────────────
    let runtime = AgentRuntime::builder()
        .provider(llm_provider)
        .memory(memory)
        .skills(skills)
        .channel(channel)
        .audit(audit)
        .config(AgentConfig {
            max_turns,
            ..AgentConfig::default()
        })
        .build();

    tracing::info!("Voltron Claw v{} starting", env!("CARGO_PKG_VERSION"));
    tracing::info!("Entering interactive mode. Type Ctrl+C to exit.");

    // ── Run ─────────────────────────────────────────────────────
    runtime.run_loop().await;
}

// ── Helpers ───────────────────────────────────────────────────────

#[allow(clippy::unnecessary_cast)]
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let a = now.as_secs();
    let b = now.subsec_nanos() as u64;
    let c = a.wrapping_mul(b).wrapping_add(0xdeadbeef);
    let d = b.wrapping_mul(0x9e3779b9).wrapping_add(a);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a as u32) ^ (c as u32),
        (b as u16) ^ (d as u16),
        ((c >> 20) as u16) & 0x0fff,
        0x8000u16 | ((d >> 8) as u16 & 0x0fff),
        (c ^ d) & 0xffff_ffff_ffff,
    )
}

/// Return the current UTC time as an ISO-8601 string.
fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            m = i + 1;
            break;
        }
        remaining -= md as i64;
    }
    if m == 0 {
        m = 12;
    }
    let d = (remaining + 1) as u8;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso_now_format() {
        let s = iso_now();
        assert!(s.len() >= 20, "ISO string too short: {s}");
        assert!(s.ends_with('Z'), "Should end with Z: {s}");
    }

    #[test]
    fn test_uuid_format() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36, "UUID should be 36 chars: {id}");
    }

    #[test]
    fn test_is_leap() {
        assert!(is_leap(2000));
        assert!(!is_leap(1900));
        assert!(is_leap(2024));
        assert!(!is_leap(2023));
    }
}
