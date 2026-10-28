//! `ErrorScript`'s native checker adapter. Effect semantics live in `errs`, not in the ported checker algorithms.

use crate::*;
use ::errs::{Effect, ErrorSet, Host, Inference};
use tsrs_core::TextRange;

pub mod catch;
mod returns;
pub mod throws;

impl Checker {
    pub fn unhandled_root_error_diagnostics(&mut self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        if self.compiler_options.error_options.no_unhandled_root_errors != Some(true)
            || self.compiler_options.no_check.is_true()
            || file.is_declaration_file()
        {
            return Vec::new();
        }
        self.infer_error_effects();
        let inference = self.errs_inference.take().unwrap();
        let errors = inference.root_errors(self, file);
        self.errs_inference = Some(inference);
        errors
            .into_iter()
            .map(|error| {
                let mut names: Vec<_> =
                    error.errors.types.iter().map(|&ty| self.type_to_string_exported(ty)).collect();
                names.sort();
                names.dedup();
                if error.errors.unknown {
                    names.push("unknown".to_string());
                }
                let channel = if error.rejection { "promise rejection" } else { "error" };
                let remedy = if error.rejection {
                    "Handle it with a rejection handler or await it inside try/catch."
                } else {
                    "Handle it with try/catch."
                };
                let text =
                    format!("Unhandled root-level {channel}: {}. {remedy}", names.join(" | "));
                let range = TextRange::new(
                    tsrs_scanner::get_token_pos_of_node(error.node, file, false),
                    error.node.end().expect_source("root error"),
                );
                ast::new_diagnostic_from_text(
                    Some(file),
                    range,
                    98001,
                    diagnostics::Category::Error,
                    &text,
                    &[],
                    &[],
                    false,
                    false,
                )
            })
            .collect()
    }

    /// Populate all function and call attachments for this checker/program snapshot, including JS bodies.
    pub fn infer_error_effects(&mut self) {
        if self.errs_inference.is_none() {
            self.errs_inference = Some(Inference::infer(self));
        }
    }

    /// Native type handles in the returned effect have the same lifetime/ownership contract as other checker types.
    pub fn error_effect_of_node(&mut self, node: P<Node>) -> Option<Effect<P<Type>>> {
        self.infer_error_effects();
        let inference = self.errs_inference.take().unwrap();
        let result = inference.at(self, node);
        self.errs_inference = Some(inference);
        result
    }

    /// Signature attachments are backed by the declaration table, then instantiated with the native signature mapper.
    pub fn error_effect_of_signature(&mut self, signature: P<Signature>) -> Effect<P<Type>> {
        let mut effect = signature
            .declaration()
            .and_then(|n| self.error_effect_of_node(n))
            .unwrap_or_else(|| Effect { throws: ErrorSet::unknown(), ..Effect::default() });
        for set in [&mut effect.throws, &mut effect.rejects, &mut effect.deferred] {
            let mut mapped = ErrorSet {
                unknown: set.unknown,
                explicit_unknown: set.explicit_unknown,
                ..ErrorSet::default()
            };
            for &ty in &set.types {
                let ty = self.instantiate_type(ty, signature.mapper.get());
                mapped.extend(&Self::errs_type_set(ty));
            }
            *set = mapped;
        }
        effect
    }

    /// Used by hover. Only attach to callable names/signatures/calls, not arbitrary tokens inside their bodies.
    pub fn error_effect_at(&mut self, mut node: P<Node>) -> Option<Effect<P<Type>>> {
        if let Some(parent) = node.parent() {
            if ast::is_access_expression(parent) && parent.name() == Some(node) {
                node = parent;
            }
            if let Some(parent) = node.parent() {
                let is_callee = matches!(
                    parent.kind(),
                    Kind::CallExpression | Kind::NewExpression | Kind::TaggedTemplateExpression
                ) && (if parent.kind() == Kind::TaggedTemplateExpression {
                    parent.as_tagged_template_expression().tag == node
                } else {
                    parent.expression() == Some(node)
                });
                if is_callee || ast::is_function_like(parent) && parent.name() == Some(node) {
                    node = parent;
                }
            }
        }
        let effect = if let Some(effect) = self.error_effect_of_node(node) {
            effect
        } else {
            let ty = self.get_type_at_location(node);
            let signatures = self.get_signatures_of_type(ty, SignatureKind::Call);
            if signatures.is_empty() {
                return None;
            }
            let mut combined = Effect::default();
            for &signature in signatures {
                let effect = self.error_effect_of_signature(signature);
                combined.throws.extend(&effect.throws);
                combined.rejects.extend(&effect.rejects);
                combined.deferred.extend(&effect.deferred);
            }
            combined
        };
        Some(effect)
    }

    pub fn error_effect_display_at(&mut self, node: P<Node>) -> Option<String> {
        let effect = self.error_effect_at(node)?;
        Some(::errs::render_effect(self, &effect))
    }

