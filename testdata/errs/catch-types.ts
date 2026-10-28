export {};
declare const flag: boolean;
class ParseError extends Error { readonly parse = true; }
class NetworkError extends Error { readonly network = true; }
declare const parse: ParseError;
declare const network: NetworkError;

function source() { if (flag) throw parse; throw network; }
function transitive() { source(); }
function generic<E>(error: E) { throw error; }

try { transitive(); } catch (error) {
    const union: ParseError | NetworkError = error;
    // @ts-expect-error The catch argument is a union, not any.
    error.parse;
    if (error instanceof ParseError) {
        const narrowed: ParseError = error;
        error.parse;
        // @ts-expect-error Native narrowing excludes NetworkError.
        error.network;
    } else {
        const narrowed: NetworkError = error;
        error.network;
    }
}

function narrowedRethrow(): Returns<void, NetworkError> {
    try { source(); } catch (error) {
        if (error instanceof ParseError) return;
        throw error;
    }
}
try { narrowedRethrow(); } catch (error) { const narrowed: NetworkError = error; }
try { generic(parse); } catch (error) { const genericError: ParseError = error; }
function genericCatch<T>(value: T) {
    try { throw value; } catch (error) { const genericError: T = error; }
}
function reassigned(): Returns<void, NetworkError> {
    try { source(); } catch (error) { error = network; throw error; }
}
function captured(): Returns<void, ParseError | NetworkError> {
    try { source(); } catch (error) {
        const rethrow = () => { throw error; };
        rethrow();
    }
}
try { throw 'literal'; } catch (error) { const literal: 'literal' = error; }
try { if (flag) throw 'text'; throw 123; } catch (error) {
    if (typeof error === 'string') { const text: 'text' = error; }
    else { const number: 123 = error; }
}
type Tagged = { kind: 'parse'; line: number } | { kind: 'network'; status: number };
declare const tagged: Tagged;
try { throw tagged; } catch (error) {
    if (error.kind === 'parse') error.line;
    else error.status;
}
try { throw { message: 'failure', code: 123 }; } catch ({ message, code }) {
    const text: string = message;
    const number: number = code;
}
try { const value = 123; } catch (error) { const empty: never = error; }
try { try { throw parse; } catch {} } catch (error) { const empty: never = error; }
try { try { throw parse; } finally { throw network; } } catch (error) {
    const finalizer: NetworkError = error;
}
try {
    try { source(); } catch (error) {
        if (error instanceof ParseError) {
            try { throw error; } catch (inner) { const outerBinding: ParseError = inner; }
        } else { throw error; }
    }
} catch (error) { const nested: NetworkError = error; }

async function rejects() { throw parse; }
async function asyncCatch() {
    try { await rejects(); } catch (error) { const awaited: ParseError = error; }
    try { rejects(); } catch (error) { const synchronous: never = error; }
    const promise = rejects();
    try { await promise; } catch (error) { const alias: ParseError = error; }
    try { await Promise.reject(network); } catch (error) { const rejected: NetworkError = error; }
    try { await rejects().catch(() => { throw network; }); }
    catch (error) { const handler: NetworkError = error; }
    try { await Promise.resolve().then(() => { throw parse; }).finally(() => {}).then(() => 123); }
    catch (error) { const chain: ParseError = error; }
}

declare function external(): void;
try { external(); } catch (error) {
    // @ts-expect-error Unknown implementations stay unknown.
    error.message;
    if (error instanceof Error) error.message;
}
declare const unknownValue: unknown;
try { throw unknownValue; } catch (error) {
    // @ts-expect-error Explicit unknown throws stay unknown.
    const known: Error = error;
}
try { throw parse; } catch (error: unknown) {
    // @ts-expect-error Explicit unknown annotation is preserved.
    error.parse;
}
try { throw parse; } catch (error: any) { error.anything; }
try { throw parse; } catch (error) {
    // @ts-expect-error Assignments must satisfy the inferred catch type.
    error = network;
}
function recursive(n: number): void { if (n) recursive(n - 1); else throw parse; }
try { recursive(2); } catch (error) { const recursion: ParseError = error; }
function cycle(): void {
    try { cycle(); } catch (error) {
        // @ts-expect-error A handler/type inference cycle stays conservative.
        error.message;
        throw error;
    }
}
// Callback effects must also specialize the demand-driven catch inference closure.
function invokeCallback(cb: () => void) { cb(); }
try { invokeCallback(() => { throw new Error('callback'); }); } catch (error) {
  const inferred: Error = error;
  error.message;
}
try { invokeCallback(() => {}); } catch (error) {
  const inferred: never = error;
}
