//! The single JSON envelope every `--json` subcommand prints.
//!
//! `{"ok":bool,"data":object|null,"error":{"code","message","hint"}|null}`,
//! one line on stdout, exit 0 on success, 1 on a business error and 2 when
//! the environment (SDK, Xcode) is the problem.

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
pub struct ErrBody {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Response<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<ErrBody>,
}

/// Error codes that mean "this machine cannot do it", not "the request is
/// wrong".
const ENVIRONMENT_CODES: &[&str] = &[
    "sdk_not_found",
    "tool_missing",
    "xcode_missing",
    "simctl_missing",
];

pub fn exit_code_for(code: &str) -> u8 {
    if ENVIRONMENT_CODES.contains(&code) {
        2
    } else {
        1
    }
}

pub fn ok_value<T: Serialize>(data: T) -> Value {
    json!({ "ok": true, "data": data, "error": Value::Null })
}

pub fn err_value(code: &str, message: &str, hint: Option<&str>) -> Value {
    json!({
        "ok": false,
        "data": Value::Null,
        "error": { "code": code, "message": message, "hint": hint },
    })
}

pub fn print_value(value: &Value) {
    println!("{value}");
}

pub fn emit_ok<T: Serialize>(data: T) -> u8 {
    print_value(&ok_value(data));
    0
}

pub fn emit_err(code: &str, message: &str, hint: Option<&str>) -> u8 {
    print_value(&err_value(code, message, hint));
    exit_code_for(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_has_code_message_hint() {
        let v = serde_json::to_value(err_value("image_missing", "no image", Some("run install")))
            .unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["data"], serde_json::Value::Null);
        assert_eq!(v["error"]["code"], "image_missing");
        assert_eq!(v["error"]["message"], "no image");
        assert_eq!(v["error"]["hint"], "run install");
    }

    #[test]
    fn ok_envelope_has_null_error() {
        let v = ok_value(serde_json::json!({"a": 1}));
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["a"], 1);
        assert_eq!(v["error"], serde_json::Value::Null);
    }

    #[test]
    fn environment_codes_exit_two_and_business_codes_exit_one() {
        assert_eq!(exit_code_for("sdk_not_found"), 2);
        assert_eq!(exit_code_for("license_required"), 1);
        assert_eq!(exit_code_for("tool_failed"), 1);
    }
}
