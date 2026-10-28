//! Native Throws<F> types. Concrete callables reduce to their inferred errors; unresolved operands
//! remain checker-owned, instantiable types. Parameter identity survives structural function typing.

use crate::*;
use ::errs::{EffectChannel, Host, Inference};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThrowsType {
    pub operand: P<Type>,
    pub parameter: Option<P<Node>>,
    pub channel: EffectChannel,
}

#[derive(Default)]
pub struct ThrowsTypes {
    pub types: FxHashMap<P<Type>, ThrowsType>,
    interned: FxHashMap<ThrowsType, P<Type>>,
    active: FxHashSet<P<Type>>,
}

impl Checker {
    pub(crate) fn errs_is_throws_declaration(&self, node: P<Node>) -> bool {
        node.kind() == Kind::TypeAliasDeclaration
            && node.name().is_some_and(|name| name.text() == "Throws")
            && node.type_parameters().len() == 1
            && node.type_node().is_some_and(|ty| ty.kind() == Kind::IntrinsicKeyword)
            && ast::get_source_file_of_node(node)
                .is_some_and(|file| self.program.is_source_file_default_library(file.path()))
    }

    pub(crate) fn errs_is_throws_symbol(&self, symbol: P<Symbol>) -> bool {
        symbol.name() == "Throws"
            && symbol.declarations().iter().any(|&node| self.errs_is_throws_declaration(node))
    }

    pub(crate) fn errs_deferred_throws(
        &mut self,
        operand: P<Type>,
        parameter: Option<P<Node>>,
        channel: EffectChannel,
    ) -> P<Type> {
        let channel = if channel == EffectChannel::All {
            let signatures = self.get_signatures_of_type(operand, SignatureKind::Call);
            if !signatures.is_empty()
                && signatures.iter().all(|&signature| {
                    let returns = self.get_return_type_of_signature(signature);
                    !returns.flags().intersects(TypeFlags::AnyOrUnknown)
                        && self.get_promised_type_of_promise(returns).is_none()
                })
            {
                EffectChannel::Throws
            } else {
                channel
            }
        } else {
            channel
        };
        let key = ThrowsType { operand, parameter, channel };
        if let Some(&ty) = self.errs_throws.interned.get(&key) {
            return ty;
        }
        // Use the native instantiable type-variable machinery, with the intrinsic's payload in a
        // side table. Shared type layouts and ordinary TypeScript type parameters stay unchanged.
        let ty = self.new_type_parameter(None);
        ty.as_type_parameter().constraint.set(Some(self.no_constraint_type));
        self.errs_throws.types.insert(ty, key);
        self.errs_throws.interned.insert(key, ty);
        ty
    }

    pub(crate) fn errs_throws_from_node(&mut self, mut node: P<Node>) -> P<Type> {
        while node.kind() == Kind::ParenthesizedType {
            node = node.type_node().unwrap();
        }
        if node.kind() == Kind::TypeQuery {
            let expression = node.as_type_query_node().expr_name;
            if let Some(parameter) = self.callback_parameter(expression) {
                let operand = self.get_type_at_location(parameter.name().unwrap_or(parameter));
                return self.errs_deferred_throws(operand, Some(parameter), EffectChannel::All);
            }
            if let Some(declaration) = self.callback_target(expression) {
                let operand = self.get_type_at_location(expression);
                return self.errs_throws_of_declaration(operand, declaration);
            }
        }
        let operand = self.get_type_from_type_node(node);
        self.errs_throws_type(operand)
    }

    pub(crate) fn errs_throws_type(&mut self, operand: P<Type>) -> P<Type> {
        if operand.flags().intersects(TypeFlags::Never) {
            return self.never_type;
        }
        if operand.flags().intersects(TypeFlags::AnyOrUnknown) {
            return self.unknown_type;
        }
        if operand.flags().intersects(TypeFlags::Union) {
            let types: Vec<_> =
                operand.types().iter().map(|&ty| self.errs_throws_type(ty)).collect();
            return self.get_union_type(&types);
        }
        if operand.flags().intersects(TypeFlags::Instantiable) {
            return self.errs_deferred_throws(operand, None, EffectChannel::All);
        }
        let signatures = self.get_signatures_of_type(operand, SignatureKind::Call);
        if signatures.is_empty() {
            return self.never_type;
        }
        let mut errors = Vec::new();
        for &signature in signatures {
            let error = if let Some(declaration) = signature.declaration() {
                let declaration = self.declaration_target(declaration);
                let error = self.errs_throws_of_declaration(operand, declaration);
                self.instantiate_type(error, signature.mapper.get())
            } else {
                self.unknown_type
            };
            errors.push(error);
        }
        self.get_union_type(&errors)
    }

    fn errs_throws_of_declaration(&mut self, operand: P<Type>, declaration: P<Node>) -> P<Type> {
        if !self.errs_throws.active.insert(operand) {
            return self.unknown_type;
        }
        let result = if declaration.body().is_some() {
            let errors = Inference::function_errors(self, declaration);
            if errors.unknown { self.unknown_type } else { self.get_union_type(&errors.types) }
        } else {
            self.errs_function_contract(declaration).unwrap_or(self.unknown_type)
        };
        self.errs_throws.active.remove(&operand);
        result
    }

    pub(crate) fn errs_instantiate_throws(
        &mut self,
        ty: P<Type>,
        mapper: P<TypeMapper>,
    ) -> Option<P<Type>> {
        let effect = *self.errs_throws.types.get(&ty)?;
        let operand = self.instantiate_type(effect.operand, Some(mapper));
        Some(if operand == effect.operand { ty } else { self.errs_throws_type(operand) })
    }
}
