//! Dependency-free `ErrorScript` configuration carried by the native compiler options.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ErrorOptions {
    pub no_unhandled_root_errors: Option<bool>,
    pub no_implicit_returns: Option<bool>,
    /// Internal incremental-cache marker; not a user-facing configuration property.
    pub returns_contracts: bool,
}

impl ErrorOptions {
    pub fn merge(&mut self, other: Self) {
        if other.no_unhandled_root_errors.is_some() {
            self.no_unhandled_root_errors = other.no_unhandled_root_errors;
        }
        if other.no_implicit_returns.is_some() {
            self.no_implicit_returns = other.no_implicit_returns;
        }
    }
}
