//! `emu --json env check`: where the SDK and Xcode are, and which versions
//! of each tool the app will be calling.

use super::json::{emit_err, ok_value, print_value};
use crate::constants::commands::ADB;
use crate::managers::android::sdk;
use crate::utils::command::CommandRunner;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub path: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct XcodeInfo {
    pub present: bool,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnvReport {
    pub sdk_path: String,
    pub avd_home: String,
    pub tools: Tools,
    pub xcode: XcodeInfo,
    pub disk_free_bytes: Option<u64>,
    pub licenses_accepted: bool,
    /// True when the emulator, avdmanager and adb are all present: the app
    /// can create and boot AVDs without further setup.
    pub ready: bool,
    /// Tools the user still has to install, for the first-run setup.
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Tools {
    pub adb: Option<ToolInfo>,
    pub emulator: Option<ToolInfo>,
    pub avdmanager: Option<ToolInfo>,
    pub sdkmanager: Option<ToolInfo>,
}

/// `$ANDROID_AVD_HOME`, else `~/.android/avd` — the directory the app reads
/// for the AVD list.
pub fn avd_home() -> PathBuf {
    if let Ok(dir) = std::env::var("ANDROID_AVD_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".android")
        .join("avd")
}

/// License files `sdkmanager --licenses` writes; a populated
/// `android-sdk-license` means the main license was accepted.
pub fn licenses_accepted(sdk_path: &Path) -> bool {
    let file = sdk_path.join("licenses").join("android-sdk-license");
    std::fs::read_to_string(file)
        .map(|body| body.lines().any(|line| !line.trim().is_empty()))
        .unwrap_or(false)
}

/// Free bytes on the volume holding `path`, via `df -k -P`.
pub async fn disk_free_bytes(exec: &CommandRunner, path: &Path) -> Option<u64> {
    let output = exec
        .run("df", ["-k", "-P", &path.to_string_lossy()])
        .await
        .ok()?;
    let line = output.lines().nth(1)?;
    let available_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kb * 1024)
}

pub async fn run() -> u8 {
    let Some(sdk_path) = sdk::sdk_home() else {
        return emit_err(
            "sdk_not_found",
            "Android SDK not found",
            Some("brew install --cask android-commandlinetools"),
        );
    };
    let exec = CommandRunner::new();

    let adb = match sdk::tool_in(&sdk_path, ADB) {
        Some(path) => tool(&exec, path, &["--version"]).await,
        None => None,
    };
    let emulator = match sdk::tool_in(&sdk_path, "emulator") {
        Some(path) => tool(&exec, path, &["-version"]).await,
        None => None,
    };
    let avdmanager = match sdk::tool_in(&sdk_path, "avdmanager") {
        Some(path) => Some(ToolInfo {
            version: cmdline_tools_version(&path),
            path: path.display().to_string(),
        }),
        None => None,
    };
    let sdkmanager = match sdk::tool_in(&sdk_path, "sdkmanager") {
        Some(path) => tool(&exec, path, &["--version"]).await,
        None => None,
    };

    let mut missing = Vec::new();
    if adb.is_none() {
        missing.push("platform-tools".to_string());
    }
    if emulator.is_none() {
        missing.push("emulator".to_string());
    }
    if avdmanager.is_none() {
        missing.push("cmdline-tools".to_string());
    }
    let ready = missing.is_empty();

    let report = EnvReport {
        sdk_path: sdk_path.display().to_string(),
        avd_home: avd_home().display().to_string(),
        tools: Tools {
            adb,
            emulator,
            avdmanager,
            sdkmanager,
        },
        xcode: xcode(&exec).await,
        disk_free_bytes: disk_free_bytes(&exec, &sdk_path).await,
        licenses_accepted: licenses_accepted(&sdk_path),
        ready,
        missing,
    };
    print_value(&ok_value(report));
    0
}

/// First non-empty line of `<tool> --version`, which is what every SDK tool
/// prints as its version banner.
async fn tool(exec: &CommandRunner, path: PathBuf, args: &[&str]) -> Option<ToolInfo> {
    if !path.exists() {
        return None;
    }
    let version = exec
        .run(&path, args)
        .await
        .ok()
        .and_then(|out| {
            out.lines()
                .find(|l| !l.trim().is_empty())
                .map(str::to_string)
        })
        .map(|l| l.trim().to_string());
    Some(ToolInfo {
        path: path.display().to_string(),
        version,
    })
}

/// `cmdline-tools/<revision>/source.properties` next to the binary.
fn cmdline_tools_version(binary: &Path) -> Option<String> {
    let props = binary.parent()?.parent()?.join("source.properties");
    let body = std::fs::read_to_string(props).ok()?;
    body.lines()
        .find_map(|line| line.trim().strip_prefix("Pkg.Revision="))
        .map(str::to_string)
}

async fn xcode(exec: &CommandRunner) -> XcodeInfo {
    if !cfg!(target_os = "macos") {
        return XcodeInfo {
            present: false,
            version: None,
        };
    }
    let output = exec.run("xcodebuild", ["-version"]).await.ok();
    XcodeInfo {
        present: output.is_some(),
        version: output.and_then(|out| {
            out.lines()
                .find(|l| l.starts_with("Xcode"))
                .map(|l| l.trim().to_string())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avd_home_prefers_android_avd_home() {
        std::env::set_var("ANDROID_AVD_HOME", "/tmp/avd-test-home");
        assert_eq!(avd_home(), PathBuf::from("/tmp/avd-test-home"));
        std::env::remove_var("ANDROID_AVD_HOME");
    }

    #[test]
    fn missing_license_file_is_not_accepted() {
        assert!(!licenses_accepted(Path::new("/definitely/not/an/sdk")));
    }
}
