//! Non-interactive CLI used by the SimDeck app.
//!
//! With no subcommand `emu` runs its TUI exactly as before; every
//! subcommand prints the JSON envelope from [`json`] instead.

pub mod catalog;
pub mod check;
pub mod config_ini;
pub mod create;
pub mod images;
pub mod json;
pub mod progress;

use clap::{Args, Subcommand};
use create::CreateRequest;
use json::{emit_cli_err, emit_err, emit_ok};

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
    /// System images and iOS runtimes.
    Images(ImagesCmd),
}

#[derive(Debug, Args)]
pub struct ImagesCmd {
    #[command(subcommand)]
    pub cmd: ImagesSub,
}

#[derive(Debug, Subcommand)]
pub enum ImagesSub {
    /// Installed and downloadable images.
    List {
        #[arg(long, default_value = "android")]
        platform: String,
    },
    /// Install a package, streaming NDJSON progress.
    Install {
        #[arg(long)]
        package: String,
    },
    /// Uninstall a package.
    Uninstall {
        #[arg(long)]
        package: String,
    },
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
    /// Create an AVD from a hardware profile and an installed system image.
    Create(CreateArgs),
    /// Print an AVD's config.ini and the fields the Edit sheet may change.
    Show {
        #[arg(long)]
        name: String,
    },
    /// Change the editable fields of a stopped AVD.
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(long)]
    pub name: String,
    /// RAM in MB.
    #[arg(long)]
    pub ram: Option<u32>,
    /// Data partition size, e.g. 8G.
    #[arg(long)]
    pub storage: Option<String>,
    /// Back camera source (virtualscene, emulated, none or a webcam id).
    #[arg(long)]
    pub camera: Option<String>,
    /// portrait or landscape.
    #[arg(long)]
    pub orientation: Option<String>,
    #[arg(long)]
    pub keyboard: Option<bool>,
    /// SD card size, e.g. 512M, or "none".
    #[arg(long)]
    pub sdcard: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub api: u32,
    #[arg(long)]
    pub tag: String,
    #[arg(long)]
    pub abi: String,
    #[arg(long)]
    pub device: String,
    /// RAM in MB.
    #[arg(long)]
    pub ram: Option<u32>,
    /// Data partition size, e.g. 8G.
    #[arg(long)]
    pub storage: Option<String>,
    /// Back camera source (virtualscene, emulated, none or a webcam id).
    #[arg(long)]
    pub camera: Option<String>,
    /// portrait or landscape.
    #[arg(long)]
    pub orientation: Option<String>,
    #[arg(long)]
    pub keyboard: Option<bool>,
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
    /// Accept every SDK license, streaming NDJSON progress.
    Accept,
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
            AvdSub::Create(args) => match create::create(CreateRequest {
                name: args.name,
                api: args.api,
                tag: args.tag,
                abi: args.abi,
                device: args.device,
                ram_mb: args.ram,
                storage: args.storage,
                camera: args.camera,
                orientation: args.orientation,
                keyboard: args.keyboard,
            })
            .await
            {
                Ok(created) => emit_ok(created),
                Err(e) => emit_cli_err(&e),
            },
            AvdSub::Show { name } => match config_ini::show(&name) {
                Ok(result) => emit_ok(result),
                Err(e) => emit_cli_err(&e),
            },
            AvdSub::Update(args) => {
                let fields = config_ini::EditFields {
                    ram_mb: args.ram,
                    storage: args.storage,
                    camera: args.camera,
                    orientation: args.orientation,
                    keyboard: args.keyboard,
                    sdcard: args.sdcard,
                };
                match config_ini::update(&args.name, fields).await {
                    Ok(result) => emit_ok(result),
                    Err(e) => emit_cli_err(&e),
                }
            }
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
        Cmd::Images(ImagesCmd { cmd }) => match cmd {
            ImagesSub::List { platform } => match images::list(&platform).await {
                Ok(rows) => emit_ok(rows),
                Err(e) => emit_cli_err(&e),
            },
            ImagesSub::Install { package } => images::install(&package).await,
            ImagesSub::Uninstall { package } => match images::uninstall(&package).await {
                Ok(value) => emit_ok(value),
                Err(e) => emit_cli_err(&e),
            },
        },
        Cmd::Licenses(LicensesCmd { cmd }) => match cmd {
            LicensesSub::Accept => images::accept_licenses().await,
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
