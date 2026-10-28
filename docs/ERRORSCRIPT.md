# ErrorScript fork

The fork adds opt-in error-effect inference to the native compiler and language service:

```sh
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
target/debug/tsrs --errorscript -p path/to/project --noEmit
target/debug/tsrs --errorscript --lsp -stdio
```

The CLI infers error metadata and checks explicit `Returns<Value, Error>` return contracts. `errorOptions.noUnhandledRootErrors` opts into root-execution diagnostics; `errorOptions.noImplicitReturns` requires return contracts on throwing user-written functions. Ordinary TypeScript value assignability remains separate. Without `--errorscript`, effect checking and effect hovers are disabled; the bundled `Returns` intrinsic still erases to its value argument. The Rust query APIs can also request inference directly without enabling the CLI mode.

## Integration boundary

* `crates/errs`: effect domain, native AST lowering, recursive fixed-point inference, and effect display. Its small `Host` trait asks the checker for type facts; it does not resolve annotations or interpret printed types.
* `crates/errs_config`: dependency-free `ErrorOptions` with optional `no_unhandled_root_errors` and `no_implicit_returns` booleans, carried by a marked field in native compiler options. `tsrs_tsoptions/src/errs.rs` adapts native JSON values, with small hooks for config inheritance and diagnostic invalidation. A private cache marker records that explicit return contracts were checked.
* `crates/tsrs_checker/src/errs/returns.rs`: native intrinsic identification, alias/contextual-signature substitution, exact error-type comparison, and declaration diagnostics. Declarations beside `NoInfer` in `lib.es5.d.ts` expose `Returns` and `Throws`. `errs/throws.rs` implements native `Throws<F>` reduction, deferred operand instantiation, and parameter identity in checker-local side tables, with marked hooks for intrinsic resolution and type serialization. Shared types and signature layouts are unchanged.
* `crates/tsrs_ls/src/errs.rs`: the 98003 quick-fix provider, registered through one marked native code-action hook. It recomputes native return/error types, builds scope-aware type syntax, and uses the ordinary change tracker and LSP edit transport.
* `crates/errs/src/roots.rs`: root execution, exception handlers, and outstanding promise obligations. The checker adapter renders native diagnostic 98001 at the escaping throw/call/chain, shared by CLI and LSP diagnostics.
* `crates/tsrs_checker/src/errs.rs`: native semantic adapter and public queries: `infer_error_effects`, `error_effect_of_node`, `error_effect_of_signature`, `error_effect_at`, and the plain-text wrapper `error_effect_display_at`.
* `crates/errs_ls`: Markdown effect rendering with declaration links from native type/alias symbols and union constituents. A callback uses the host language service's source-map-aware URI and UTF-16 position conversion. Plain-text clients retain the text-only display.
* `Checker::errs_inference`: a clearly marked, checker-owned side table attaching effects to native function declarations and call expressions. Signature queries use their declaration and native generic mapper. No fields are added to shared ASTs, types, or signatures, and no upstream arena layouts change.
* `crates/tsrs_checker/src/errs/catch.rs`: checker-local catch type cache and recursion guard. Demand-driven `Inference::caught` solves the try body's executable dependency closure before native catch typing caches a symbol. Small marked hooks supply the type, mark catch bindings initialized, and break recursive catch queries before native initializer-cycle detection.
* Small hooks in checking, CLI startup, config parsing, and hover invoke these APIs. The CLI's leaf-AST disposal and checker recycling are disabled in this mode because the attachments refer to the entire program.

Native type handles never cross checker boundaries. Each checker infers its own table; forks start with an empty table. A new project program gets new checkers and summaries, including when only a dependency's implementation changes. LSP overlays, resolution, configs, project references, source ownership, and UTF-16 conversions belong to tsrs.

In ErrorScript mode the native options parser enables `allowJs` before root-file discovery and module resolution. `checkJs` remains user-controlled. Unchecked JS bodies still participate in native type queries and error inference. Resolution follows TypeScript: a package resolved to an unavailable `.d.ts` implementation has unknown effects; this version does not independently locate a JS implementation behind that declaration.

