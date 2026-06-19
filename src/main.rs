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
use voltron_core::{AuditSink, LLMProvider, ManifestVerifier, MemoryStore, SkillExecutor, VoltronError};
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

    /// Path to directory containing signed skill manifests (`.ironclaw` files).
    /// When set, IronClaw capability-manifest verification is enabled.
    #[arg(long)]
    ironclaw_manifest_dir: Option<PathBuf>,
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

/// Load signed manifests from a directory and construct an
/// [`IronclawManifestVerifier`].
///
/// Each file in the directory is read and deserialised as a
/// [`SignedManifest`]. The verifier is pre-populated with all
/// successfully loaded manifests.
fn load_ironclaw_manifests(
    dir: &PathBuf,
) -> Result<voltron_ironclaw_adapter::IronclawManifestVerifier, Box<dyn std::error::Error>> {
    use voltron_core::SignedManifest;
    use voltron_ironclaw_adapter::{IronclawManifestVerifier, ManifestRegistry, RevocationRegistry};

    let manifest_registry = ManifestRegistry::new();
    let revocation_registry = RevocationRegistry::new();
    let mut verifier = IronclawManifestVerifier::new(manifest_registry, revocation_registry);

    if !dir.exists() {
        return Err(format!("IronClaw manifest directory does not exist: {:?}", dir).into());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let content = std::fs::read_to_string(&path)?;
            let signed: SignedManifest = serde_json::from_str(&content)?;

            let skill_name = signed.manifest.skill_name.clone();
            let version = signed.manifest.version.clone();

            // register_signed stores the signed manifest AND makes the
            // capability manifest available for hash/permission checks
            verifier.register_signed(signed);

            tracing::info!(
                "Loaded IronClaw manifest: {} (v{})",
                skill_name,
                version,
            );
        }
    }

    Ok(verifier)
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
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                VoltronError::Config("OPENAI_API_KEY environment variable is not set".into())
            });
            let p = match api_key {
                Ok(key) => OpenAIProvider::new(&key, model_override.clone()),
                Err(e) => {
                    tracing::error!("{e}");
                    eprintln!("OPENAI_API_KEY env var required for OpenAI provider");
                    std::process::exit(1);
                }
            };
            Arc::new(p)
        }
        _ => {
            let api_key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| {
                VoltronError::Config("DEEPSEEK_API_KEY environment variable is not set".into())
            });
            let p = match api_key {
                Ok(key) => DeepSeekProvider::new(&key, model_override.clone()),
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

    // ── IronClaw capability-manifest verifier ───────────────────
    let manifest_verifier: Option<Arc<dyn ManifestVerifier>> =
        if let Some(dir) = &cli.ironclaw_manifest_dir {
            match load_ironclaw_manifests(dir) {
                Ok(verifier) => {
                    let v: Arc<dyn ManifestVerifier> = Arc::new(verifier);
                    tracing::info!(
                        "IronClaw manifest verifier enabled — {:?}",
                        dir
                    );
                    Some(v)
                }
                Err(e) => {
                    tracing::error!("Failed to load IronClaw manifests: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            tracing::info!("IronClaw manifest verifier disabled (no --ironclaw-manifest-dir)");
            None
        };

    // ── Build AgentRuntime ──────────────────────────────────────
    let mut runtime_builder = AgentRuntime::builder()
        .provider(llm_provider)
        .memory(memory)
        .skills(skills)
        .channel(channel)
        .audit(audit)
        .config(AgentConfig {
            max_turns,
            ..AgentConfig::default()
        });

    if let Some(verifier) = manifest_verifier {
        runtime_builder = runtime_builder.manifest_verifier(verifier);
    }

    let runtime = runtime_builder.build();

    tracing::info!("Voltron Claw v{} starting", env!("CARGO_PKG_VERSION"));
    tracing::info!("Entering interactive mode. Type Ctrl+C to exit.");

    // ── Run ─────────────────────────────────────────────────────
    runtime.run_loop().await;
}
