//! `config.ini` editing that keeps every byte it does not change, and
//! writes atomically so a crash never leaves a half-written AVD config.

use std::io::Write;
use std::path::Path;

/// Replaces the keys named in `patch`, in place and in file order, keeping
/// comments, unknown keys and each line's own ending; new keys are appended.
pub fn apply_patch(original: &str, patch: &[(String, String)]) -> String {
    let mut remaining: Vec<(&str, &str)> = patch
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let mut out = String::with_capacity(original.len() + 64);

    for line in original.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let replaced = body.split_once('=').and_then(|(key, _)| {
            let key = key.trim();
            let index = remaining.iter().position(|(name, _)| *name == key)?;
            let (_, value) = remaining.remove(index);
            Some(format!("{key}={value}"))
        });
        out.push_str(&replaced.unwrap_or_else(|| body.to_string()));
        out.push_str(ending);
    }

    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    for (key, value) in remaining {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

/// Writes `config.ini` through a sibling temp file and a rename, so readers
/// (adb, the emulator) never see a partial file.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let temp = path.with_extension("ini.tmp");
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)
}

/// Parses `key=value` lines; comments and blanks are skipped.
pub fn read_config(path: &Path) -> std::io::Result<std::collections::BTreeMap<String, String>> {
    let body = std::fs::read_to_string(path)?;
    Ok(body
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once('=')
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        })
        .collect())
}

/// The fields the Edit sheet may change; everything else is locked because
/// changing it means a different AVD.
#[derive(Debug, Clone, Default)]
pub struct EditFields {
    pub ram_mb: Option<u32>,
    /// e.g. "8G".
    pub storage: Option<String>,
    /// Back camera source.
    pub camera: Option<String>,
    /// portrait or landscape.
    pub orientation: Option<String>,
    pub keyboard: Option<bool>,
    /// e.g. "512M", or "none" to remove the card.
    pub sdcard: Option<String>,
}

pub fn edit_patch(fields: &EditFields) -> Vec<(String, String)> {
    let mut patch = Vec::new();
    if let Some(ram) = fields.ram_mb {
        patch.push(("hw.ramSize".to_string(), ram.to_string()));
    }
    if let Some(storage) = &fields.storage {
        patch.push(("disk.dataPartition.size".to_string(), storage.clone()));
    }
    if let Some(camera) = &fields.camera {
        patch.push(("hw.camera.back".to_string(), camera.clone()));
    }
    if let Some(orientation) = &fields.orientation {
        patch.push(("hw.initialOrientation".to_string(), orientation.clone()));
    }
    if let Some(keyboard) = fields.keyboard {
        patch.push((
            "hw.keyboard".to_string(),
            if keyboard { "yes" } else { "no" }.to_string(),
        ));
    }
    match fields.sdcard.as_deref() {
        Some("none") => patch.push(("hw.sdCard".to_string(), "no".to_string())),
        Some(size) => {
            patch.push(("hw.sdCard".to_string(), "yes".to_string()));
            patch.push(("sdcard.size".to_string(), size.to_string()));
        }
        None => {}
    }
    patch
}

/// Keys the patch actually changes, in patch order.
pub fn changed_keys(
    current: &std::collections::BTreeMap<String, String>,
    patch: &[(String, String)],
) -> Vec<String> {
    patch
        .iter()
        .filter(|(key, value)| current.get(key) != Some(value))
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unknown_keys_and_comments() {
        let src = "# hand written\nAvdId=E2\nweird.key=1\nhw.ramSize=2048\n";
        let out = apply_patch(src, &[("hw.ramSize".into(), "4096".into())]);
        assert!(out.contains("# hand written"));
        assert!(out.contains("weird.key=1"));
        assert!(out.contains("hw.ramSize=4096"));
        assert_eq!(out.matches("hw.ramSize").count(), 1);
    }

    #[test]
    fn appends_keys_the_file_does_not_have() {
        let out = apply_patch("AvdId=ZZ_Test\n", &[("hw.keyboard".into(), "yes".into())]);
        assert_eq!(out, "AvdId=ZZ_Test\nhw.keyboard=yes\n");
    }

    #[test]
    fn leaves_untouched_lines_byte_identical() {
        let src = "# keep  spaces\r\ndisk.dataPartition.size=8G\r\n";
        let out = apply_patch(src, &[("hw.ramSize".into(), "2048".into())]);
        assert!(out.starts_with("# keep  spaces\r\n"));
        assert!(out.contains("disk.dataPartition.size=8G\r\n"));
        assert!(out.ends_with("hw.ramSize=2048\n"));
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn current() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("hw.ramSize".to_string(), "2048".to_string()),
            ("hw.keyboard".to_string(), "no".to_string()),
        ])
    }

    #[test]
    fn changed_keys_reports_only_different_values() {
        let patch = vec![
            ("hw.ramSize".to_string(), "4096".to_string()),
            ("hw.keyboard".to_string(), "no".to_string()),
        ];
        assert_eq!(changed_keys(&current(), &patch), vec!["hw.ramSize"]);
    }

    #[test]
    fn edit_patch_maps_form_fields_to_config_keys() {
        let patch = edit_patch(&EditFields {
            ram_mb: Some(4096),
            storage: None,
            camera: Some("emulated".into()),
            orientation: None,
            keyboard: Some(true),
            sdcard: None,
        });
        assert!(patch.contains(&("hw.ramSize".to_string(), "4096".to_string())));
        assert!(patch.contains(&("hw.camera.back".to_string(), "emulated".to_string())));
        assert!(patch.contains(&("hw.keyboard".to_string(), "yes".to_string())));
        assert_eq!(patch.len(), 3);
    }

    #[test]
    fn sdcard_none_turns_the_card_off() {
        let patch = edit_patch(&EditFields {
            ram_mb: None,
            storage: None,
            camera: None,
            orientation: None,
            keyboard: None,
            sdcard: Some("none".into()),
        });
        assert_eq!(patch, vec![("hw.sdCard".to_string(), "no".to_string())]);
    }
}

