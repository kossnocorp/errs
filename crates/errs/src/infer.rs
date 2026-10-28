use crate::lower::{self, Function, Ir};
use crate::{Effect, EffectChannel, ErrorSet, Host, PromiseKind};
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, Kind, Node};
use tsrs_core::P;

/// Checker-local attachments, keyed by native declaration/call identities. Never shared across checker arenas.
pub struct Inference<T> {
    pub functions: FxHashMap<P<Node>, Effect<T>>,
    pub calls: FxHashMap<P<Node>, Effect<T>>,
    pub callbacks: FxHashMap<P<Node>, Effect<T>>,
}

struct Call<T> {
    node: P<Node>,
    target: Option<P<Node>>,
    intrinsic: Option<Effect<T>>,
    promise: bool,
    parameter: Option<P<Node>>,
    arguments: Vec<(P<Node>, Option<P<Node>>)>,
    callback: bool,
}

impl<T: Copy + Eq> Call<T> {
    fn new<H: Host<Type = T>>(host: &mut H, node: P<Node>, callback: bool) -> Self {
        let target = if callback { host.callback_target(node) } else { host.call_target(node) };
        let callee = if callback {
            Some(node)
        } else if node.kind() == Kind::TaggedTemplateExpression {
            Some(node.as_tagged_template_expression().tag)
        } else {
            node.expression()
        };
        let parameter = callee.and_then(|callee| host.callback_parameter(callee));
        let promise = if callback {
            target.is_some_and(|target| host.signature_returns_promise(target))
        } else {
            host.is_promise(node)
        };
        let mut arguments = Vec::new();
        if !callback && let Some(target) = target {
            // Tagged templates and spread arguments need runtime argument expansion; retain uncertainty.
            let positional = node.kind() != Kind::TaggedTemplateExpression
                && !node.arguments().iter().any(|n| n.kind() == Kind::SpreadElement);
            for (index, &parameter) in target
                .parameters()
                .iter()
                .filter(|parameter| parameter.name().is_none_or(|name| name.text() != "this"))
                .enumerate()
            {
                let argument = if positional {
                    node.arguments().get(index).copied().or_else(|| parameter.initializer())
                } else {
                    None
                };
                arguments.push((parameter, argument));
            }
        }
        Self {
            node,
            target,
            intrinsic: if callback { None } else { host.intrinsic_call(node) },
            promise,
            parameter,
            arguments,
            callback,
        }
    }
}

struct Flow<T> {
    throws: ErrorSet<T>,
    value: ErrorSet<T>,
    returned: ErrorSet<T>,
    normal: bool,
}

impl<T: Copy + Eq> Flow<T> {
    fn empty() -> Self {
        Self {
            throws: ErrorSet::default(),
            value: ErrorSet::default(),
            returned: ErrorSet::default(),
            normal: true,
        }
    }

    fn join(&mut self, other: &Self) {
        self.throws.extend(&other.throws);
        self.value.extend(&other.value);
        self.returned.extend(&other.returned);
        self.normal |= other.normal;
    }
}

