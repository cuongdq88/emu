//! Non-interactive CLI used by the SimDeck app.
//!
//! With no subcommand `emu` runs its TUI exactly as before; every
//! subcommand prints the JSON envelope from [`json`] instead.

pub mod check;
pub mod json;

use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Environment diagnostics (SDK path, tool versions, licenses, disk).
    Env(EnvCmd),
}

#[derive(Debug, Args)]
pub struct EnvCmd {
    #[command(subcommand)]
    pub cmd: EnvSub,
}

#[derive(Debug, Subcommand)]
pub enum EnvSub {
    /// Print the environment report.
    Check,
}

/// Runs one subcommand and returns its process exit code.
pub async fn run(cmd: Cmd) -> u8 {
    match cmd {
        Cmd::Env(EnvCmd { cmd: EnvSub::Check }) => check::run().await,
    }
}
