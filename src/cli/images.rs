//! `emu --json images list|install|uninstall`: the system images and iOS
//! runtimes the Runtime Manager shows.

use super::json::CliError;
use super::progress;
use crate::managers::android::sdk;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageRow {
    /// `system-images;android-36;google_apis_playstore;arm64-v8a`, or the
    /// runtime identifier for iOS.
    pub package: String,
    pub api: String,
    pub tag: String,
    pub abi: String,
    pub installed: bool,
    pub size_bytes: u64,
}

pub fn parse_image_row(package: &str, installed: bool, size_bytes: u64) -> Option<ImageRow> {
    let parts: Vec<&str> = package.split(';').collect();
    if parts.len() < 4 || parts[0] != "system-images" {
        return None;
    }
    let api = parts[1].strip_prefix("android-").unwrap_or(parts[1]);
    Some(ImageRow {
        package: package.to_string(),
        api: api.to_string(),
        tag: parts[2].to_string(),
        abi: parts[3].to_string(),
        installed,
        size_bytes,
    })
}

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Every system image the SDK knows about, installed or not, plus installed
/// iOS runtimes when `platform` is ios.
pub async fn list(platform: &str) -> Result<Vec<ImageRow>, CliError> {
    if platform == "ios" {
        let manager = crate::managers::IosManager::new()
            .map_err(|e| CliError::new("simctl_missing", &e.to_string(), None))?;
        let runtimes = manager
            .list_runtimes()
            .await
            .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
        return Ok(runtimes
            .into_iter()
            .map(|(id, name)| ImageRow {
                api: crate::cli::catalog::runtime_version(&name),
                tag: "ios".to_string(),
                abi: String::new(),
                package: id,
                installed: true,
                size_bytes: 0,
            })
            .collect());
    }

    let home = sdk::sdk_home().ok_or_else(|| {
        CliError::new(
            "sdk_not_found",
            "Android SDK not found",
            Some("brew install --cask android-commandlinetools"),
        )
    })?;
    let sdkmanager = sdkmanager_path()?;
    let output = crate::utils::command::CommandRunner::new()
        .run(&sdkmanager, ["--list", "--verbose", "--include_obsolete"])
        .await
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
    let output = crate::managers::AndroidManager::normalize_sdkmanager_output(output);
    let mut rows = Vec::new();
    let mut in_installed = false;
    for line in output.lines() {
        let lower = line.trim().to_lowercase();
        if lower.starts_with("installed packages:") {
            in_installed = true;
            continue;
        }
        if lower.starts_with("available packages:") || lower.starts_with("available updates:") {
            in_installed = false;
            continue;
        }
        let Some(package) = line.split_whitespace().next() else {
            continue;
        };
        if !package.starts_with("system-images;") {
            continue;
        }
        let size = if in_installed {
            let parts: Vec<&str> = package.split(';').collect();
            let dir = home
                .join("system-images")
                .join(format!(
                    "android-{}",
                    parts[1].trim_start_matches("android-")
                ))
                .join(parts.get(2).copied().unwrap_or(""))
                .join(parts.get(3).copied().unwrap_or(""));
            dir_size(&dir)
        } else {
            0
        };
        if let Some(row) = parse_image_row(package, in_installed, size) {
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| {
        b.api
            .cmp(&a.api)
            .then_with(|| a.tag.cmp(&b.tag))
            .then_with(|| a.abi.cmp(&b.abi))
    });
    Ok(rows)
}

pub fn sdkmanager_path() -> Result<std::path::PathBuf, CliError> {
    let home = sdk::sdk_home().ok_or_else(|| {
        CliError::new(
            "sdk_not_found",
            "Android SDK not found",
            Some("brew install --cask android-commandlinetools"),
        )
    })?;
    sdk::tool_in(&home, "sdkmanager").ok_or_else(|| {
        CliError::new(
            "tool_missing",
            "sdkmanager not found in the Android SDK",
            Some("brew install --cask android-commandlinetools"),
        )
    })
}

/// Runs `sdkmanager <args>`, answering license prompts with `y`, and turns
/// its stdout into NDJSON progress. Returns the collected stderr.
async fn stream_sdkmanager(
    args: &[&str],
    answer_licenses: bool,
) -> Result<(bool, String), CliError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let sdkmanager = sdkmanager_path()?;
    let mut child = tokio::process::Command::new(&sdkmanager)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;

    let mut stdin = child.stdin.take();
    if let Some(stdin) = stdin.as_mut() {
        // sdkmanager asks "Accept? (y/N)" once per license; a few Y's up
        // front cover every prompt without deadlocking on a full pipe.
        let _ = stdin.write_all(b"y\ny\ny\ny\ny\ny\ny\ny\n").await;
        let _ = stdin.flush().await;
    }

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer).await;
        String::from_utf8_lossy(&buffer).to_string()
    });

    let mut pending = String::new();
    let mut last_percent: Option<u8> = None;
    let mut chunk = [0u8; 4096];
    loop {
        let read = stdout
            .read(&mut chunk)
            .await
            .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
        if read == 0 {
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&chunk[..read]));
        // sdkmanager redraws its bar with \r, so split on both.
        while let Some(index) = pending.find(['\r', '\n']) {
            let segment: String = pending.drain(..=index).collect();
            let text = segment.trim();
            if text.is_empty() {
                continue;
            }
            if answer_licenses && (text.contains("(y/N)") || text.contains("[y/n]")) {
                if let Some(stdin) = stdin.as_mut() {
                    let _ = stdin.write_all(b"y\n").await;
                    let _ = stdin.flush().await;
                }
            }
            if let Some(percent) = progress::parse_sdkmanager_percent(text) {
                if last_percent != Some(percent) {
                    last_percent = Some(percent);
                    progress::emit_progress(percent, text);
                }
            }
        }
    }
    if !pending.trim().is_empty() {
        progress::emit_progress(last_percent.unwrap_or(0), pending.trim());
    }

    let status = child
        .wait()
        .await
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
    let stderr = stderr_task.await.unwrap_or_default();
    Ok((status.success(), stderr))
}

