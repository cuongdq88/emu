use super::AndroidManager;
use crate::constants::{env_vars, files, limits::SYSTEM_IMAGE_PARTS_REQUIRED};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Resolves the SDK home: the environment wins, then the Android Studio
/// default, then Homebrew's command-line tools cask. `exists` lets tests
/// answer without touching the disk.
pub(crate) fn resolve_android_home(
    android_home: Option<String>,
    sdk_root: Option<String>,
    home: Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(path) = android_home.filter(|value| !value.trim().is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = sdk_root.filter(|value| !value.trim().is_empty()) {
        return Some(PathBuf::from(path));
    }
    let home = home?;
    [
        format!("{home}/Library/Android/sdk"),
        "/opt/homebrew/share/android-commandlinetools".to_string(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| exists(path))
}

impl AndroidManager {
    /// Locates the Android SDK home directory.
    pub(super) fn find_android_home() -> Result<PathBuf> {
        let resolved = resolve_android_home(
            std::env::var(env_vars::ANDROID_HOME).ok(),
            std::env::var(env_vars::ANDROID_SDK_ROOT).ok(),
            std::env::var("HOME").ok(),
            |path| path.exists(),
        );
        resolved.ok_or_else(|| {
            anyhow::anyhow!(
                "Android SDK not found. Set ANDROID_HOME, or install it with: \
                 brew install --cask android-commandlinetools"
            )
        })
    }

    /// Finds a specific tool within the Android SDK directory structure.
    pub(super) fn find_tool(android_home: &Path, tool: &str) -> Result<PathBuf> {
        let paths = [
            android_home
                .join(files::android::CMDLINE_TOOLS_LATEST_BIN)
                .join(tool),
            android_home.join(files::android::TOOLS_BIN).join(tool),
            android_home.join(files::android::EMULATOR_DIR).join(tool),
        ];

        for path in &paths {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        bail!("Tool '{tool}' not found in Android SDK")
    }

    pub async fn check_system_image_available(
        &self,
        api_level: &str,
        tag: &str,
        abi: &str,
    ) -> Result<bool> {
        let package_path = format!("system-images;android-{api_level};{tag};{abi}");
        let installed_images = self.list_available_system_images().await?;
        Ok(installed_images.contains(&package_path))
    }

    pub async fn list_available_system_images(&self) -> Result<Vec<String>> {
        let mut images = Vec::new();
        let output = self.get_sdkmanager_verbose_output().await?;
        let mut in_installed_section = false;

        for line in output.lines() {
            let trimmed = line.trim();

            // cmdline-tools 23 prints "Available packages:" (lowercase p).
            let lower = trimmed.to_lowercase();
            if lower.starts_with("installed packages:") {
                in_installed_section = true;
                continue;
            }

            if in_installed_section
                && (lower.starts_with("available packages:")
                    || lower.starts_with("available updates:"))
            {
                in_installed_section = false;
                continue;
            }

            if in_installed_section && trimmed.starts_with("system-images;") {
                if let Some(space_pos) = trimmed.find(' ') {
                    let package_path = &trimmed[..space_pos];
                    images.push(package_path.to_string());
                } else {
                    images.push(trimmed.to_string());
                }
            }
        }

        Ok(images)
    }

    pub async fn get_first_available_system_image(
        &self,
        api_level: &str,
    ) -> Result<Option<(String, String)>> {
        let installed_images = self.list_available_system_images().await?;

        for image in installed_images {
            let parts: Vec<&str> = image.split(';').collect();
            if parts.len() >= SYSTEM_IMAGE_PARTS_REQUIRED {
                if let Some(android_part) = parts.get(1) {
                    if android_part == &format!("android-{api_level}") {
                        return Ok(Some((parts[2].to_string(), parts[3].to_string())));
                    }
                }
            }
        }

        Ok(None)
    }
}
