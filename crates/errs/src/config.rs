//! Persist only the configuration marker, not inferred effects, in incremental build information.
//! This lets an ordinary compiler run invalidate diagnostics left by an `ErrorScript` run.

use tsrs_core::CompilerOptions;
use tsrs_core::json::{JsonMap, Value};

pub fn write_build_info(options: &CompilerOptions, entries: &mut Option<JsonMap>) {
    let mut error_options = JsonMap::new();
    if let Some(enabled) = options.error_options.no_unhandled_root_errors {
        error_options.insert("noUnhandledRootErrors".to_string(), Value::Bool(enabled));
    }
    if let Some(enabled) = options.error_options.no_implicit_returns {
        error_options.insert("noImplicitReturns".to_string(), Value::Bool(enabled));
    }
    // Explicit Returns contracts are checked even when both optional diagnostics are disabled.
    if crate::enabled() {
        error_options.insert("returnsContracts".to_string(), Value::Bool(true));
    }
    if !error_options.is_empty() {
        entries
            .get_or_insert_with(JsonMap::new)
            .insert("errorOptions".to_string(), Value::Object(error_options));
    }
}

pub fn read_build_info(options: &mut CompilerOptions, name: &str, value: &Value) -> bool {
    if name != "errorOptions" {
        return false;
    }
    if let Value::Object(fields) = value
        && let Some(Value::Bool(enabled)) = fields.get("noUnhandledRootErrors")
    {
        options.error_options.no_unhandled_root_errors = Some(*enabled);
    }
    if let Value::Object(fields) = value {
        if let Some(Value::Bool(enabled)) = fields.get("noImplicitReturns") {
            options.error_options.no_implicit_returns = Some(*enabled);
        }
        // A private marker ensures switching back to plain TS invalidates contract diagnostics.
        if matches!(fields.get("returnsContracts"), Some(Value::Bool(true))) {
            options.error_options.returns_contracts = true;
        }
    }
    true
}