## Inferred metadata

Effects keep three channels: synchronous `throws`, promise `rejects`, and generator errors deferred until iteration. Each contains native types plus an independent unknown alternative, so uncertainty does not erase known errors. `never` means no errors found by the currently modelled operations.

Hover error-type links open declaration names using standard Markdown file links. The ErrorScript VS Code extension ships the matching vendored `lib*.d.ts` files and sets `TSRS_LIB_PATH` to their directory; standard-library links and ordinary Go to Definition therefore open real files without installing TypeScript from npm. A standalone embedded server still uses `bundled:///libs` unless its caller provides that override.

Supported starting points include explicit throws, direct and transitive calls, constructors/method bodies, instance-field initializers with explicit constructors, arrows, tagged templates, recursive calls, selected overload implementations, native generic substitution (including instantiated methods), parameter defaults, catch/rethrow, finally replacement, and local return/throw reachability. Async functions turn body throws into rejections; await releases rejections; returning a promise forwards its rejection effects. Immutable local promise bindings retain their initializer's rejection information. Unawaited promises do not become synchronous exceptions of their caller.

Invoked callback parameters contribute native `Throws<typeof callback>` types. At calls to the enclosing function, inference substitutes the actual callback's effects; unused callbacks contribute nothing. This includes forwarding, recursion, captured parameters, immutable callback aliases, omitted callback defaults, and synchronous versus awaited/returned promise effects. Body errors remain part of the union. Reassigned parameters, spread argument bindings, unknown callees, declaration-only functions, and `any`/`unknown` thrown values retain uncertainty. A small host model recognizes standard-library Error constructors with primitive arguments using the actual native library declaration, not the identifier's spelling.

Native standard-library `Promise.then`, `catch`, `finally`, `resolve`, and `reject` calls model handler effects and rejection propagation. A user-defined method merely named `catch` does not count as a promise handler. Static class fields/blocks and base expressions execute in the surrounding scope; instance methods remain deferred.

This is the initial inference layer, not complete JavaScript runtime-effect analysis. Higher-order callable arguments to callbacks, Promise executors/combinators, iterator consumption, precise accessor/proxy/coercion effects, implicit constructors, decorators, dynamic dispatch, and runtime/declaration pairing need further work. Known accessor reads retain an unknown synchronous effect. Branches, loops, and handler reachability in parameter-dependent summaries are conservative. Generic overload implementations with type parameters distinct from the selected overload are not yet fully substituted. Recursive generic growth is bounded to 32 native types per channel and 128 propagation rounds, with an unknown fallback instead of silently dropping effects. Inferred effects are not emitted into `.d.ts` or `.tsbuildinfo`, and impose no general function-effect assignability constraints.

## Inferred catch types

In ErrorScript mode, an unannotated catch binding receives the union of errors inferred to escape its `try` body. Ordinary TypeScript control-flow analysis then handles `instanceof`, `typeof`, discriminants, assignments, and destructuring. Hovers and property diagnostics use the same native type:

```ts
try {
  if (condition) throw new Error('failed');
  throw 'cancelled';
} catch (error) { // Error | "cancelled"
  if (error instanceof Error) {
    error.message; // Error
  } else {
    const reason: 'cancelled' = error;
  }
}
```

Narrowed rethrows contribute their native flow type to inference and `Returns` validation. Nested catches, transitive/generic calls, and awaited rejections participate. An unawaited rejecting promise does not contribute to a synchronous catch. No inferred errors yields `never`; unknown external behavior or an explicit unknown throw yields `unknown`. Recursive dependencies through a catch's own type conservatively fall back to `unknown`.

Explicit `catch (error: any)` and `catch (error: unknown)` retain their annotations. Inferred catch typing applies regardless of `useUnknownInCatchVariables` in ErrorScript mode; ordinary TypeScript mode keeps its original behavior. The scope and uncertainty of the effect model still apply. Catch types belong to the checker snapshot and are recomputed after unsaved dependency edits.

