//! Local promise ownership, checked in every executable scope. Detached rejections do not become
//! returned-promise effects: a caller's catch cannot handle a promise discarded inside its callee.
use crate::lower::{self, Ir};
use crate::roots::{extend, types};
use crate::{ErrorSet, Host, Inference, PromiseKind, RootError};
use tsrs_ast::{self as ast, Kind, Node, SourceFile};
use tsrs_core::P;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Completion {
    Normal,
    Return,
    Throw,
    Stop,
}

#[derive(Clone)]
struct Flow<T> {
    pending: Vec<RootError<T>>,
    value: Vec<RootError<T>>,
    thrown: ErrorSet<T>,
    completion: Completion,
}

impl<T: Copy + Eq> Flow<T> {
    fn empty() -> Self {
        Self {
            pending: Vec::new(),
            value: Vec::new(),
            thrown: ErrorSet::default(),
            completion: Completion::Normal,
        }
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

// Keep abrupt exits separate: handling a promise after a conditional return cannot handle the
// exited path. Bound distinct value paths; overflow intersects values, conservatively retaining
// obligations rather than treating a possibly different promise as definitely handled.
fn merge<T: Copy + Eq>(flows: Vec<Flow<T>>) -> Vec<Flow<T>> {
    let bounded = flows.len() > 64;
    let mut result: Vec<Flow<T>> = Vec::new();
    for flow in flows {
        if let Some(existing) = result.iter_mut().find(|candidate| {
            candidate.completion == flow.completion && (bounded || candidate.value == flow.value)
        }) {
            extend(&mut existing.pending, &flow.pending);
            existing.thrown.extend(&flow.thrown);
            if bounded {
                existing.value.retain(|value| flow.value.contains(value));
            }
        } else {
            result.push(flow);
        }
    }
    result
}

impl<T: Copy + Eq> Inference<T> {
    pub fn unhandled_promises<H: Host<Type = T>>(
        &self,
        host: &mut H,
        file: P<SourceFile>,
    ) -> Vec<RootError<T>> {
        fn collect(node: P<Node>, scopes: &mut Vec<P<Node>>) {
            if ast::is_function_like(node) && node.body().is_some()
                || node.kind() == Kind::PropertyDeclaration && node.initializer().is_some()
            {
                scopes.push(node);
            }
            node.for_each_child(&mut |child| {
                collect(child, scopes);
                false
            });
        }
        if file.is_declaration_file() {
            return Vec::new();
        }
        let mut scopes = vec![file.as_node()];
        collect(file.as_node(), &mut scopes);
        let mut errors = Vec::new();
        for scope in scopes {
            let ir = if ast::is_function_like(scope) {
                lower::function(host, scope).body
            } else {
                lower::lower(host, scope, 0)
            };
            for mut flow in self.promise_flow(&ir, Flow::empty(), &[]) {
                if flow.completion == Completion::Return {
                    let returned = std::mem::take(&mut flow.value);
                    flow.consume(&returned);
                }
                extend(&mut errors, &flow.pending);
            }
        }
        errors
    }

    fn promise_sequence(
        &self,
        parts: &[Ir<T>],
        mut flows: Vec<Flow<T>>,
        catches: &[(P<Node>, ErrorSet<T>)],
    ) -> Vec<Flow<T>> {
        for part in parts {
            flows = merge(
                flows.into_iter().flat_map(|flow| self.promise_flow(part, flow, catches)).collect(),
            );
        }
        flows
    }

    fn promise_flow(
        &self,
        ir: &Ir<T>,
        mut flow: Flow<T>,
        catches: &[(P<Node>, ErrorSet<T>)],
    ) -> Vec<Flow<T>> {
        if flow.completion != Completion::Normal {
            return vec![flow];
        }
        flow.value.clear();
        match ir {
            Ir::Empty => {}
            Ir::Stop => flow.completion = Completion::Stop,
            Ir::Unknown(_) => return throwing(flow, ErrorSet::unknown()),
            Ir::Promise(node) => flow.promise(*node, ErrorSet::unknown()),
            Ir::Sequence(parts) => return self.promise_sequence(parts, vec![flow], catches),
            Ir::Alternatives(parts) => {
                let mut paths = vec![flow.clone()];
                for part in parts {
                    paths.extend(self.promise_flow(part, flow.clone(), catches));
                    paths = merge(paths);
                }
                return paths;
            }
            Ir::Branch(condition, yes, no) => {
                let mut paths = Vec::new();
                for flow in self.promise_flow(condition, flow, catches) {
                    paths.extend(self.promise_flow(yes, flow.clone(), catches));
                    paths.extend(self.promise_flow(no, flow, catches));
                }
                return merge(paths);
            }
            Ir::Loop(body) => {
                let mut paths = vec![flow.clone()];
                for mut flow in self.promise_flow(body, flow, catches) {
                    if flow.completion == Completion::Stop {
                        flow.completion = Completion::Normal;
                    }
                    paths.push(flow);
                }
                return merge(paths);
            }
            Ir::Read(initializer) => {
                // Reading an alias neither re-executes its initializer nor creates an obligation.
                return self
                    .promise_flow(initializer, Flow::empty(), catches)
                    .into_iter()
                    .filter(|value| value.completion == Completion::Normal)
                    .map(|value| {
                        let mut flow = flow.clone();
                        flow.value = value.value;
                        flow
                    })
                    .collect();
            }
            Ir::DiscardValue(value) | Ir::Return(value) | Ir::Await(value) => {
                let mut paths = Vec::new();
                for mut flow in self.promise_flow(value, flow, catches) {
                    if flow.completion == Completion::Normal {
                        match ir {
                            Ir::Return(_) => flow.completion = Completion::Return,
                            Ir::Await(_) => {
                                let values = std::mem::take(&mut flow.value);
                                flow.consume(&values);
                                paths.extend(throwing(flow, types(&values)));
                                continue;
                            }
                            _ => flow.value.clear(),
                        }
                    }
                    paths.push(flow);
                }
                return merge(paths);
            }
            Ir::Call(node, operands) => {
                let mut paths = Vec::new();
                for flow in self.promise_sequence(operands, vec![flow], catches) {
                    if flow.completion != Completion::Normal {
                        paths.push(flow);
                        continue;
                    }
                    let effect = self.calls.get(node).cloned().unwrap_or_default();
                    for mut flow in throwing(flow, effect.throws) {
                        if flow.completion == Completion::Normal {
                            flow.promise(*node, effect.rejects.clone());
                        }
                        paths.push(flow);
                    }
                }
                return merge(paths);
            }
            Ir::Throw(_, operand, errors, binding) => {
                let errors = binding
                    .and_then(|binding| catches.iter().rev().find(|(n, _)| *n == binding))
                    .map_or(errors, |(_, errors)| errors);
                return self
                    .promise_flow(operand, flow, catches)
                    .into_iter()
                    .map(|mut flow| {
                        if flow.completion == Completion::Normal {
                            flow.completion = Completion::Throw;
                            flow.thrown = errors.clone();
                        }
                        flow
                    })
                    .collect();
            }
            Ir::Try(body, handler, finalizer) => {
                let mut paths = Vec::new();
                for mut flow in self.promise_flow(body, flow, catches) {
                    if flow.completion == Completion::Throw
                        && let Some((binding, handler)) = handler
                    {
                        let mut environment = catches.to_vec();
                        if let Some(binding) = binding {
                            environment.push((*binding, flow.thrown.clone()));
                        }
                        flow.completion = Completion::Normal;
                        flow.thrown = ErrorSet::default();
                        paths.extend(self.promise_flow(handler, flow, &environment));
                    } else {
                        paths.push(flow);
                    }
                }
                if let Some(finalizer) = finalizer {
                    let mut finalized = Vec::new();
                    for mut flow in merge(paths) {
                        let completion = flow.completion;
                        let value = std::mem::take(&mut flow.value);
                        let thrown = std::mem::take(&mut flow.thrown);
                        flow.completion = Completion::Normal;
                        for mut flow in self.promise_flow(finalizer, flow, catches) {
                            if flow.completion == Completion::Normal {
                                flow.completion = completion;
                                flow.value.clone_from(&value);
                                flow.thrown = thrown.clone();
                            }
                            finalized.push(flow);
                        }
                    }
                    paths = finalized;
                }
                return merge(paths);
            }
            Ir::PromiseCall { node, kind, receiver, arguments, .. } => {
                let mut paths = Vec::new();
                for mut flow in self.promise_flow(receiver, flow, catches) {
                    let receiver = std::mem::take(&mut flow.value);
                    for mut flow in self.promise_sequence(arguments, vec![flow], catches) {
                        if flow.completion == Completion::Normal {
                            match kind {
                                PromiseKind::Resolve => {
                                    let argument = std::mem::take(&mut flow.value);
                                    flow.consume(&argument);
                                }
                                PromiseKind::Reject => {}
                                _ => flow.consume(&receiver),
                            }
                            let errors = self
                                .calls
                                .get(node)
                                .map_or_else(ErrorSet::unknown, |effect| effect.rejects.clone());
                            flow.promise(*node, errors);
                        }
                        paths.push(flow);
                    }
                }
                return merge(paths);
            }
        }
        vec![flow]
    }
}

fn throwing<T: Copy + Eq>(flow: Flow<T>, errors: ErrorSet<T>) -> Vec<Flow<T>> {
    if errors.is_empty() {
        return vec![flow];
    }
    let mut thrown = flow.clone();
    thrown.completion = Completion::Throw;
    thrown.thrown = errors;
    vec![flow, thrown]
}