impl<T: Copy + Eq> Inference<T> {
    pub fn infer<H: Host<Type = T>>(host: &mut H) -> Self {
        fn collect(node: P<Node>, declarations: &mut Vec<P<Node>>, calls: &mut Vec<P<Node>>) {
            if ast::is_function_like(node) && node.kind() != Kind::IndexSignature {
                declarations.push(node);
            }
            if matches!(
                node.kind(),
                Kind::CallExpression | Kind::NewExpression | Kind::TaggedTemplateExpression
            ) {
                calls.push(node);
            }
            node.for_each_child(&mut |n| {
                collect(n, declarations, calls);
                false
            });
        }
        let mut declarations = Vec::new();
        let mut call_nodes = Vec::new();
        for &file in host.files() {
            // Library signatures are queried lazily at calls; walking their bodies would yield no information.
            if !file.is_declaration_file() {
                collect(file.as_node(), &mut declarations, &mut call_nodes);
            }
        }
        let functions: Vec<Function<T>> =
            declarations.iter().map(|&n| lower::function(host, n)).collect();
        let mut calls = Vec::new();
        let mut promise_calls = Vec::new();
        let mut callbacks = FxHashSet::default();
        for node in call_nodes {
            if host.promise_operation(node).is_some() {
                promise_calls.push((node, lower::lower(host, node, 0)));
                for &argument in node.arguments() {
                    if host.callback_target(argument).is_some() && callbacks.insert(argument) {
                        calls.push(Call::new(host, argument, true));
                    }
                }
            } else {
                let call = Call::new(host, node, false);
                for &(_, argument) in &call.arguments {
                    if let Some(argument) = argument
                        && host.callback_target(argument).is_some()
                        && callbacks.insert(argument)
                    {
                        calls.push(Call::new(host, argument, true));
                    }
                }
                calls.push(call);
            }
        }
        Self::solve(host, &functions, &calls, &promise_calls)
    }

    /// Infer only the executable dependency closure of a try body. Native catch typing calls this
    /// before the enclosing function has been checked; walking unrelated handlers here would force
    /// their catch symbols to be cached before their inputs are known.
    pub fn caught<H: Host<Type = T>>(host: &mut H, body: P<Node>) -> ErrorSet<T> {
        Self::expression_flow(host, body).throws
    }

    /// Rejections carried by a promise expression, before a new handler is applied.
    pub fn rejected<H: Host<Type = T>>(host: &mut H, expression: P<Node>) -> ErrorSet<T> {
        Self::expression_flow(host, expression).value
    }

    /// Demand-driven function summary for the native Throws intrinsic, without entering global inference.
    pub fn function_errors<H: Host<Type = T>>(host: &mut H, declaration: P<Node>) -> ErrorSet<T> {
        let function = lower::function(host, declaration);
        let flow = Self::ir_flow(host, &function.body);
        let mut errors = flow.throws;
        errors.extend(&flow.returned);
        errors
    }

    fn expression_flow<H: Host<Type = T>>(host: &mut H, body: P<Node>) -> Flow<T> {
        let ir = lower::lower(host, body, 0);
        Self::ir_flow(host, &ir)
    }

    fn ir_flow<H: Host<Type = T>>(host: &mut H, ir: &Ir<T>) -> Flow<T> {
        let mut pending = Vec::new();
        dependencies(ir, &mut pending);
        let mut seen_calls = FxHashSet::default();
        let mut seen_functions = FxHashSet::default();
        let mut functions = Vec::new();
        let mut calls = Vec::new();
        let mut promises = Vec::new();
        while let Some((node, callback)) = pending.pop() {
            if !seen_calls.insert((node, callback)) {
                continue;
            }
            if !callback && host.promise_operation(node).is_some() {
                let ir = lower::lower(host, node, 0);
                dependencies(&ir, &mut pending);
                promises.push((node, ir));
                continue;
            }
            let call = Call::new(host, node, callback);
            for &(_, argument) in &call.arguments {
                if let Some(argument) = argument
                    && host.callback_target(argument).is_some()
                {
                    pending.push((argument, true));
                }
            }
            if call.intrinsic.is_none()
                && call.parameter.is_none()
                && let Some(target) = call.target
                && seen_functions.insert(target)
            {
                let function = lower::function(host, target);
                dependencies(&function.body, &mut pending);
                functions.push(function);
            }
            calls.push(call);
        }
        let inference = Self::solve(host, &functions, &calls, &promises);
        evaluate(ir, &inference.calls, &inference.callbacks, &[])
    }

