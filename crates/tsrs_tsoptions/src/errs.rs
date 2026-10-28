//! Parse the fork's top-level tsconfig section without changing TypeScript's generated option tables.

use crate::CompilerOptionsValue;
use ::errs::ErrorOptions;
use tsrs_ast::{Diagnostic, new_compiler_diagnostic};
use tsrs_core::P;
use tsrs_diagnostics as diagnostics;

pub fn parse(raw: &CompilerOptionsValue) -> (ErrorOptions, Vec<P<Diagnostic>>) {
    let mut options = ErrorOptions::default();
    let mut errors = Vec::new();
    let Some(value) = raw.as_object().and_then(|raw| raw.get("errorOptions")) else {
        return (options, errors);
    };
    let Some(object) = value.as_object() else {
        errors.push(new_compiler_diagnostic(
            &diagnostics::COMPILER_OPTION_0_REQUIRES_A_VALUE_OF_TYPE_1,
            &[&"errorOptions", &"object"],
        ));
        return (options, errors);
    };
    for (name, value) in object {
        let option = match name.as_str() {
            "noUnhandledRootErrors" => Some(&mut options.no_unhandled_root_errors),
            "noImplicitReturns" => Some(&mut options.no_implicit_returns),
            _ => None,
        };
        if let Some(option) = option {
            if let CompilerOptionsValue::Bool(value) = value {
                *option = Some(*value);
            } else {
                errors.push(new_compiler_diagnostic(
                    &diagnostics::COMPILER_OPTION_0_REQUIRES_A_VALUE_OF_TYPE_1,
                    &[&format!("errorOptions.{name}"), &"boolean"],
                ));
            }
        } else {
            errors.push(new_compiler_diagnostic(
                &diagnostics::UNKNOWN_COMPILER_OPTION_0,
                &[&format!("errorOptions.{name}")],
            ));
        }
    }
    (options, errors)
}
