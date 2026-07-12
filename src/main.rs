//! Respondami — A privacy-first coding agent. One binary, no telemetry, skills-driven.

use mimalloc::MiMalloc;
use respondami::run;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run().await
}
