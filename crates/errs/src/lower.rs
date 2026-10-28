use crate::{ErrorSet, Host, PromiseKind};
use tsrs_ast::{self as ast, Kind, ModifierFlags, Node};
use tsrs_core::P;

pub enum Ir<T> {
    Empty,
    Sequence(Vec<Ir<T>>),
    Alternatives(Vec<Ir<T>>),
    Branch(Box<Ir<T>>, Box<Ir<T>>, Box<Ir<T>>),
    Throw(P<Node>, Box<Ir<T>>, ErrorSet<T>, Option<P<Node>>),
    Call(P<Node>, Vec<Ir<T>>),
    Await(Box<Ir<T>>),
    Read(Box<Ir<T>>),
    DiscardValue(Box<Ir<T>>),
    Promise(P<Node>),
    Return(Box<Ir<T>>),
    Stop,
    Try(Box<Ir<T>>, Option<(Option<P<Node>>, Box<Ir<T>>)>, Option<Box<Ir<T>>>),
    Loop(Box<Ir<T>>),
    Unknown(P<Node>),
    PromiseCall {
        node: P<Node>,
        kind: PromiseKind,
        receiver: Box<Ir<T>>,
        arguments: Vec<Ir<T>>,
        callbacks: [Option<P<Node>>; 2],
        rejected: ErrorSet<T>,
    },
}

pub struct Function<T> {
    pub node: P<Node>,
    pub body: Ir<T>,
    pub is_async: bool,
    pub is_generator: bool,
    pub returns_promise: bool,
}

pub fn function<H: Host>(host: &mut H, node: P<Node>) -> Function<H::Type> {
    let mut parts = Vec::new();
    for &parameter in node.parameters() {
        if let Some(initializer) = parameter.initializer() {
            parts.push(lower(host, initializer, 0));
        }
    }
    if node.kind() == Kind::Constructor
        && let Some(class) = node.parent()
    {
        for &member in class.members() {
            if member.kind() == Kind::PropertyDeclaration
                && !ast::get_combined_modifier_flags(member).intersects(ModifierFlags::Static)
                && let Some(initializer) = member.initializer()
            {
                parts.push(lower(host, initializer, 0));
            }
        }
    }
    if let Some(body) = node.body() {
        let body_ir = lower(host, body, 0);
        parts.push(if body.kind() == Kind::Block {
            body_ir
        } else {
            Ir::Return(Box::new(if host.is_promise(body) {
                body_ir
            } else {
                Ir::DiscardValue(Box::new(body_ir))
            }))
        });
    } else {
        parts.push(Ir::Unknown(node));
    }
    Function {
        node,
        body: Ir::Sequence(parts),
        is_async: ast::get_combined_modifier_flags(node).intersects(ModifierFlags::Async),
        is_generator: ast::get_function_flags(Some(node)).intersects(ast::FunctionFlags::Generator),
        returns_promise: node.body().is_none() && host.signature_returns_promise(node),
    }
}

