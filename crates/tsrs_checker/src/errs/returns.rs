//! Native `Returns<Value, Error>` contracts. The intrinsic erases to Value in ordinary TS operations;
//! its error argument is checked separately against the body's inferred effects, never used to infer them.

use crate::*;
use ::errs::ErrorSet;
use tsrs_core::TextRange;

impl Checker {
    fn errs_returns_text(&mut self, declaration: P<Node>, errors: P<Type>) -> String {
        let signature = self.get_signature_from_declaration(declaration);
        let value = self.get_return_type_of_signature(signature);
        let flags = TypeFormatFlags::NoTruncation;
        let value = self.type_to_string_ex(value, Some(declaration), flags, None);
        let errors = self.type_to_string_ex(errors, Some(declaration), flags, None);
        format!("Returns<{value}, {errors}>")
    }

    /// Recompute a missing contract from native types in the current snapshot. Code actions never
    /// parse diagnostic text or reuse a stale inferred annotation from an earlier document version.
    pub fn suggested_returns_annotation(
        &mut self,
        declaration: P<Node>,
    ) -> Option<(String, P<Node>)> {
        if !::errs::enabled() || self.errs_function_contract(declaration).is_some() {
            return None;
        }
        let symbol =
            self.resolve_name(Some(declaration), "Returns", SymbolFlags::Type, None, false, false)?;
        if !self.errs_is_returns_symbol(symbol) {
            return None;
        }
        let effect = self.error_effect_of_node(declaration)?;
        let mut errors = ErrorSet::default();
        for set in [&effect.throws, &effect.rejects, &effect.deferred] {
            errors.extend(set);
        }
        if errors.types.is_empty() && !errors.explicit_unknown {
            return None;
        }
        if errors.unknown {
            errors.types.push(self.unknown_type);
        }
        let errors = self.get_union_type(&errors.types);
        let text = self.errs_returns_text(declaration, errors);
        let signature = self.get_signature_from_declaration(declaration);
        let value = self.get_return_type_of_signature(signature);
        let flags = Flags::NoTruncation;
        let value_node = self.type_to_type_node(value, Some(declaration), flags, None)?;
        let error_node = self.type_to_type_node(errors, Some(declaration), flags, None)?;
        let factory = ast::NodeFactory::new(ast::NodeFactoryHooks::default());
        let annotation = factory.new_type_reference_node(
            factory.new_identifier("Returns"),
            Some(factory.new_node_list(vec![value_node, error_node])),
        );
        Some((text, annotation))
    }

    pub(crate) fn errs_is_returns_declaration(&self, node: P<Node>) -> bool {
        node.kind() == Kind::TypeAliasDeclaration
            && node.name().is_some_and(|name| name.text() == "Returns")
            && node.type_parameters().len() == 2
            && node.type_node().is_some_and(|ty| ty.kind() == Kind::IntrinsicKeyword)
            && ast::get_source_file_of_node(node)
                .is_some_and(|file| self.program.is_source_file_default_library(file.path()))
    }

    pub(crate) fn errs_is_returns_symbol(&self, symbol: P<Symbol>) -> bool {
        symbol.name() == "Returns"
            && symbol.declarations().iter().any(|&node| self.errs_is_returns_declaration(node))
    }

    /// Follow native alias symbols and mappers, preserving the annotation even though its value type erases.
    fn errs_return_contract(&mut self, annotation: P<Node>, depth: usize) -> Option<P<Type>> {
        if depth > 64 {
            return None;
        }
        if annotation.kind() == Kind::ParenthesizedType {
            return self.errs_return_contract(annotation.type_node()?, depth + 1);
        }
        if annotation.kind() != Kind::TypeReference {
            return None;
        }
        let symbol = self.resolve_type_reference_name(annotation, SymbolFlags::Type, true);
        let arguments = annotation.type_arguments();
        if self.errs_is_returns_symbol(symbol) {
            return (arguments.len() == 2).then(|| self.get_type_from_type_node(arguments[1]));
        }
        if matches!(symbol.name(), "Promise" | "PromiseLike")
            && symbol.declarations().iter().any(|&declaration| {
                ast::get_source_file_of_node(declaration)
                    .is_some_and(|file| self.program.is_source_file_default_library(file.path()))
            })
        {
            return self.errs_return_contract(*arguments.first()?, depth + 1);
        }
        if !symbol.flags().intersects(SymbolFlags::TypeAlias) {
            return None;
        }
        let declaration =
            *symbol.declarations().iter().find(|n| n.kind() == Kind::TypeAliasDeclaration)?;
        let contract = self.errs_return_contract(declaration.type_node()?, depth + 1)?;
        // Populate the native alias's type parameters and use native default/argument substitution.
        self.get_declared_type_of_symbol(symbol);
        let parameters = self.type_alias_links.get(symbol).type_parameters.get();
        if parameters.is_empty() {
            return Some(contract);
        }
        let arguments = self.get_type_arguments_from_node(annotation);
        let arguments = self.fill_missing_type_arguments(
            &arguments,
            parameters,
            Self::get_min_type_argument_count(parameters),
            ast::is_in_js_file(Some(annotation)),
        );
        let mapper = new_type_mapper(parameters, alloc_slice(&arguments));
        Some(self.instantiate_type(contract, Some(mapper)))
    }

    pub(crate) fn errs_function_contract(&mut self, node: P<Node>) -> Option<P<Type>> {
        if let Some(annotation) = node.type_node() {
            return self.errs_return_contract(annotation, 0);
        }
        let signature = self
            .get_signature_of_full_signature_type(node)
            .or_else(|| self.get_contextual_signature_for_function_like_declaration(node))?;
        let contract = self.errs_return_contract(signature.declaration()?.type_node()?, 0)?;
        Some(self.instantiate_type(contract, signature.mapper.get()))
    }