    fn errs_type_set(ty: P<Type>) -> ErrorSet<P<Type>> {
        let mut set = ErrorSet::default();
        if ty.flags().intersects(TypeFlags::Any | TypeFlags::Unknown) {
            set.unknown = true;
        } else if !ty.flags().intersects(TypeFlags::Never) {
            set.insert(ty);
        }
        set
    }

    fn errs_callback_expression(&mut self, mut expression: P<Node>) -> P<Node> {
        // An annotation on a const callback must not hide its implementation or parameter identity.
        // Bound alias traversal so malformed/cyclic initializers retain their native uncertainty.
        for _ in 0..32 {
            expression = ast::skip_outer_expressions(expression, ast::OuterExpressionKinds::All);
            if expression.kind() != Kind::Identifier {
                break;
            }
            let Some(declaration) = self
                .get_symbol_at_location_exported(expression)
                .and_then(|symbol| symbol.value_declaration())
            else {
                break;
            };
            if declaration.kind() == Kind::VariableDeclaration
                && declaration
                    .parent()
                    .is_some_and(|node| node.flags().intersects(NodeFlags::Const))
                && let Some(initializer) = declaration.initializer()
            {
                expression = initializer;
            } else {
                break;
            }
        }
        expression
    }
}

impl Host for Checker {
    type Type = P<Type>;