Inline `.catch(error => ...)` callbacks and the second callback of `.then(onFulfilled, error => ...)` receive the receiver's inferred rejection type. This includes errors from previous `.then`/`.catch`/`.finally` handlers and immutable promise aliases. Native narrowing, destructuring, and narrowed rethrows work in these callbacks too. Explicit parameter annotations are preserved, fulfilled-value parameters retain normal TypeScript inference, and user-defined methods named `catch` are unaffected. Named callbacks declared separately retain their own parameter types. The native library's `any` rejection parameter is specialized per inline callback, so even a never-rejecting promise accepts a handler whose inferred parameter is `never`.

## Root error checking

Add this top-level sibling of `compilerOptions` to `tsconfig.json` (in ErrorScript mode):

```json
{
  "errorOptions": {
    "noUnhandledRootErrors": true
  }
}
```

The option is optional and off by default. It inherits through `extends`; explicit `false` overrides an inherited `true`. Invalid non-boolean values and unknown error options are config errors. The extension supplies JSON completion/validation for this section.

With it enabled, executable module/script code must handle inferred throws and promise rejections, including unknown/any effects. Uncalled function bodies do not require local handling. Pure calls (including pure async calls) produce no root errors. Unmodelled external calls such as `console.error`, `console.log`, and `Math.random` currently carry unknown effects and require handling when those effects escape root execution. Any/unknown error types are conservatively represented and reported as `unknown`. A console call inside a catch handler can therefore produce its own root error; a surrounding handler must catch it. Unknown rejections from promise callbacks likewise remain obligations until handled.

```ts
function fail() { throw new Error('failure'); }
async function rejected() { throw new Error('rejection'); }

fail();                          // error 98001
try { fail(); } catch {}          // handled
rejected();                      // error 98001
try { rejected(); } catch {}      // still unhandled: not awaited
rejected().catch(() => {});       // handled
try { await rejected(); } catch {} // handled
rejected().catch(() => fail());   // handler's error still escapes
```

Immutable local promise aliases can be handled in subsequent statements; a conditional handler must cover every analysed branch. `finally` does not consume rejections. Rethrows and failures in handlers remain obligations. Diagnostics use the native checking pipeline (including normal file-checking controls); `--noEmitOnError` sees them like ordinary type errors.

Incremental build information records the checking configuration and a return-contract marker so disabling ErrorScript or its options cannot reuse old effect errors. In ErrorScript mode, semantic diagnostics are recomputed across snapshots: changing a dependency body can change its errors without changing its TypeScript declaration signature.

## Checked return contracts

`Throws<F>` is a native type intrinsic for a callable's error union (synchronous, rejected, and deferred errors). Concrete implementations reduce to their inferred errors; a pure function yields `never`. Generic operands remain deferred and instantiate through ordinary native type mappers. Unavailable implementations yield `unknown`, while bodyless signatures with explicit `Returns` contracts expose their declared error argument. `any`/`unknown` yield `unknown`, non-callable types and `never` yield `never`, and unions distribute. Type aliases, unions, mapped types, and conditional utilities such as `Exclude` can manipulate the result.

For a callback parameter, `Throws<typeof cb>` preserves the parameter's identity. It is also the native type used in inferred summaries and suggested return annotations:

```ts
export function fn(cb: () => void): Returns<void, Throws<typeof cb>> {
  cb();
}

fn(() => {}); // pure
fn(() => { throw new Error('Nope!'); }); // 98001 with noUnhandledRootErrors

type Errors<F> = Throws<F>;
function fail() { throw 'failure'; }
type Failure = Errors<typeof fail>; // "failure"
```

Errors thrown by the wrapper itself must also appear in its contract, for example `Returns<void, Throws<typeof cb> | TypeError>`. Distinct parameters retain distinct identities even when their callable shapes match. `Throws` and `Returns` are recognized by their native default-library symbols; user aliases with those names retain their ordinary meaning.

