//! Non-interactive CLI used by the SimDeck app.
//!
//! With no subcommand `emu` runs its TUI exactly as before; every
//! subcommand prints the JSON envelope from [`json`] instead.

pub mod catalog;
pub mod check;
pub mod json;

use clap::{Args, Subcommand};
use json::{emit_err, emit_ok};

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Environment diagnostics (SDK path, tool versions, licenses, disk).
    Env(EnvCmd),
    /// Android virtual device catalogs and operations.
    Avd(AvdCmd),
    /// iOS simulator catalogs and operations.
    Ios(IosCmd),
    /// SDK license state.
    Licenses(LicensesCmd),
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

#[derive(Debug, Args)]
pub struct AvdCmd {
    #[command(subcommand)]
    pub cmd: AvdSub,
}

#[derive(Debug, Subcommand)]
pub enum AvdSub {
    /// Hardware profiles with their screen size.
    Profiles,
    /// Tags and ABIs for API level N.
    Tags {
        #[arg(long)]
        api: u32,
    },
}

#[derive(Debug, Args)]
pub struct IosCmd {
    #[command(subcommand)]
    pub cmd: IosSub,
}

#[derive(Debug, Subcommand)]
pub enum IosSub {
    /// Simulator device types.
    DeviceTypes,
    /// Installed simulator runtimes.
    Runtimes,
}

#[derive(Debug, Args)]
pub struct LicensesCmd {
    #[command(subcommand)]
    pub cmd: LicensesSub,
}

#[derive(Debug, Subcommand)]
pub enum LicensesSub {
    /// Whether every SDK license has been accepted.
    Status,
}

/// Runs one subcommand and returns its process exit code.
pub async fn run(cmd: Cmd) -> u8 {
    match cmd {
        Cmd::Env(EnvCmd { cmd: EnvSub::Check }) => check::run().await,
        Cmd::Avd(AvdCmd { cmd }) => match cmd {
            AvdSub::Profiles => match catalog::profiles().await {
                Ok(rows) => emit_ok(rows),
                Err(e) => android_err(e),
            },
            AvdSub::Tags { api } => match catalog::tags(api).await {
                Ok(rows) => emit_ok(rows),
                Err(e) if e.to_string().contains("No system image information") => emit_err(
                    "image_missing",
                    &e.to_string(),
                    Some("Install an image for this API first"),
                ),
                Err(e) => android_err(e),
            },
        },
        Cmd::Ios(IosCmd { cmd }) => match cmd {
            IosSub::DeviceTypes => match catalog::ios_device_types().await {
                Ok(rows) => emit_ok(rows),
                Err(e) => ios_err(e),
            },
            IosSub::Runtimes => match catalog::ios_runtimes().await {
                Ok(rows) => emit_ok(rows),
                Err(e) => ios_err(e),
            },
        },
        Cmd::Licenses(LicensesCmd { cmd }) => match cmd {
            LicensesSub::Status => match crate::managers::AndroidManager::new() {
                Ok(manager) => emit_ok(catalog::license_state(manager.android_home())),
                Err(e) => emit_err(
                    "sdk_not_found",
                    &e.to_string(),
                    Some("Install the Android SDK and set ANDROID_HOME"),
                ),
            },
        },
    }
}

fn android_err(e: anyhow::Error) -> u8 {
    let message = e.to_string();
    if message.contains("Android SDK not found") {
        emit_err(
            "sdk_not_found",
            &message,
            Some("Install the Android SDK and set ANDROID_HOME"),
        )
    } else {
        emit_err("tool_failed", &message, None)
    }
}

fn ios_err(e: anyhow::Error) -> u8 {
    let message = e.to_string();
    if message.contains("Xcode") {
        emit_err(
            "xcode_missing",
            &message,
            Some("Install Xcode or run xcode-select --install"),
        )
    } else {
        emit_err("tool_failed", &message, None)
    }
}
