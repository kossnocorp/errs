//! `ErrorScript`'s effect domain and inference pass. Value types and resolution belong to the host checker.
//! All native identities in a `Inference` belong to that checker and its program snapshot.

pub mod config;
mod infer;
mod lower;
pub use errs_config::ErrorOptions;
mod roots;
pub use infer::Inference;
pub use roots::RootError;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use tsrs_ast::{Node, SourceFile};
use tsrs_core::P;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Select `ErrorScript` before constructing any programs (normally at process startup).
pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Keep the fork's flag outside the generated upstream option tables, including the LSP's flags.
pub fn take_cli_flag(args: &mut Vec<String>) {
    args.retain(|arg| {
        if arg == "--errorscript" {
            enable();
            false
        } else {
            true
        }
    });
}

/// A finite union of native types, plus uncertainty independent of that union. Unknown must not erase known errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorSet<T> {
    pub types: Vec<T>,
    pub unknown: bool,
    /// Unknown type from an actual throw/rejection, distinct from an unknown implementation.
    pub explicit_unknown: bool,
}

impl<T> Default for ErrorSet<T> {
    fn default() -> Self {
        Self { types: Vec::new(), unknown: false, explicit_unknown: false }
    }
}

impl<T: Copy + Eq> ErrorSet<T> {
    pub fn unknown() -> Self {
        Self { unknown: true, ..Self::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty() && !self.unknown
    }

    pub fn insert(&mut self, ty: T) {
        if !self.types.contains(&ty) {
            // Recursive generic instantiation can generate an unbounded family of types.
            if self.types.len() < 32 {
                self.types.push(ty);
            } else {
                self.unknown = true;
            }
        }
    }

    pub fn extend(&mut self, other: &Self) {
        self.unknown |= other.unknown;
        self.explicit_unknown |= other.explicit_unknown;
        for &ty in &other.types {
            self.insert(ty);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectChannel {
    All,
    Throws,
    Rejects,
}

/// Function/signature and call metadata. Rejections are distinct from synchronous exceptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect<T> {
    pub throws: ErrorSet<T>,
    pub rejects: ErrorSet<T>,
    /// Generator execution is deferred until iteration. Iterator protocol modelling is not yet available.
    pub deferred: ErrorSet<T>,
}

impl<T> Default for Effect<T> {
    fn default() -> Self {
        Self {
            throws: ErrorSet::default(),
            rejects: ErrorSet::default(),
            deferred: ErrorSet::default(),
        }
    }
}

/// Narrow semantic boundary: no rendered type is ever used as an identity or parsed back into a type.
pub trait Host {
    type Type: Copy + Eq;

    fn files(&self) -> &'static [P<SourceFile>];
    fn type_of(&mut self, node: P<Node>) -> ErrorSet<Self::Type>;
    fn call_target(&mut self, call: P<Node>) -> Option<P<Node>>;
    fn declaration_target(&mut self, declaration: P<Node>) -> P<Node>;
    fn intrinsic_call(&mut self, call: P<Node>) -> Option<Effect<Self::Type>>;
    fn instantiate(&mut self, ty: Self::Type, call: P<Node>) -> ErrorSet<Self::Type>;
    fn catch_binding(&mut self, expression: P<Node>) -> Option<P<Node>>;
    fn initializer(&mut self, expression: P<Node>) -> Option<P<Node>>;
    fn is_promise(&mut self, expression: P<Node>) -> bool;
    fn signature_returns_promise(&mut self, declaration: P<Node>) -> bool;
    fn has_accessor(&mut self, expression: P<Node>) -> bool;
    fn render(&mut self, ty: Self::Type) -> String;
    fn promise_operation(&mut self, call: P<Node>) -> Option<PromiseOperation>;
    fn callback_target(&mut self, expression: P<Node>) -> Option<P<Node>>;
    fn callback_parameter(&mut self, expression: P<Node>) -> Option<P<Node>>;
    fn parameter_effect(&mut self, parameter: P<Node>, channel: EffectChannel) -> Self::Type;
    fn effect_parameter(&self, ty: Self::Type) -> Option<(P<Node>, EffectChannel)>;
}

#[derive(Clone, Copy)]
pub enum PromiseKind {
    Then,
    Catch,
    Finally,
    Resolve,
    Reject,
}

#[derive(Clone, Copy)]
pub struct PromiseOperation {
    pub kind: PromiseKind,
    pub receiver: P<Node>,
}

pub fn render_effect<H: Host>(host: &mut H, effect: &Effect<H::Type>) -> String {
    fn render_set<H: Host>(host: &mut H, set: &ErrorSet<H::Type>) -> String {
        let mut names: Vec<_> = set.types.iter().map(|&ty| host.render(ty)).collect();
        names.sort();
        names.dedup();
        if set.unknown {
            names.push("unknown".to_string());
        }
        if names.is_empty() { "never".to_string() } else { names.join(" | ") }
    }
    let mut text = format!("throws: {}", render_set(host, &effect.throws));
    if !effect.rejects.is_empty() {
        write!(text, "\nrejects: {}", render_set(host, &effect.rejects)).unwrap();
    }
    if !effect.deferred.is_empty() {
        write!(text, "\non iteration: {}", render_set(host, &effect.deferred)).unwrap();
    }
    text
}
