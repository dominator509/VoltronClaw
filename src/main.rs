// Voltron Claw — binary entry point
// License: Apache-2.0

use std::process;

fn main() {
    tracing_subscriber::fmt::init();

    tracing::info!("Voltron Claw v{} starting", env!("CARGO_PKG_VERSION"));

    // TODO: Phase 1 — parse CLI args, load config, construct Agent, run_loop()
    tracing::warn!("Runtime loop not yet implemented — Phase 1 pending");

    process::exit(0);
}