    fn solve<H: Host<Type = T>>(
        host: &mut H,
        functions: &[Function<T>],
        calls: &[Call<T>],
        promise_calls: &[(P<Node>, Ir<T>)],
    ) -> Self {
        let mut result = Self {
            functions: functions
                .iter()
                .map(|function| (function.node, Effect::default()))
                .collect(),
            calls: FxHashMap::default(),
            callbacks: FxHashMap::default(),
        };
        // Monotone may-effect fixed point. Keep individual summaries even within recursive components.
        // A bounded fallback handles recursive generic type growth conservatively.
        let mut stable = false;
        for _ in 0..128 {
            let mut changed = result.update_calls(host, calls);
            changed |= result.update_promises(promise_calls);
            for function in functions {
                let flow = evaluate(&function.body, &result.calls, &result.callbacks, &[]);
                let mut effect = Effect::default();
                if function.is_generator {
                    effect.deferred = flow.throws;
                    effect.deferred.extend(&flow.returned);
                } else if function.is_async {
                    effect.rejects = flow.throws;
                    effect.rejects.extend(&flow.returned);
                } else {
                    effect.throws = flow.throws;
                    effect.rejects = flow.returned;
                }
                if function.returns_promise {
                    effect.rejects.unknown = true;
                }
                let previous = result.functions.get_mut(&function.node).unwrap();
                if *previous != effect {
                    *previous = effect;
                    changed = true;
                }
            }
            if !changed {
                stable = true;
                break;
            }
        }
        if !stable {
            for function in functions {
                let effect = result.functions.get_mut(&function.node).unwrap();
                effect.throws.unknown = true;
                effect.rejects.unknown = true;
                if function.is_generator {
                    effect.deferred.unknown = true;
                }
            }
        }
        result.update_calls(host, calls);
        result.update_promises(promise_calls);
        result
    }

    fn update_promises(&mut self, calls: &[(P<Node>, Ir<T>)]) -> bool {
        let mut changed = false;
        for (node, ir) in calls {
            let flow = evaluate(ir, &self.calls, &self.callbacks, &[]);
            let effect = Effect { throws: flow.throws, rejects: flow.value, ..Effect::default() };
            if self.calls.get(node) != Some(&effect) {
                self.calls.insert(*node, effect);
                changed = true;
            }
        }
        changed
    }

    fn update_calls<H: Host<Type = T>>(&mut self, host: &mut H, calls: &[Call<T>]) -> bool {
        let mut changed = false;
        for call in calls {
            let effect = if let Some(parameter) = call.parameter {
                let mut effect = Effect::default();
                effect.throws.insert(host.parameter_effect(parameter, EffectChannel::Throws));
                if call.promise {
                    effect.rejects.insert(host.parameter_effect(parameter, EffectChannel::Rejects));
                }
                effect
            } else if let Some(intrinsic) = &call.intrinsic {
                intrinsic.clone()
            } else if let Some(effect) = call.target.and_then(|n| self.functions.get(&n)) {
                let map = |host: &mut H, set: &ErrorSet<T>| {
                    let mut mapped = ErrorSet {
                        unknown: set.unknown,
                        explicit_unknown: set.explicit_unknown,
                        ..ErrorSet::default()
                    };
                    for &ty in &set.types {
                        if let Some((parameter, channel)) = host.effect_parameter(ty)
                            && let Some((_, argument)) =
                                call.arguments.iter().find(|(candidate, _)| *candidate == parameter)
                        {
                            if let Some(effect) =
                                argument.and_then(|node| self.callbacks.get(&node))
                            {
                                match channel {
                                    EffectChannel::Throws => mapped.extend(&effect.throws),
                                    EffectChannel::Rejects => mapped.extend(&effect.rejects),
                                    EffectChannel::All => {
                                        mapped.extend(&effect.rejects);
                                        mapped.extend(&effect.deferred);
                                        mapped.extend(&effect.throws);
                                    }
                                }
                            } else if argument
                                .is_none_or(|node| host.callback_target(node).is_none())
                            {
                                mapped.unknown = true;
                            }
                        } else {
                            let mut instantiated = host.instantiate(ty, call.node);
                            instantiated.explicit_unknown |= instantiated.unknown;
                            mapped.extend(&instantiated);
                        }
                    }
                    mapped
                };
                Effect {
                    throws: map(host, &effect.throws),
                    rejects: map(host, &effect.rejects),
                    deferred: map(host, &effect.deferred),
                }
            } else {
                Effect {
                    throws: ErrorSet::unknown(),
                    rejects: if call.promise { ErrorSet::unknown() } else { ErrorSet::default() },
                    ..Effect::default()
                }
            };
            let effects = if call.callback { &mut self.callbacks } else { &mut self.calls };
            if effects.get(&call.node) != Some(&effect) {
                effects.insert(call.node, effect);
                changed = true;
            }
        }
        changed
    }

