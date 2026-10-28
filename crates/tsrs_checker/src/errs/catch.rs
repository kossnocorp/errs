//! Demand-driven native catch types. Keep these independent of the program-wide effect query:
//! inference asks the native checker for operand types, including catch references and their flow types.

use crate::*;

#[derive(Default)]
pub struct CatchTypes {
    types: FxHashMap<P<Node>, P<Type>>,
    active: FxHashSet<P<Node>>,
    recursive: FxHashSet<P<Node>>,
}

impl Checker {
    /// Check the inline handler against the same rejection input used to type its body. In
    /// particular, a never-rejecting promise must accept its inferred `(error: never)` callback.
    pub(crate) fn errs_rejection_callback_context(
        &mut self,
        argument: P<Node>,
        context: P<Type>,
    ) -> P<Type> {
        if !::errs::enabled() {
            return context;
        }
        let function = ast::skip_parentheses(argument);
        if !ast::is_function_expression_or_arrow_function(function) {
            return context;
        }
        let Some(&parameter) = function.parameters().first() else {
            return context;
        };
        let Some(errors) = self.errs_rejection_parameter_type(parameter) else {
            return context;
        };
        self.errs_rejection_context_type(context, errors)
    }

    fn errs_rejection_context_type(&mut self, context: P<Type>, errors: P<Type>) -> P<Type> {
        if context.flags().intersects(TypeFlags::Union) {
            let parts: Vec<_> = context
                .types()
                .iter()
                .map(|&ty| self.errs_rejection_context_type(ty, errors))
                .collect();
            return self.get_union_type(&parts);
        }
        let signatures = self.get_signatures_of_type(context, SignatureKind::Call);
        let [signature] = signatures else {
            return context;
        };
        let Some(&parameter) = signature.parameters().first() else {
            return context;
        };
        let return_type = self.get_return_type_of_signature(*signature);
        let signature = self.clone_signature(*signature);
        let mut parameters = signature.parameters().to_vec();
        parameters[0] = self.create_symbol_with_type(parameter, Some(errors));
        signature.parameters.set(alloc_slice(&parameters));
        signature.resolved_return_type.set(Some(return_type));
        self.get_or_create_type_from_signature(signature)
    }

    pub(crate) fn errs_catch_cycle(&mut self, symbol: P<Symbol>) -> Option<P<Type>> {
        if self.errs_catches.active.is_empty() {
            return None;
        }
        let mut declaration = symbol.value_declaration()?;
        if ast::is_binding_element(declaration) {
            declaration = ast::walk_up_binding_elements_and_patterns(declaration);
        }
        if !self.errs_catches.active.contains(&declaration) {
            return None;
        }
        self.errs_catches.recursive.insert(declaration);
        Some(self.unknown_type)
    }

    pub(crate) fn errs_catch_type(&mut self, declaration: P<Node>) -> Option<P<Type>> {
        if !::errs::enabled() {
            return None;
        }
        let clause = declaration.parent()?;
        if clause.kind() != Kind::CatchClause {
            return None;
        }
        let body = clause.parent()?.as_try_statement().try_block;
        Some(self.errs_handler_type(declaration, body, false))
    }

    /// Contextual rejection parameters must be inferred from the receiver, without resolving the
    /// enclosing call signature (which is currently contextually typing this very callback).
    pub(crate) fn errs_rejection_parameter_type(&mut self, parameter: P<Node>) -> Option<P<Type>> {
        if !::errs::enabled()
            || parameter.kind() != Kind::Parameter
            || has_dot_dot_dot_token(parameter)
            || self.try_get_type_from_type_node(parameter).is_some()
        {
            return None;
        }
        let function = parameter.parent()?;
        if !ast::is_function_expression_or_arrow_function(function)
            || function.parameters().first().copied() != Some(parameter)
        {
            return None;
        }
        let mut argument = function;
        while argument.parent().is_some_and(|parent| parent.kind() == Kind::ParenthesizedExpression)
        {
            argument = argument.parent()?;
        }
        let call = argument.parent()?;
        if call.kind() != Kind::CallExpression {
            return None;
        }
        let access = call.expression()?;
        if access.kind() != Kind::PropertyAccessExpression {
            return None;
        }
        let name = access.name()?.text();
        let index = match name {
            "catch" => 0,
            "then" => 1,
            _ => return None,
        };
        if call.arguments().get(index).copied() != Some(argument) {
            return None;
        }
        let receiver = access.expression()?;
        let receiver_type = self.get_type_at_location(receiver);
        let property = self.get_property_of_type(receiver_type, name)?;
        if !property.declarations().iter().any(|&declaration| {
            declaration
                .parent()
                .and_then(|parent| parent.name())
                .is_some_and(|name| matches!(name.text(), "Promise" | "PromiseLike"))
                && ast::get_source_file_of_node(declaration)
                    .is_some_and(|file| self.program.is_source_file_default_library(file.path()))
        }) {
            return None;
        }
        Some(self.errs_handler_type(parameter, receiver, true))
    }

    fn errs_handler_type(
        &mut self,
        declaration: P<Node>,
        body: P<Node>,
        rejection: bool,
    ) -> P<Type> {
        if let Some(&ty) = self.errs_catches.types.get(&declaration) {
            return ty;
        }
        // Recursive dependencies through a handler must not turn uncertainty into `never`, nor recurse
        // through program inference while its attachments are being built or temporarily borrowed.
        if !self.errs_catches.active.insert(declaration) {
            self.errs_catches.recursive.insert(declaration);
            return self.unknown_type;
        }
        let errors = if rejection {
            ::errs::Inference::rejected(self, body)
        } else {
            ::errs::Inference::caught(self, body)
        };
        let recursive = self.errs_catches.recursive.remove(&declaration);
        let ty = if errors.unknown || recursive {
            self.unknown_type
        } else {
            self.get_union_type(&errors.types)
        };
        self.errs_catches.active.remove(&declaration);
        self.errs_catches.types.insert(declaration, ty);
        ty
    }
}
