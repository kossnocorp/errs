//! Root execution analysis. Synchronous throws and outstanding promises are separate obligations:
//! a try block only handles a rejection once that promise is awaited.

use crate::lower::{self, Ir};
use crate::{Effect, ErrorSet, Host, Inference, PromiseKind};
use rustc_hash::FxHashMap;
use tsrs_ast::{Node, SourceFile};
use tsrs_core::P;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootError<T> {
    pub node: P<Node>,
    pub errors: ErrorSet<T>,
    pub rejection: bool,
}

#[derive(Clone)]
struct Flow<T> {
    throws: Vec<RootError<T>>,
    pending: Vec<RootError<T>>,
    value: Vec<RootError<T>>,
    normal: bool,
}

impl<T: Copy + Eq> Flow<T> {
    fn empty() -> Self {
        Self { throws: Vec::new(), pending: Vec::new(), value: Vec::new(), normal: true }
    }

    fn join(&mut self, other: &Self) {
        extend(&mut self.throws, &other.throws);
        extend(&mut self.pending, &other.pending);
        extend(&mut self.value, &other.value);
        self.normal |= other.normal;
    }

    fn consume(&mut self, values: &[RootError<T>]) {
        self.pending.retain(|error| !values.iter().any(|value| value.node == error.node));
    }

    fn promise(&mut self, node: P<Node>, errors: ErrorSet<T>) {
        self.value.clear();
        if !errors.is_empty() {
            let error = RootError { node, errors, rejection: true };
            extend(&mut self.pending, std::slice::from_ref(&error));
            self.value.push(error);
        }
    }
}

pub fn extend<T: Copy + Eq>(target: &mut Vec<RootError<T>>, source: &[RootError<T>]) {
    for error in source {
        if let Some(existing) =
            target.iter_mut().find(|e| e.node == error.node && e.rejection == error.rejection)
        {
            existing.errors.extend(&error.errors);
        } else {
            target.push(error.clone());
        }
    }
}

pub fn types<T: Copy + Eq>(values: &[RootError<T>]) -> ErrorSet<T> {
    let mut errors = ErrorSet::default();
    for value in values {
        errors.extend(&value.errors);
    }
    errors
}

impl<T: Copy + Eq> Inference<T> {
    pub fn root_errors<H: Host<Type = T>>(
        &self,
        host: &mut H,
        file: P<SourceFile>,
    ) -> Vec<RootError<T>> {
        if file.is_declaration_file() {
            return Vec::new();
        }
        let ir = lower::lower(host, file.as_node(), 0);
        let mut flow = Flow::empty();
        evaluate(&ir, &self.calls, &self.callbacks, &[], &mut flow);
        extend(&mut flow.throws, &flow.pending);
        flow.throws.retain(|error| !error.errors.is_empty());
        flow.throws
    }
}

fn sequence<T: Copy + Eq>(
    parts: &[Ir<T>],
    calls: &FxHashMap<P<Node>, Effect<T>>,
    callbacks: &FxHashMap<P<Node>, Effect<T>>,
    catches: &[(P<Node>, ErrorSet<T>)],
    flow: &mut Flow<T>,
) {
    for part in parts {
        if !flow.normal {
            break;
        }
        evaluate(part, calls, callbacks, catches, flow);
    }
}