pub fn lower<H: Host>(host: &mut H, node: P<Node>, depth: usize) -> Ir<H::Type> {
    if depth > 128 {
        return Ir::Unknown(node);
    }
    if ast::is_function_like(node)
        || ast::is_type_node(node)
        || ast::get_combined_modifier_flags(node).intersects(ModifierFlags::Ambient)
    {
        return Ir::Empty;
    }
    let child = |host: &mut H, node| Box::new(lower(host, node, depth + 1));
    match node.kind() {
        Kind::ThrowStatement => {
            let expression = node.as_throw_statement().expression();
            Ir::Throw(
                node,
                child(host, expression),
                explicit(host.type_of(expression)),
                host.catch_binding(expression),
            )
        }
        Kind::CallExpression | Kind::NewExpression | Kind::TaggedTemplateExpression => {
            if let Some(operation) = host.promise_operation(node) {
                let args = node.arguments();
                let callbacks = match operation.kind {
                    PromiseKind::Then => [args.first().copied(), args.get(1).copied()],
                    PromiseKind::Catch => [None, args.first().copied()],
                    PromiseKind::Finally => [args.first().copied(), None],
                    PromiseKind::Resolve | PromiseKind::Reject => [None, None],
                }
                .map(|callback| callback.filter(|&n| host.callback_target(n).is_some()));
                let rejected = if matches!(operation.kind, PromiseKind::Reject) {
                    args.first().map_or_else(ErrorSet::unknown, |&n| host.type_of(n))
                } else {
                    ErrorSet::default()
                };
                return Ir::PromiseCall {
                    node,
                    kind: operation.kind,
                    receiver: child(host, operation.receiver),
                    arguments: args.iter().map(|&n| lower(host, n, depth + 1)).collect(),
                    callbacks,
                    rejected: explicit(rejected),
                };
            }
            // Only runtime operands: type arguments are erased, nested function bodies are deferred.
            let mut operands = Vec::new();
            node.for_each_child(&mut |n| {
                operands.push(lower(host, n, depth + 1));
                false
            });
            Ir::Call(node, operands)
        }
        Kind::AwaitExpression => {
            let expression = node.expression().unwrap();
            let value = child(host, expression);
            Ir::Await(if host.is_promise(expression) {
                value
            } else {
                Box::new(Ir::DiscardValue(value))
            })
        }
        Kind::ReturnStatement => Ir::Return(node.expression().map_or_else(
            || Box::new(Ir::Empty),
            |n| {
                let value = child(host, n);
                if host.is_promise(n) { value } else { Box::new(Ir::DiscardValue(value)) }
            },
        )),
        Kind::BreakStatement | Kind::ContinueStatement => Ir::Stop,
        Kind::IfStatement => {
            let statement = node.as_if_statement();
            if statement.expression.kind() == Kind::TrueKeyword {
                return lower(host, statement.then_statement, depth + 1);
            }
            if statement.expression.kind() == Kind::FalseKeyword {
                return statement.else_statement.map_or(Ir::Empty, |n| lower(host, n, depth + 1));
            }
            Ir::Branch(
                child(host, statement.expression),
                child(host, statement.then_statement),
                statement.else_statement.map_or_else(|| Box::new(Ir::Empty), |n| child(host, n)),
            )
        }
        Kind::ConditionalExpression => {
            let expression = node.as_conditional_expression();
            Ir::Branch(
                child(host, expression.condition),
                child(host, expression.when_true),
                child(host, expression.when_false),
            )
        }
        Kind::TryStatement => {
            let statement = node.as_try_statement();
            let body = child(host, statement.try_block);
            let handler = statement.catch_clause.map(|n| {
                let catch = n.as_catch_clause();
                (catch.variable_declaration, child(host, catch.block))
            });
            Ir::Try(body, handler, statement.finally_block.map(|n| child(host, n)))
        }
        Kind::Identifier => {
            if ast::is_declaration_name(node)
                && node
                    .parent()
                    .is_none_or(|parent| parent.kind() != Kind::ShorthandPropertyAssignment)
            {
                return Ir::Empty;
            }
            // Reading a promise does not re-execute the initializer's synchronous effects.
            if let Some(initializer) = host.initializer(node) {
                Ir::Read(child(host, initializer))
            } else if host.is_promise(node) {
                Ir::Promise(node)
            } else {
                Ir::Empty
            }
        }
        Kind::ClassDeclaration | Kind::ClassExpression => {
            // Only class-definition-time execution belongs to the surrounding scope.
            let mut parts = Vec::new();
            for &ty in ast::get_heritage_elements(node, Kind::ExtendsKeyword) {
                if let Some(expression) = ty.expression() {
                    parts.push(lower(host, expression, depth + 1));
                }
            }
            for &member in node.members() {
                if let Some(name) = member.name().filter(|n| n.kind() == Kind::ComputedPropertyName)
                {
                    parts.push(lower(host, name.expression().unwrap(), depth + 1));
                }
                if member.kind() == Kind::ClassStaticBlockDeclaration {
                    parts.push(lower(
                        host,
                        member.as_class_static_block_declaration().body,
                        depth + 1,
                    ));
                } else if member.kind() == Kind::PropertyDeclaration
                    && ast::get_combined_modifier_flags(member).intersects(ModifierFlags::Static)
                    && let Some(initializer) = member.initializer()
                {
                    parts.push(lower(host, initializer, depth + 1));
                }
            }
            Ir::Sequence(parts)
        }
        Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
            // Property names are not runtime value reads (an unresolved name can have native any).
            let mut parts = vec![lower(host, node.expression().unwrap(), depth + 1)];
            if node.kind() == Kind::ElementAccessExpression {
                parts.push(lower(
                    host,
                    node.as_element_access_expression().argument_expression,
                    depth + 1,
                ));
            }
            // Accessor invocation needs its own instantiated receiver context. Preserve uncertainty until modelled.
            if host.has_accessor(node) {
                parts.push(Ir::Unknown(node));
            }
            if host.is_promise(node) {
                parts.push(Ir::Promise(node));
            }
            Ir::Sequence(parts)
        }
        Kind::CaseBlock => {
            let mut alternatives = Vec::new();
            node.for_each_child(&mut |n| {
                alternatives.push(lower(host, n, depth + 1));
                false
            });
            Ir::Alternatives(alternatives)
        }
        Kind::WhileStatement
        | Kind::DoStatement
        | Kind::ForStatement
        | Kind::ForInStatement
        | Kind::ForOfStatement
        | Kind::SwitchStatement => {
            // Iteration/branch exits must not terminate the enclosing statement sequence.
            let mut parts = Vec::new();
            node.for_each_child(&mut |n| {
                parts.push(lower(host, n, depth + 1));
                false
            });
            Ir::Loop(Box::new(Ir::Sequence(parts)))
        }
        _ => {
            let mut parts = Vec::new();
            node.for_each_child(&mut |n| {
                parts.push(lower(host, n, depth + 1));
                false
            });
            Ir::Sequence(parts)
        }
    }
}

fn explicit<T>(mut errors: ErrorSet<T>) -> ErrorSet<T> {
    errors.explicit_unknown |= errors.unknown;
    errors
}