/// Installs one package, streaming progress; the last line is the envelope.
pub async fn install(package: &str) -> u8 {
    progress::emit_progress(0, &format!("Installing {package}"));
    match stream_sdkmanager(&[package], true).await {
        Ok((true, _)) => {
            progress::emit_done(serde_json::json!({"package": package, "installed": true}));
            0
        }
        Ok((false, stderr)) => {
            let code = if stderr.to_lowercase().contains("license") {
                "license_required"
            } else {
                "tool_failed"
            };
            progress::emit_stream_error(
                code,
                stderr.trim(),
                if code == "license_required" {
                    Some("Run: emu --json licenses accept")
                } else {
                    None
                },
            );
            crate::cli::json::exit_code_for(code)
        }
        Err(error) => {
            progress::emit_stream_error(&error.code, &error.message, error.hint.as_deref());
            crate::cli::json::exit_code_for(&error.code)
        }
    }
}

pub async fn uninstall(package: &str) -> Result<serde_json::Value, CliError> {
    let sdkmanager = sdkmanager_path()?;
    let output = tokio::process::Command::new(&sdkmanager)
        .args(["--uninstall", package])
        .output()
        .await
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
    if !output.status.success() {
        return Err(CliError::new(
            "tool_failed",
            String::from_utf8_lossy(&output.stderr).trim(),
            None,
        ));
    }
    Ok(serde_json::json!({"package": package, "removed": true}))
}

/// `sdkmanager --licenses`, answering every prompt, then reporting whether
/// the license files are all populated.
pub async fn accept_licenses() -> u8 {
    progress::emit_progress(0, "Accepting SDK licenses");
    match stream_sdkmanager(&["--licenses"], true).await {
        Ok((success, stderr)) => {
            let accepted = sdk::sdk_home()
                .map(|home| crate::cli::catalog::license_state(&home).accepted)
                .unwrap_or(false);
            if success && accepted {
                progress::emit_done(serde_json::json!({"accepted": true}));
                0
            } else {
                let message = if stderr.trim().is_empty() {
                    "Some SDK licenses are still not accepted"
                } else {
                    stderr.trim()
                };
                progress::emit_stream_error("license_required", message, None);
                1
            }
        }
        Err(error) => {
            progress::emit_stream_error(&error.code, &error.message, error.hint.as_deref());
            crate::cli::json::exit_code_for(&error.code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_image_package_into_columns() {
        let row = parse_image_row(
            "system-images;android-36.1;google_apis_playstore;arm64-v8a",
            true,
            123,
        )
        .unwrap();
        assert_eq!(row.api, "36.1");
        assert_eq!(row.tag, "google_apis_playstore");
        assert_eq!(row.abi, "arm64-v8a");
        assert!(row.installed);
        assert_eq!(row.size_bytes, 123);
    }

    #[test]
    fn ignores_packages_that_are_not_system_images() {
        assert!(parse_image_row("platforms;android-36", true, 0).is_none());
    }
}
