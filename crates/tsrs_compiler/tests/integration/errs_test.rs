//! Fork-specific effect semantics, deliberately separate from TypeScript's reference diagnostic baselines.
use std::sync::Arc;
use tsrs_ast::{self as ast, Kind, Node};
use tsrs_compiler::{Context, ProgramOptions, new_compiler_host, new_program};
use tsrs_core::tspath::ComparePathsOptions;
use tsrs_core::{CompilerOptions, P, Tristate};
use tsrs_tsoptions::new_parsed_command_line;
use tsrs_vfs::{FS, bundled, vfstest};

const SOURCE: &str = include_str!("../../../../testdata/errs/inference.ts");
const JS: &str = include_str!("../../../../testdata/errs/dependency.js");

fn visit(node: P<Node>, functions: &mut Vec<P<Node>>, calls: &mut Vec<P<Node>>) {
    if ast::is_function_like(node) {
        functions.push(node);
    }
    if node.kind() == Kind::CallExpression
        && node.parent().is_some_and(|n| n.parent().is_some_and(ast::is_source_file))
    {
        calls.push(node);
    }
    node.for_each_child(&mut |n| {
        visit(n, functions, calls);
        false
    });
}

#[test]
fn native_error_effects() {
    std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(|| {
            let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
                [("/index.ts", SOURCE), ("/dependency.js", JS)],
                true,
            )));
            let options = CompilerOptions {
                allow_js: Tristate::True,
                check_js: Tristate::False,
                no_emit: Tristate::True,
                ..CompilerOptions::default()
            };
            let config = new_parsed_command_line(
                P::new(options),
                vec!["/index.ts".to_string()],
                Vec::new(),
                ComparePathsOptions {
                    use_case_sensitive_file_names: true,
                    current_directory: "/".to_string(),
                },
            );
            let host = new_compiler_host("/", fs, &bundled::lib_path(), None, None, None);
            let program = new_program(ProgramOptions::new(P::new(config), host));
            let context = Context::background();
            let mut checker = program.get_type_checker(&context);
            let file = program.get_source_file("/index.ts").unwrap();
            let mut functions = Vec::new();
            let mut calls = Vec::new();
            visit(file.as_node(), &mut functions, &mut calls);
            let expected = [
                ("direct", "throws: TypeError"),
                ("transitive", "throws: RangeError | SyntaxError | TypeError"),
                ("caught", "throws: never"),
                ("rethrown", "throws: TypeError"),
                ("replaced", "throws: SyntaxError"),
                ("suppressed", "throws: never"),
                ("unreachable", "throws: never"),
                ("nested", "throws: never"),
                ("generic", "throws: T"),
                ("genericCaller", "throws: RangeError"),
                ("genericWrapper", "throws: T"),
                ("genericTransitive", "throws: SyntaxError"),
                ("even", "throws: TypeError"),
                ("odd", "throws: TypeError"),
                ("rejection", "throws: never\nrejects: RangeError"),
                ("awaited", "throws: never\nrejects: RangeError"),
                ("unawaited", "throws: never"),
                ("caughtRejection", "throws: never"),
                ("returnedRejection", "throws: never\nrejects: RangeError"),
                ("storedRejection", "throws: never\nrejects: RangeError"),
                ("promiseArray", "throws: never"),
                ("awaitedArray", "throws: never"),
                ("promised", "throws: never\nrejects: RangeError"),
                ("iterator", "throws: never\non iteration: SyntaxError"),
                ("overload", "throws: TypeError"),
                ("overloadedCaller", "throws: TypeError"),
                ("external", "throws: unknown"),
                ("unknownCaller", "throws: TypeError | unknown"),
                ("callbackCaller", "throws: Throws<typeof callback>"),
                ("callbackPure", "throws: never"),
                ("callbackThrows", "throws: Error"),
                ("callbackNamed", "throws: TypeError"),
                ("callbackForward", "throws: Throws<typeof callback>"),
                ("callbackForwardPure", "throws: never"),
                ("callbackForwardThrows", "throws: TypeError"),
                ("callbackBody", "throws: RangeError | Throws<typeof callback>"),
                ("callbackBodyThrows", "throws: RangeError | TypeError"),
                ("callbackIgnoredThrows", "throws: never"),
                ("callbackHandledThrows", "throws: never"),
                ("callbackSelectPure", "throws: never"),
                ("callbackSelectThrows", "throws: TypeError"),
                ("callbackRecursivePure", "throws: never"),
                ("callbackRecursiveThrows", "throws: TypeError"),
                ("callbackAsyncPure", "throws: never"),
                ("callbackAsyncThrows", "throws: never\nrejects: RangeError"),
                ("callbackDefaultThrows", "throws: TypeError"),
                ("callbackDefaultPure", "throws: never"),
                ("callbackCapturedPure", "throws: never"),
                ("callbackCapturedThrows", "throws: TypeError"),
                ("callbackAliasedPure", "throws: never"),
                ("callbackAliasedThrows", "throws: TypeError"),
                ("callbackTypedThrows", "throws: SyntaxError"),
                ("callbackMultipleThrows", "throws: SyntaxError | TypeError"),
                ("defaults", "throws: TypeError"),
                ("jsdocCaller", "throws: SyntaxError"),
                ("externalPromise", "throws: unknown\nrejects: unknown"),
                ("awaitsExternal", "throws: never\nrejects: unknown"),
                ("awaitsAny", "throws: never\nrejects: unknown"),
                ("switchErrors", "throws: RangeError | TypeError"),
                ("taggedCaller", "throws: SyntaxError"),
                ("instantiatedMethod", "throws: RangeError"),
                ("fieldCaller", "throws: TypeError"),
                ("getterCaller", "throws: unknown"),
                ("method", "throws: RangeError"),
                ("builtinRandom", "throws: never"),
                ("builtinLogging", "throws: never"),
            ];
            let actual: Vec<_> = expected
                .iter()
                .map(|(name, _)| {
                    let node = *functions
                        .iter()
                        .find(|node| node.name().is_some_and(|n| n.text() == *name))
                        .unwrap();
                    (*name, checker.error_effect_display_at(node).unwrap())
                })
                .collect();
            let expected: Vec<_> =
                expected.iter().map(|(name, effect)| (*name, (*effect).to_string())).collect();
            assert_eq!(actual, expected);
            let actual: Vec<_> =
                calls.iter().map(|&call| checker.error_effect_display_at(call).unwrap()).collect();
            assert_eq!(
                actual,
                [
                    "throws: RangeError",
                    "throws: TypeError",
                    "throws: TypeError",
                    "throws: never\nrejects: RangeError",
                    "throws: never",
                    "throws: never"
                ]
            );
            // Native signature queries expose the same metadata, without changing their ordinary return types.
            let signature = checker.get_resolved_signature_exported(calls[0]);
            let effect = checker.error_effect_of_signature(signature);
            assert_eq!(errs::render_effect(&mut *checker, &effect), "throws: RangeError");
            for &call in &calls[4..] {
                let signature = checker.get_resolved_signature_exported(call);
                let effect = checker.error_effect_of_signature(signature);
                assert_eq!(errs::render_effect(&mut *checker, &effect), "throws: never");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