    pub fn at<H: Host<Type = T>>(&self, host: &mut H, node: P<Node>) -> Option<Effect<T>> {
        if ast::is_function_like(node) && node.body().is_none() {
            let target = host.declaration_target(node);
            if target != node {
                return self.functions.get(&target).cloned();
            }
        }
        if let Some(effect) = self.calls.get(&node).or_else(|| self.functions.get(&node)) {
            return Some(effect.clone());
        }
        if ast::is_function_like(node) {
            let target = host.declaration_target(node);
            return Some(self.functions.get(&target).cloned().unwrap_or_else(|| Effect {
                throws: ErrorSet::unknown(),
                rejects: if host.signature_returns_promise(node) {
                    ErrorSet::unknown()
                } else {
                    ErrorSet::default()
                },
                ..Effect::default()
            }));
        }
        None
    }
}

/// Calls actually present in lowered execution, including immutable promise aliases and callbacks.
fn dependencies<T>(ir: &Ir<T>, pending: &mut Vec<(P<Node>, bool)>) {
    match ir {
        Ir::Call(node, parts) => {
            pending.push((*node, false));
            for part in parts {
                dependencies(part, pending);
            }
        }
        Ir::PromiseCall { node, receiver, arguments, callbacks, .. } => {
            pending.push((*node, false));
            for &callback in callbacks.iter().flatten() {
                pending.push((callback, true));
            }
            dependencies(receiver, pending);
            for argument in arguments {
                dependencies(argument, pending);
            }
        }
        Ir::Sequence(parts) | Ir::Alternatives(parts) => {
            for part in parts {
                dependencies(part, pending);
            }
        }
        Ir::Throw(_, value, _, _)
        | Ir::Await(value)
        | Ir::Read(value)
        | Ir::DiscardValue(value)
        | Ir::Return(value)
        | Ir::Loop(value) => dependencies(value, pending),
        Ir::Branch(condition, yes, no) => {
            dependencies(condition, pending);
            dependencies(yes, pending);
            dependencies(no, pending);
        }
        Ir::Try(body, handler, finalizer) => {
            dependencies(body, pending);
            if let Some((_, handler)) = handler {
                dependencies(handler, pending);
            }
            if let Some(finalizer) = finalizer {
                dependencies(finalizer, pending);
            }
        }
        Ir::Empty | Ir::Promise(_) | Ir::Stop | Ir::Unknown(_) => {}
    }
}

fn sequence<T: Copy + Eq>(
    parts: &[Ir<T>],
    calls: &FxHashMap<P<Node>, Effect<T>>,
    callbacks: &FxHashMap<P<Node>, Effect<T>>,
    catches: &[(P<Node>, ErrorSet<T>)],
) -> Flow<T> {
    let mut flow = Flow::empty();
    for part in parts {
        if !flow.normal {
            break;
        }
        let next = evaluate(part, calls, callbacks, catches);
        flow.throws.extend(&next.throws);
        flow.returned.extend(&next.returned);
        flow.value = next.value;
        flow.normal = next.normal;
    }
    flow
}

