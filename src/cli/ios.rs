//! `emu --json ios create`: `xcrun simctl create`, with the errors the
//! Create sheet knows how to show.

use super::json::CliError;
use crate::managers::common::DeviceManager;
use crate::managers::IosManager;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CreatedSimulator {
    pub udid: String,
    pub name: String,
}

/// simctl answers with one line we have to classify, since it has no error
/// codes of its own.
pub fn map_create_error(message: &str) -> CliError {
    let lower = message.to_lowercase();
    if lower.contains("invalid runtime") || lower.contains("runtime") {
        CliError::new(
            "runtime_missing",
            message,
            Some("Open Xcode › Settings › Platforms and download this runtime"),
        )
    } else if lower.contains("already exists") || lower.contains("duplicate") {
        CliError::new("name_exists", message, Some("Pick another name"))
    } else if lower.contains("invalid device type") || lower.contains("device type") {
        CliError::new(
            "device_not_found",
            message,
            Some("Refresh the device type list"),
        )
    } else {
        CliError::new("tool_failed", message, None)
    }
}

/// The manager renders simulator names as "ZZ_iOS (iOS 26.5)", so compare
/// the name the user typed against that prefix.
pub fn same_device_name(existing: &str, wanted: &str) -> bool {
    existing == wanted
        || existing
            .strip_prefix(wanted)
            .is_some_and(|rest| rest.starts_with(" ("))
}

pub async fn create(
    device_type: &str,
    runtime: &str,
    name: &str,
) -> Result<CreatedSimulator, CliError> {
    super::create::validate_name(name)?;
    let manager = IosManager::new()
        .map_err(|e| CliError::new("simctl_missing", &e.to_string(), Some("Install Xcode")))?;

    let runtimes = manager
        .list_runtimes()
        .await
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
    if !runtimes.iter().any(|(id, _)| id == runtime) {
        return Err(CliError::new(
            "runtime_missing",
            &format!("Simulator runtime '{runtime}' is not installed"),
            Some("Open Xcode › Settings › Platforms and download this runtime"),
        ));
    }

    let devices = manager
        .list_devices()
        .await
        .map_err(|e| CliError::new("tool_failed", &e.to_string(), None))?;
    if devices
        .iter()
        .any(|device| same_device_name(&device.name, name))
    {
        return Err(CliError::new(
            "name_exists",
            &format!("A simulator named '{name}' already exists"),
            Some("Pick another name, or delete the existing simulator first"),
        ));
    }

    let udid = manager
        .create_simulator(name, device_type, runtime)
        .await
        .map_err(|e| map_create_error(&e.to_string()))?;

    Ok(CreatedSimulator {
        udid,
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_runtime_is_reported_with_hint() {
        let e = map_create_error("Invalid runtime");
        assert_eq!(e.code, "runtime_missing");
        assert!(e.hint.unwrap().contains("Xcode"));
    }

    #[test]
    fn device_names_match_with_the_runtime_suffix() {
        assert!(same_device_name("ZZ_iOS (iOS 26.5)", "ZZ_iOS"));
        assert!(same_device_name("ZZ_iOS", "ZZ_iOS"));
        assert!(!same_device_name("ZZ_iOS2 (iOS 26.5)", "ZZ_iOS"));
    }

    #[test]
    fn duplicate_name_is_name_exists() {
        assert_eq!(
            map_create_error("Unable to create a device: Invalid device: name already exists").code,
            "name_exists"
        );
    }
}