    fn files(&self) -> &'static [P<SourceFile>] {
        self.files
    }

    fn type_of(&mut self, node: P<Node>) -> ErrorSet<P<Type>> {
        let ty = self.get_type_at_location(node);
        Self::errs_type_set(ty)
    }

    fn declaration_target(&mut self, declaration: P<Node>) -> P<Node> {
        if declaration.body().is_some() {
            return declaration;
        }
        // Overload signatures and their implementation share a native bound symbol.
        self.get_symbol_of_declaration(declaration)
            .and_then(|symbol| {
                symbol
                    .declarations()
                    .iter()
                    .copied()
                    .find(|n| ast::is_function_like(*n) && n.body().is_some())
            })
            .unwrap_or(declaration)
    }

    fn is_non_throwing_declaration(&mut self, declaration: P<Node>) -> bool {
        // Trust an explicit never contract only when there is no implementation to infer/check.
        // Never contracts cover every channel, so no sync/rejection/iteration split is needed.
        self.declaration_target(declaration).body().is_none()
            && self
                .errs_function_contract(declaration)
                .is_some_and(|errors| errors.flags().intersects(TypeFlags::Never))
    }

    fn call_target(&mut self, call: P<Node>) -> Option<P<Node>> {
        let signature = self.get_resolved_signature_exported(call);
        signature.declaration().map(|n| self.declaration_target(n))
    }

    fn intrinsic_call(&mut self, call: P<Node>) -> Option<Effect<P<Type>>> {
        if call.kind() == Kind::TaggedTemplateExpression {
            return None;
        }
        let signature = self.get_resolved_signature_exported(call);
        let declaration = signature.declaration()?;
        let file = ast::get_source_file_of_node(declaration)?;
        if !self.program.is_source_file_default_library(file.path()) {
            return None;
        }
        let parent = declaration.parent()?;
        let name = parent.name()?.text();
        // A deliberately small trusted host model. Match native lib declarations, never the call's spelling.
        // Message coercion can invoke user code; only absent or primitive arguments are modelled as pure.
        if matches!(
            name,
            "ErrorConstructor"
                | "EvalErrorConstructor"
                | "RangeErrorConstructor"
                | "ReferenceErrorConstructor"
                | "SyntaxErrorConstructor"
                | "TypeErrorConstructor"
                | "URIErrorConstructor"
        ) {
            let arguments = call.arguments();
            if arguments.iter().all(|&arg| {
                let ty = self.get_type_at_location(arg);
                ty.flags().intersects(
                    TypeFlags::StringLike
                        | TypeFlags::NumberLike
                        | TypeFlags::BooleanLike
                        | TypeFlags::Null
                        | TypeFlags::Undefined,
                )
            }) {
                return Some(Effect::default());
            }
        }
        None
    }

    fn instantiate(&mut self, ty: P<Type>, call: P<Node>) -> ErrorSet<P<Type>> {
        let signature = if matches!(
            call.kind(),
            Kind::CallExpression | Kind::NewExpression | Kind::TaggedTemplateExpression
        ) {
            self.get_resolved_signature_exported(call)
        } else {
            let callable = self.get_type_at_location(call);
            let Some(&signature) =
                self.get_signatures_of_type(callable, SignatureKind::Call).first()
            else {
                return Self::errs_type_set(ty);
            };
            signature
        };
        let ty = self.instantiate_type(ty, signature.mapper.get());
        Self::errs_type_set(ty)
    }

    fn catch_binding(&mut self, expression: P<Node>) -> Option<P<Node>> {
        // Native catch types now include control-flow narrowing and assignments at the throw site.
        // The metadata-only API in ordinary TS mode retains its incoming-error substitution.
        if ::errs::enabled() {
            return None;
        }
        if expression.kind() != Kind::Identifier {
            return None;
        }
        let declaration = self.get_symbol_at_location_exported(expression)?.value_declaration()?;
        (declaration.parent()?.kind() == Kind::CatchClause).then_some(declaration)
    }

    fn initializer(&mut self, expression: P<Node>) -> Option<P<Node>> {
        if !self.is_promise(expression) {
            return None;
        }
        let declaration = self.get_symbol_at_location_exported(expression)?.value_declaration()?;
        // Only immutable bindings; reassigned promises retain unknown rejection effects.
        if declaration.kind() == Kind::VariableDeclaration
            && declaration.parent().is_some_and(|n| n.flags().intersects(NodeFlags::Const))
        {
            declaration.initializer()
        } else {
            None
        }
    }

    fn is_promise(&mut self, expression: P<Node>) -> bool {
        let ty = self.get_type_at_location(expression);
        // A native type-checking error is not evidence of a runtime promise obligation.
        if ty == self.error_type {
            return false;
        }
        ty.flags().intersects(TypeFlags::Any | TypeFlags::Unknown)
            || self.get_promised_type_of_promise(ty).is_some()
    }

    fn signature_returns_promise(&mut self, declaration: P<Node>) -> bool {
        let signature = self.get_signature_from_declaration(declaration);
        let ty = self.get_return_type_of_signature(signature);
        ty.flags().intersects(TypeFlags::Any | TypeFlags::Unknown)
            || self.get_promised_type_of_promise(ty).is_some()
    }

    fn has_accessor(&mut self, expression: P<Node>) -> bool {
        self.get_symbol_at_location_exported(expression).is_some_and(|symbol| {
            symbol
                .declarations()
                .iter()
                .any(|node| matches!(node.kind(), Kind::GetAccessor | Kind::SetAccessor))
        })
    }

    fn render(&mut self, ty: P<Type>) -> String {
        self.type_to_string_exported(ty)
    }

    fn callback_target(&mut self, expression: P<Node>) -> Option<P<Node>> {
        let expression = self.errs_callback_expression(expression);
        if matches!(expression.kind(), Kind::ArrowFunction | Kind::FunctionExpression) {
            return Some(expression);
        }
        let ty = self.get_type_at_location(expression);
        let signature = *self.get_signatures_of_type(ty, SignatureKind::Call).first()?;
        Some(self.declaration_target(signature.declaration()?))
    }

    fn callback_parameter(&mut self, expression: P<Node>) -> Option<P<Node>> {
        let expression = self.errs_callback_expression(expression);
        if expression.kind() != Kind::Identifier {
            return None;
        }
        let symbol = self.get_symbol_at_location_exported(expression)?;
        let declaration = symbol.value_declaration()?;
        (declaration.kind() == Kind::Parameter && !self.is_symbol_assigned(symbol))
            .then_some(declaration)
    }

    fn parameter_effect(&mut self, parameter: P<Node>, channel: ::errs::EffectChannel) -> P<Type> {
        let operand = self.get_type_at_location(parameter.name().unwrap_or(parameter));
        self.errs_deferred_throws(operand, Some(parameter), channel)
    }

    fn effect_parameter(&self, ty: P<Type>) -> Option<(P<Node>, ::errs::EffectChannel)> {
        let effect = self.errs_throws.types.get(&ty)?;
        Some((effect.parameter?, effect.channel))
    }

    fn promise_operation(&mut self, call: P<Node>) -> Option<::errs::PromiseOperation> {
        use ::errs::PromiseKind;
        if call.kind() != Kind::CallExpression {
            return None;
        }
        let access = call.expression()?;
        if access.kind() != Kind::PropertyAccessExpression {
            return None;
        }
        let signature = self.get_resolved_signature_exported(call);
        let declaration = signature.declaration()?;
        let file = ast::get_source_file_of_node(declaration)?;
        if !self.program.is_source_file_default_library(file.path()) {
            return None;
        }
        let parent = declaration.parent()?.name()?.text();
        let name = declaration.name()?.text();
        let kind = match (parent, name) {
            ("Promise" | "PromiseLike", "then") => PromiseKind::Then,
            ("Promise", "catch") => PromiseKind::Catch,
            ("Promise", "finally") => PromiseKind::Finally,
            ("PromiseConstructor", "resolve") => PromiseKind::Resolve,
            ("PromiseConstructor", "reject") => PromiseKind::Reject,
            _ => return None,
        };
        Some(::errs::PromiseOperation { kind, receiver: access.expression()? })
    }
}
