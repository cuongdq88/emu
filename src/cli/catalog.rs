//! Read-only catalogs the Create Device form needs: Android hardware
//! profiles, tags/ABIs of a system image, iOS device types and runtimes,
//! and the SDK license state.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Screen size from the device's skin; 0 when the SDK has no skin for it.
    pub width: u32,
    pub height: u32,
    /// DPI of the skin; 0 when unknown.
    pub density: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TagRow {
    pub tag: String,
    pub abi: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IosDeviceTypeRow {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IosRuntimeRow {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LicenseStatus {
    pub accepted: bool,
    pub missing: Vec<String>,
}

/// License files `sdkmanager --licenses` writes, by the name the app shows.
const LICENSE_FILES: &[&str] = &[
    "android-sdk-license",
    "android-sdk-preview-license",
    "android-googletv-license",
    "android-sdk-arm-dbt-license",
    "mips-android-sysimage-license",
];

/// Picks the `display { width N height N }` block out of a skin's `layout`
/// file (Android's own skin DSL, not JSON).
pub fn parse_skin_layout(text: &str) -> Option<(u32, u32)> {
    let mut in_device = false;
    let mut in_display = false;
    let mut width = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("device") {
            in_device = true;
            continue;
        }
        if in_device && line.starts_with("display") {
            in_display = true;
            continue;
        }
        // `layouts {` ends the parts section; stop before portrait sizes.
        if line.starts_with("layouts") {
            break;
        }
        if !in_display {
            continue;
        }
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("width") => width = parts.next().and_then(|v| v.parse().ok()),
            Some("height") => {
                if let (Some(width), Some(height)) =
                    (width, parts.next().and_then(|v| v.parse().ok()))
                {
                    return Some((width, height));
                }
            }
            Some("}") => in_display = false,
            _ => {}
        }
    }
    None
}

pub fn sort_profiles(profiles: &mut [Profile]) {
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
}

/// "iOS 26.0" → "26.0"; runtimes without a number get "".
pub fn runtime_version(name: &str) -> String {
    let version: String = name
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    version.trim_end_matches('.').to_string()
}

pub fn license_state_from(files: &[(&str, bool)]) -> LicenseStatus {
    let missing: Vec<String> = files
        .iter()
        .filter(|(_, accepted)| !accepted)
        .map(|(name, _)| name.to_string())
        .collect();
    LicenseStatus {
        accepted: missing.is_empty(),
        missing,
    }
}

/// Reads the SDK's `licenses/` directory.
pub fn license_state(sdk_path: &std::path::Path) -> LicenseStatus {
    let files: Vec<(&str, bool)> = LICENSE_FILES
        .iter()
        .map(|name| {
            let accepted = std::fs::read_to_string(sdk_path.join("licenses").join(name))
                .map(|body| body.lines().any(|line| !line.trim().is_empty()))
                .unwrap_or(false);
            (*name, accepted)
        })
        .collect();
    license_state_from(&files)
}

/// Hardware profiles with the screen size their skin declares.
pub async fn profiles() -> anyhow::Result<Vec<Profile>> {
    let manager = crate::managers::AndroidManager::new()?;
    let devices = manager.list_available_devices().await?;
    let skins = manager.android_home().join("skins");
    let mut rows: Vec<Profile> = devices
        .into_iter()
        .map(|(id, name)| {
            let (width, height) = std::fs::read_to_string(skins.join(&id).join("layout"))
                .ok()
                .and_then(|layout| parse_skin_layout(&layout))
                .unwrap_or((0, 0));
            Profile {
                id,
                name,
                width,
                height,
                density: 0,
            }
        })
        .collect();
    sort_profiles(&mut rows);
    Ok(rows)
}

/// Tags and ABIs the SDK knows for API `api`, installed or not.
pub async fn tags(api: u32) -> anyhow::Result<Vec<TagRow>> {
    let manager = crate::managers::AndroidManager::new()?;
    let levels = manager.list_api_levels().await?;
    let level = levels
        .into_iter()
        .find(|level| level.api == api)
        .ok_or_else(|| anyhow::anyhow!("No system image information for API {api}"))?;
    Ok(level
        .variants
        .into_iter()
        .map(|variant| TagRow {
            tag: variant.variant,
            abi: variant.architecture,
            installed: variant.is_installed,
        })
        .collect())
}

pub async fn ios_device_types() -> anyhow::Result<Vec<IosDeviceTypeRow>> {
    let manager = crate::managers::IosManager::new()?;
    let types = manager.list_device_types_with_names().await?;
    Ok(types
        .into_iter()
        .map(|(id, name)| IosDeviceTypeRow { id, name })
        .collect())
}

pub async fn ios_runtimes() -> anyhow::Result<Vec<IosRuntimeRow>> {
    let manager = crate::managers::IosManager::new()?;
    let runtimes = manager.list_runtimes().await?;
    Ok(runtimes
        .into_iter()
        .map(|(id, name)| IosRuntimeRow {
            version: runtime_version(&name),
            id,
            name,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKIN_LAYOUT: &str = "\
parts {
  device {
    display {
      width 1080
      height 1920
      x 0
      y 0
    }
  }
}
layouts {
  portrait {
    width 1370
    height 2405
  }
}
";

    #[test]
    fn parses_device_display_from_skin_layout() {
        assert_eq!(parse_skin_layout(SKIN_LAYOUT), Some((1080, 1920)));
        assert_eq!(
            parse_skin_layout("layouts { portrait { width 5 height 5 } }"),
            None
        );
    }

    #[test]
    fn profiles_are_sorted_by_name() {
        let mut rows = vec![
            Profile {
                id: "zed".into(),
                name: "Zed".into(),
                width: 1,
                height: 1,
                density: 0,
            },
            Profile {
                id: "alpha".into(),
                name: "Alpha".into(),
                width: 1,
                height: 1,
                density: 0,
            },
        ];
        sort_profiles(&mut rows);
        assert!(rows.windows(2).all(|w| w[0].name <= w[1].name));
        assert_eq!(rows[0].id, "alpha");
    }

    #[test]
    fn runtime_version_is_the_numeric_part() {
        assert_eq!(runtime_version("iOS 26.0"), "26.0");
        assert_eq!(runtime_version("watchOS 11.2"), "11.2");
        assert_eq!(runtime_version("no digits"), "");
    }

    #[test]
    fn license_status_lists_missing_files() {
        let state = license_state_from(&[
            ("android-sdk-license", true),
            ("android-sdk-preview-license", false),
        ]);
        assert!(!state.accepted);
        assert_eq!(
            state.missing,
            vec!["android-sdk-preview-license".to_string()]
        );
    }
}