fn evaluate<T: Copy + Eq>(
    ir: &Ir<T>,
    calls: &FxHashMap<P<Node>, Effect<T>>,
    callbacks: &FxHashMap<P<Node>, Effect<T>>,
    catches: &[(P<Node>, ErrorSet<T>)],
) -> Flow<T> {
    let mut flow = Flow::empty();
    match ir {
        Ir::Empty => {}
        Ir::Unknown(_) => flow.throws.unknown = true,
        Ir::Promise(_) => flow.value.unknown = true,
        Ir::Stop => flow.normal = false,
        Ir::Sequence(parts) => return sequence(parts, calls, callbacks, catches),
        Ir::Alternatives(parts) => {
            for part in parts {
                flow.join(&evaluate(part, calls, callbacks, catches));
            }
        }
        Ir::Loop(body) => {
            flow = evaluate(body, calls, callbacks, catches);
            flow.normal = true;
        }
        Ir::Read(value) => {
            flow.value = evaluate(value, calls, callbacks, catches).value;
        }
        Ir::DiscardValue(value) => {
            flow = evaluate(value, calls, callbacks, catches);
            flow.value = ErrorSet::default();
        }
        Ir::Throw(_, operand, ty, binding) => {
            flow = evaluate(operand, calls, callbacks, catches);
            let errors = binding
                .and_then(|b| catches.iter().rev().find(|(n, _)| *n == b))
                .map_or(ty, |(_, ty)| ty);
            flow.throws.extend(errors);
            flow.normal = false;
        }
        Ir::Return(value) => {
            flow = evaluate(value, calls, callbacks, catches);
            flow.returned.extend(&flow.value);
            flow.normal = false;
        }
        Ir::Await(value) => {
            flow = evaluate(value, calls, callbacks, catches);
            flow.throws.extend(&flow.value);
            flow.value = ErrorSet::default();
        }
        Ir::Call(node, operands) => {
            flow = sequence(operands, calls, callbacks, catches);
            if let Some(effect) = calls.get(node) {
                flow.throws.extend(&effect.throws);
                flow.value.clone_from(&effect.rejects);
            } else {
                flow.throws.unknown = true;
                flow.value.unknown = true;
            }
        }
        Ir::Branch(condition, yes, no) => {
            flow = evaluate(condition, calls, callbacks, catches);
            let mut branches = evaluate(yes, calls, callbacks, catches);
            branches.join(&evaluate(no, calls, callbacks, catches));
            flow.throws.extend(&branches.throws);
            flow.returned.extend(&branches.returned);
            flow.value = branches.value;
            flow.normal = branches.normal;
        }
        Ir::Try(body, handler, finalizer) => {
            flow = evaluate(body, calls, callbacks, catches);
            if let Some((binding, handler)) = handler {
                let mut environment = catches.to_vec();
                if let Some(binding) = binding {
                    environment.push((*binding, flow.throws.clone()));
                }
                let can_throw = !flow.throws.is_empty();
                flow.throws = ErrorSet::default();
                if can_throw {
                    flow.join(&evaluate(handler, calls, callbacks, &environment));
                }
            }
            if let Some(finalizer) = finalizer {
                let final_flow = evaluate(finalizer, calls, callbacks, catches);
                if final_flow.normal {
                    flow.throws.extend(&final_flow.throws);
                    flow.returned.extend(&final_flow.returned);
                } else {
                    flow = final_flow;
                }
            }
        }
        Ir::PromiseCall { kind, receiver, arguments, callbacks: handlers, rejected, .. } => {
            flow = evaluate(receiver, calls, callbacks, catches);
            let incoming = flow.value.clone();
            let arguments = sequence(arguments, calls, callbacks, catches);
            flow.throws.extend(&arguments.throws);
            flow.returned.extend(&arguments.returned);
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
            flow.value = match kind {
                PromiseKind::Reject => rejected.clone(),
                PromiseKind::Resolve => arguments.value,
                PromiseKind::Finally => {
                    let mut errors = incoming;
                    errors.extend(&callback(0));
                    errors
                }
                PromiseKind::Then | PromiseKind::Catch => {
                    let mut errors = callback(0);
                    if handlers[1].is_none() {
                        errors.extend(&incoming);
                    } else if !incoming.is_empty() {
                        errors.extend(&callback(1));
                    }
                    errors
                }
            };
        }
    }
    flow
}
