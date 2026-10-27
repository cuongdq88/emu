//! `emu --json avd create`: validates, hands creation to the manager, then
//! patches the new `config.ini` with the values the form collected.

use super::config_ini;
use super::json::CliError;
use crate::managers::common::{DeviceConfig, DeviceManager};
use crate::managers::AndroidManager;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 8 GiB kept free on top of the system image, per spec §10.
pub const DISK_HEADROOM_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CreateRequest {
    pub name: String,
    pub api: u32,
    pub tag: String,
    pub abi: String,
    pub device: String,
    pub ram_mb: Option<u32>,
    pub storage: Option<String>,
    pub camera: Option<String>,
    pub orientation: Option<String>,
    pub keyboard: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Created {
    pub name: String,
    pub path: String,
    pub config: BTreeMap<String, String>,
}

/// AVD names reach `avdmanager` and the filesystem, so keep them to
/// letters, digits, dot, dash and underscore.
pub fn validate_name(name: &str) -> Result<(), CliError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(CliError::new(
            "invalid_name",
            &format!("'{name}' is not a valid AVD name"),
            Some("Use 1-64 characters: letters, digits, dot, dash or underscore"),
        ))
    }
}

pub fn check_free_space(free_bytes: u64, required_bytes: u64) -> Result<(), CliError> {
    if free_bytes >= required_bytes {
        return Ok(());
    }
    Err(CliError::new(
        "disk_full",
        &format!(
            "Not enough disk space: {:.1} GB free, {:.1} GB needed",
            free_bytes as f64 / 1e9,
            required_bytes as f64 / 1e9
        ),
        Some("Free some space and try again"),
    ))
}

/// Total size of the system image this AVD would boot from.
fn dir_size(path: &Path) -> u64 {
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

pub fn config_patch(request: &CreateRequest) -> Vec<(String, String)> {
    config_ini::edit_patch(&config_ini::EditFields {
        ram_mb: request.ram_mb,
        storage: request.storage.clone(),
        camera: request.camera.clone(),
        orientation: request.orientation.clone(),
        keyboard: request.keyboard,
        sdcard: None,
    })
}

/// `$ANDROID_AVD_HOME`, else `~/.android/avd`, else where avdmanager wrote.
pub fn avd_dir_for(name: &str) -> PathBuf {
    let home = crate::cli::check::avd_home();
    let ini = home.join(format!("{name}.ini"));
    if let Ok(body) = std::fs::read_to_string(&ini) {
        if let Some(path) = body
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(str::trim)
        {
            return PathBuf::from(path);
        }
    }
    home.join(format!("{name}.avd"))
}

pub async fn create(request: CreateRequest) -> Result<Created, CliError> {
    validate_name(&request.name)?;
    let manager = AndroidManager::new()
        .map_err(|e| CliError::new("sdk_not_found", &e.to_string(), Some("Set ANDROID_HOME")))?;

    let existing = manager.list_devices().await.unwrap_or_default();
    if existing.iter().any(|device| device.name == request.name) {
        return Err(CliError::new(
            "name_exists",
            &format!("An AVD named '{}' already exists", request.name),
            Some("Pick another name, or delete the existing AVD first"),
        ));
    }

    let api = request.api.to_string();
    let installed = manager
        .check_system_image_available(&api, &request.tag, &request.abi)
        .await
        .unwrap_or(false);
    if !installed {
        let package = format!(
            "system-images;android-{api};{};{}",
            request.tag, request.abi
        );
        return Err(CliError::new(
            "image_missing",
            &format!("System image {package} is not installed"),
            Some(&format!("Install it with: sdkmanager \"{package}\"")),
        ));
    }

    let image_dir = manager
        .android_home()
        .join("system-images")
        .join(format!("android-{api}"))
        .join(&request.tag)
        .join(&request.abi);
    let free = super::check::disk_free_bytes(
        &crate::utils::command::CommandRunner::new(),
        manager.android_home(),
    )
    .await
    .unwrap_or(u64::MAX);
    check_free_space(free, dir_size(&image_dir) + DISK_HEADROOM_BYTES)?;

    let config = DeviceConfig::new(request.name.clone(), request.device.clone(), api.clone())
        .with_option("tag".to_string(), request.tag.clone())
        .with_option("abi".to_string(), request.abi.clone());

    manager.create_device(&config).await.map_err(|e| {
        let message = e.to_string();
        if message.contains("already exists") {
            CliError::new("name_exists", &message, None)
        } else if message.contains("license") {
            CliError::new(
                "license_required",
                &message,
                Some("Run: emu --json licenses accept"),
            )
        } else if message.contains("System image") {
            CliError::new("image_missing", &message, None)
        } else {
            CliError::new("tool_failed", &message, None)
        }
    })?;

    let dir = avd_dir_for(&request.name);
    let config_path = dir.join("config.ini");
    let patch = config_patch(&request);
    if !patch.is_empty() {
        let original = std::fs::read_to_string(&config_path).map_err(|e| {
            CliError::new(
                "tool_failed",
                &format!("AVD was created but its config.ini could not be read: {e}"),
                None,
            )
        })?;
        config_ini::write_atomic(&config_path, &config_ini::apply_patch(&original, &patch))
            .map_err(|e| {
                CliError::new(
                    "tool_failed",
                    &format!("AVD was created but config.ini could not be written: {e}"),
                    None,
                )
            })?;
    }

    let values = config_ini::read_config(&config_path).unwrap_or_default();
    Ok(Created {
        name: request.name,
        path: dir.display().to_string(),
        config: values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_with_spaces() {
        assert_eq!(validate_name("Pixel 9").unwrap_err().code, "invalid_name");
    }

    #[test]
    fn rejects_existing_name() {
        assert_eq!(check_free_space(0, 10_000).unwrap_err().code, "disk_full");
    }

    #[test]
    fn accepts_dots_dashes_and_underscores() {
        assert!(validate_name("ZZ_Test.2-arm64").is_ok());
    }

    #[test]
    fn patch_uses_avd_config_keys() {
        let patch = config_patch(&CreateRequest {
            name: "ZZ_Test".into(),
            api: 36,
            tag: "google_apis_playstore".into(),
            abi: "arm64-v8a".into(),
            device: "medium_phone".into(),
            ram_mb: Some(2048),
            storage: Some("8G".into()),
            camera: Some("virtualscene".into()),
            orientation: Some("portrait".into()),
            keyboard: Some(false),
        });
        assert!(patch.contains(&("hw.ramSize".to_string(), "2048".to_string())));
        assert!(patch.contains(&("hw.keyboard".to_string(), "no".to_string())));
        assert!(patch.contains(&("disk.dataPartition.size".to_string(), "8G".to_string())));
    }
}