    /// Exact error identity: named classes/interfaces stay distinct even if their shapes happen to match.
    /// In particular, the standard `Error` and `TypeError` interfaces have the same structural members.
    fn errs_same_error_type(&mut self, actual: P<Type>, declared: P<Type>, depth: usize) -> bool {
        if actual == declared {
            return true;
        }
        if depth > 64 {
            return false;
        }
        if let Some(actual) = self.errs_throws.types.get(&actual)
            && let Some(declared) = self.errs_throws.types.get(&declared)
        {
            return actual.operand == declared.operand
                && (actual.parameter.is_none()
                    || declared.parameter.is_none()
                    || actual.parameter == declared.parameter);
        }
        if actual.flags().intersects(TypeFlags::Union)
            || declared.flags().intersects(TypeFlags::Union)
        {
            let actual_parts = if actual.flags().intersects(TypeFlags::Union) {
                actual.types()
            } else {
                std::slice::from_ref(&actual)
            };
            let declared_parts = if declared.flags().intersects(TypeFlags::Union) {
                declared.types()
            } else {
                std::slice::from_ref(&declared)
            };
            return actual_parts.iter().all(|&a| {
                declared_parts.iter().any(|&d| self.errs_same_error_type(a, d, depth + 1))
            }) && declared_parts.iter().all(|&d| {
                actual_parts.iter().any(|&a| self.errs_same_error_type(a, d, depth + 1))
            });
        }
        if actual.flags().intersects(TypeFlags::Any | TypeFlags::Unknown | TypeFlags::Never)
            || declared.flags().intersects(TypeFlags::Any | TypeFlags::Unknown | TypeFlags::Never)
        {
            return actual.flags() == declared.flags();
        }
        let named = |ty: P<Type>| {
            ty.flags().intersects(TypeFlags::TypeParameter)
                || ty.symbol().is_some_and(|symbol| {
                    symbol
                        .flags()
                        .intersects(SymbolFlags::Class | SymbolFlags::Interface | SymbolFlags::Enum)
                })
        };
        if (named(actual) || named(declared)) && actual.symbol() != declared.symbol() {
            return false;
        }
        if actual.object_flags().intersects(ObjectFlags::Reference)
            && declared.object_flags().intersects(ObjectFlags::Reference)
        {
            let actual_args = self.get_type_arguments(actual);
            let declared_args = self.get_type_arguments(declared);
            return actual_args.len() == declared_args.len()
                && actual_args
                    .iter()
                    .zip(declared_args)
                    .all(|(&a, &d)| self.errs_same_error_type(a, d, depth + 1));
        }
        self.is_type_identical_to(actual, declared)
    }

    pub fn returns_contract_diagnostics(&mut self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        fn collect(node: P<Node>, functions: &mut Vec<P<Node>>) {
            if ast::is_function_like(node)
                && node.body().is_some()
                && !matches!(node.kind(), Kind::Constructor | Kind::SetAccessor)
            {
                functions.push(node);
            }
            node.for_each_child(&mut |child| {
                collect(child, functions);
                false
            });
        }
        if file.is_declaration_file() || self.compiler_options.no_check.is_true() {
            return Vec::new();
        }
        let mut functions = Vec::new();
        collect(file.as_node(), &mut functions);
        let require = self.compiler_options.error_options.no_implicit_returns == Some(true)
            && !file.file_name().replace('\\', "/").contains("/node_modules/");
        let mut diagnostics = Vec::new();
        for function in functions {
            let Some(effect) = self.error_effect_of_node(function) else {
                continue;
            };
            let mut errors = ErrorSet::default();
            for set in [&effect.throws, &effect.rejects, &effect.deferred] {
                errors.extend(set);
            }
            let throws = !errors.types.is_empty() || errors.explicit_unknown;
            let mut actual_types = errors.types.clone();
            if errors.unknown {
                actual_types.push(self.unknown_type);
            }
            let actual = self.get_union_type(&actual_types);
            let mut declarations: Vec<_> = self
                .get_symbol_of_declaration(function)
                .map(|symbol| {
                    symbol
                        .declarations()
                        .iter()
                        .copied()
                        .filter(|n| n.kind() == function.kind() && n.body().is_none())
                        .collect()
                })
                .unwrap_or_default();
            if declarations.is_empty() || self.errs_function_contract(function).is_some() {
                declarations.push(function);
            }
            for declaration in declarations {
                if let Some(declared) = self.errs_function_contract(declaration) {
                    if !self.errs_same_error_type(actual, declared, 0) {
                        let declared_name = self.type_to_string_exported(declared);
                        let actual_name = self.type_to_string_exported(actual);
                        diagnostics.push(Self::errs_contract_diagnostic(file, declaration.type_node().unwrap_or(declaration), 98002,
                            &format!("Returns error type '{declared_name}' does not exactly match inferred error type '{actual_name}'.")));
                    }
                } else if require && throws {
                    let annotation = self.errs_returns_text(declaration, actual);
                    diagnostics.push(Self::errs_contract_diagnostic(file, declaration.name().unwrap_or(declaration), 98003,
                        &format!("Function has inferred errors but no {annotation} return annotation (errorOptions.noImplicitReturns).")));
                }
            }
        }
        diagnostics
    }

    fn errs_contract_diagnostic(
        file: P<SourceFile>,
        node: P<Node>,
        code: i32,
        message: &str,
    ) -> P<Diagnostic> {
        ast::new_diagnostic_from_text(
            Some(file),
            TextRange::new(
                tsrs_scanner::get_token_pos_of_node(node, file, false),
                node.end().expect_source("Returns annotation"),
            ),
            code,
            diagnostics::Category::Error,
            message,
            &[],
            &[],
            false,
            false,
        )
    }
}