`Returns<Value, Error>` is a globally available compiler intrinsic alongside `NoInfer`, requiring exactly two type arguments. It behaves as `Value` for normal return checking, calls, generic inference, and value assignability. Its second argument is checked against the implementation's inferred errors in ErrorScript mode, even with both error options disabled:

```ts
function parse(): Returns<number, TypeError> {
  throw new TypeError('invalid input');
}

function wrong(): Returns<number, Error> {
  throw new TypeError('invalid input'); // error 98002: Error is not exactly TypeError
}

function pure(): Returns<number, never> {
  return 123;
}
```

Matching is exact, not an allowed upper bound. Missing and extra union members are errors. Named class/interface identities remain distinct even if structurally identical (including `Error`, `TypeError`, and `RangeError`); type aliases are transparent, and anonymous types use native type identity. `any` does not bypass the check. Inference uncertainty participates in explicit contract checking: an unknown external implementation may require `unknown`; annotations never replace inferred body effects or hide a mismatching callee's actual errors.

The declared error type covers the union of synchronous errors, promise rejections, and deferred generator errors. Both `Returns<Promise<Value>, Error>` and `Promise<Returns<Value, Error>>` work for async functions. Direct annotations, generic/defaulted aliases, contextual arrow/function-expression annotations, object methods, overload return annotations, and native JSDoc return annotations are supported. Each overload is checked against the conservative implementation summary; this does not add input-dependent overload-effect specialization or effect assignability between arbitrary function values. Bodyless declarations cannot be verified and remain subject to the inference model's existing uncertainty rules.

To require these annotations, add a top-level ErrorScript option, distinct from TypeScript's fall-through-return check:

```json
{
  "errorOptions": {
    "noImplicitReturns": true
  }
}
```

This option defaults to off, inherits through `extends`, and accepts an explicit `false` override. A user-written function with known inferred errors (or an explicit throw of an unknown value) without a `Returns` annotation gets error 98003, even if uncalled. Pure functions and unknown external behavior alone do not require annotations. Declaration files and implementations inside `node_modules` are exempt from the annotation requirement. Constructors and setters are exempt because TypeScript does not permit return-type annotations there; their effects still participate in inference. Native JS checking controls apply, with JSDoc providing the return annotation when JS is checked.

Diagnostic 98003 includes the concrete suggested contract, such as `Returns<string, CustomError | Error>` or `Returns<"string", TypeError>` when the native return type is a string literal. The **Add return type Returns<…>** Quick Fix inserts that annotation or replaces an existing ordinary return annotation. It supports functions, methods, getters, arrows (adding parameter parentheses when needed), async/generator signatures, and overload declarations. JS files receive a JSDoc return annotation; existing return documentation is retained. Native type printing qualifies imported types when needed. The fix recomputes inference for the current snapshot and does not act on stale diagnostics or a shadowed `Returns` name. Ordinary TypeScript return inference is preserved, including its normal literal widening.

## Verification

```sh
cargo test -p tsrs_compiler -p tsrs_cli errs_test
cargo test -p tsrs_ls hover_
cargo clippy --all-targets --config 'build.warnings="deny"'
tools/lint/source.mjs
```

`testdata/errs/` contains fork-specific effect fixtures rather than TypeScript reference diagnostic baselines. Compiler tests cover native types, signatures, generics, overloads, recursion, completion handling, async channels, and unchecked JS/JSDoc. Root fixtures cover handling, rethrows, pure/unknown calls, promise chains, immutable aliases, conditional handling, static initialization, and configuration inheritance/disablement. Return-contract fixtures cover exact types/unions, value-type preservation, aliases, generics, async/generator channels, contextual functions, overloads, JSDoc, user/dependency boundaries, option overrides, and incremental mode switches. The CLI integration test runs real LSP processes with the mode on/off and arena poisoning, checks Markdown declaration links (custom types, built-ins, aliases, unions, and rejections), plain-text output, UTF-16 hovers, and edits a JS dependency repeatedly without saving it to verify snapshot invalidation.