fn evaluate<T: Copy + Eq>(
    ir: &Ir<T>,
    calls: &FxHashMap<P<Node>, Effect<T>>,
    callbacks: &FxHashMap<P<Node>, Effect<T>>,
    catches: &[(P<Node>, ErrorSet<T>)],
    flow: &mut Flow<T>,
) {
    flow.value.clear();
    match ir {
        Ir::Empty => {}
        Ir::Unknown(node) => extend(
            &mut flow.throws,
            &[RootError { node: *node, errors: ErrorSet::unknown(), rejection: false }],
        ),
        Ir::Promise(node) => flow.promise(*node, ErrorSet::unknown()),
        Ir::Stop => flow.normal = false,
        Ir::Sequence(parts) => sequence(parts, calls, callbacks, catches, flow),
        Ir::Alternatives(parts) => {
            let initial = flow.clone();
            for part in parts {
                let mut branch = initial.clone();
                evaluate(part, calls, callbacks, catches, &mut branch);
                flow.join(&branch);
            }
        }
        Ir::Branch(condition, yes, no) => {
            evaluate(condition, calls, callbacks, catches, flow);
            let mut alternate = flow.clone();
            evaluate(yes, calls, callbacks, catches, flow);
            evaluate(no, calls, callbacks, catches, &mut alternate);
            flow.join(&alternate);
        }
        Ir::Loop(body) => {
            let mut body_flow = flow.clone();
            evaluate(body, calls, callbacks, catches, &mut body_flow);
            flow.join(&body_flow);
            flow.normal = true;
        }
        Ir::Read(initializer) => {
            // Immutable aliases refer to the original promise obligation; reading does not create another one.
            let mut isolated = Flow::empty();
            evaluate(initializer, calls, callbacks, catches, &mut isolated);
            flow.value = isolated.value;
        }
        Ir::DiscardValue(value) => {
            evaluate(value, calls, callbacks, catches, flow);
            flow.value.clear();
        }
        Ir::Call(node, operands) => {
            sequence(operands, calls, callbacks, catches, flow);
            if let Some(effect) = calls.get(node) {
                if !effect.throws.is_empty() {
                    extend(
                        &mut flow.throws,
                        &[RootError {
                            node: *node,
                            errors: effect.throws.clone(),
                            rejection: false,
                        }],
                    );
                }
                flow.promise(*node, effect.rejects.clone());
            }
        }
        Ir::Throw(node, operand, errors, binding) => {
            evaluate(operand, calls, callbacks, catches, flow);
            let errors = binding
                .and_then(|binding| catches.iter().rev().find(|(n, _)| *n == binding))
                .map_or(errors, |(_, errors)| errors);
            extend(
                &mut flow.throws,
                &[RootError { node: *node, errors: errors.clone(), rejection: false }],
            );
            flow.normal = false;
        }
        Ir::Return(value) => {
            evaluate(value, calls, callbacks, catches, flow);
            flow.normal = false;
        }
        Ir::Await(value) => {
            evaluate(value, calls, callbacks, catches, flow);
            let values = std::mem::take(&mut flow.value);
            flow.consume(&values);
            for mut error in values {
                error.rejection = false;
                extend(&mut flow.throws, &[error]);
            }
        }
        Ir::Try(body, handler, finalizer) => {
            let previous = std::mem::take(&mut flow.throws);
            evaluate(body, calls, callbacks, catches, flow);
            if let Some((binding, handler)) = handler {
                let incoming = types(&flow.throws);
                flow.throws.clear();
                if !incoming.is_empty() {
                    let mut environment = catches.to_vec();
                    if let Some(binding) = binding {
                        environment.push((*binding, incoming));
                    }
                    let mut handled = flow.clone();
                    handled.normal = true;
                    evaluate(handler, calls, callbacks, &environment, &mut handled);
                    if flow.normal {
                        flow.join(&handled);
                    } else {
                        *flow = handled;
                    }
                }
            }
            if let Some(finalizer) = finalizer {
                let normal = flow.normal;
                let throws = std::mem::take(&mut flow.throws);
                flow.normal = true;
                evaluate(finalizer, calls, callbacks, catches, flow);
                if flow.normal {
                    extend(&mut flow.throws, &throws);
                    flow.normal = normal;
                }
            }
            extend(&mut flow.throws, &previous);
        }
        Ir::PromiseCall { node, kind, receiver, arguments, callbacks: handlers, rejected } => {
            evaluate(receiver, calls, callbacks, catches, flow);
            let receiver = std::mem::take(&mut flow.value);
            sequence(arguments, calls, callbacks, catches, flow);
            let argument = std::mem::take(&mut flow.value);
            let incoming = types(&receiver);
            let callback = |index: usize| {
                handlers[index]
                    .and_then(|n| callbacks.get(&n))
                    .map(|effect| {
                        let mut errors = effect.throws.clone();
                        errors.extend(&effect.rejects);
                        errors
                    })
                    .unwrap_or_default()
            };
            let errors = match kind {
                PromiseKind::Reject => rejected.clone(),
                PromiseKind::Resolve => {
                    flow.consume(&argument);
                    types(&argument)
                }
                PromiseKind::Finally => {
                    flow.consume(&receiver);
                    let mut errors = incoming;
                    errors.extend(&callback(0));
                    errors
                }
                PromiseKind::Then | PromiseKind::Catch => {
                    flow.consume(&receiver);
                    let mut errors = callback(0);
                    if handlers[1].is_none() {
                        errors.extend(&incoming);
                    } else if !incoming.is_empty() {
                        errors.extend(&callback(1));
                    }
                    errors
                }
            };
            flow.promise(*node, errors);
        }
    }
}
