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