/// What `avd show` returns: every config key, plus the fields the Edit
/// sheet may change.
#[derive(Debug, serde::Serialize)]
pub struct ShowResult {
    pub name: String,
    pub path: String,
    pub config: std::collections::BTreeMap<String, String>,
    pub editable: Editable,
}

#[derive(Debug, serde::Serialize)]
pub struct Editable {
    pub ram: Option<u32>,
    pub storage: Option<String>,
    pub camera: Option<String>,
    pub orientation: Option<String>,
    pub keyboard: Option<bool>,
    pub sdcard: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct UpdateResult {
    pub name: String,
    pub changed: Vec<String>,
}

fn editable_from(config: &std::collections::BTreeMap<String, String>) -> Editable {
    let get = |key: &str| config.get(key).cloned();
    Editable {
        ram: get("hw.ramSize").and_then(|value| value.parse().ok()),
        storage: get("disk.dataPartition.size"),
        camera: get("hw.camera.back"),
        orientation: get("hw.initialOrientation"),
        keyboard: get("hw.keyboard").map(|value| value == "yes"),
        sdcard: match get("hw.sdCard").as_deref() {
            Some("no") => Some("none".to_string()),
            _ => get("sdcard.size"),
        },
    }
}

pub fn show(name: &str) -> Result<ShowResult, super::json::CliError> {
    let dir = super::create::avd_dir_for(name);
    let config_path = dir.join("config.ini");
    let config = read_config(&config_path).map_err(|_| {
        super::json::CliError::new(
            "device_not_found",
            &format!("No AVD named '{name}'"),
            Some("Refresh the device list"),
        )
    })?;
    Ok(ShowResult {
        name: name.to_string(),
        path: dir.display().to_string(),
        editable: editable_from(&config),
        config,
    })
}

/// Rewrites `config.ini` in place; refuses while the AVD is running.
pub async fn update(name: &str, fields: EditFields) -> Result<UpdateResult, super::json::CliError> {
    super::create::validate_name(name)?;
    let manager = crate::managers::AndroidManager::new().map_err(|e| {
        super::json::CliError::new("sdk_not_found", &e.to_string(), Some("Set ANDROID_HOME"))
    })?;

    let dir = super::create::avd_dir_for(name);
    let config_path = dir.join("config.ini");
    let original = std::fs::read_to_string(&config_path).map_err(|_| {
        super::json::CliError::new(
            "device_not_found",
            &format!("No AVD named '{name}'"),
            Some("Refresh the device list"),
        )
    })?;

    let running = manager.get_running_avd_names().await.unwrap_or_default();
    if running.contains_key(name) {
        return Err(super::json::CliError::new(
            "device_running",
            &format!("'{name}' is running"),
            Some("Stop the AVD before changing its configuration"),
        ));
    }

    let current = read_config(&config_path).unwrap_or_default();
    let patch = edit_patch(&fields);
    let changed = changed_keys(&current, &patch);
    if changed.is_empty() {
        return Ok(UpdateResult {
            name: name.to_string(),
            changed,
        });
    }
    write_atomic(&config_path, &apply_patch(&original, &patch)).map_err(|e| {
        super::json::CliError::new(
            "tool_failed",
            &format!("Could not write config.ini: {e}"),
            None,
        )
    })?;
    Ok(UpdateResult {
        name: name.to_string(),
        changed,
    })
}
