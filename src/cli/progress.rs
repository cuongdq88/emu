//! NDJSON progress for long-running SDK work.
//!
//! Every line but the last is `{"event":"progress","percent":N,"message":"…"}`;
//! the last line is the usual JSON envelope.

use serde_json::{json, Value};
use std::io::Write;

/// `[=====    ] 45% Computing updates...` → 45. Lines without a percent
/// (licenses, warnings) return None.
pub fn parse_sdkmanager_percent(line: &str) -> Option<u8> {
    let percent_at = line.find('%')?;
    let digits: String = line[..percent_at]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

pub fn emit_progress(percent: u8, message: &str) {
    println!("{}", json!({"event": "progress", "percent": percent, "message": message}));
    let _ = std::io::stdout().flush();
}

/// The final line of a streamed command.
pub fn emit_done(data: Value) {
    println!("{}", json!({"event": "done", "ok": true, "data": data, "error": Value::Null}));
    let _ = std::io::stdout().flush();
}

pub fn emit_stream_error(code: &str, message: &str, hint: Option<&str>) {
    println!(
        "{}",
        json!({
            "event": "done",
            "ok": false,
            "data": Value::Null,
            "error": {"code": code, "message": message, "hint": hint},
        })
    );
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_percent() {
        assert_eq!(
            parse_sdkmanager_percent("[=====    ] 45% Computing updates..."),
            Some(45)
        );
        assert_eq!(parse_sdkmanager_percent("just a line"), None);
    }

    #[test]
    fn parses_three_digit_and_zero_percent() {
        assert_eq!(parse_sdkmanager_percent("[         ]   0% Fetch remote"), Some(0));
        assert_eq!(parse_sdkmanager_percent("[=========] 100% Unzipping"), Some(100));
    }

    #[test]
    fn ignores_percent_that_is_not_the_bar_value() {
        assert_eq!(parse_sdkmanager_percent("Warning: 5% of disk used"), Some(5));
        assert_eq!(parse_sdkmanager_percent("100%"), Some(100));
    }
}
