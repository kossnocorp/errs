import { javascriptFailure, jsdocFailure } from './dependency.js';

export function direct() { throw new TypeError('direct'); }
export const arrow = () => { throw new RangeError('arrow'); };
export function transitive() { direct(); arrow(); javascriptFailure(); }
export function caught() { try { direct(); } catch {} }
export function rethrown() { try { direct(); } catch (error) { throw error; } }
export function replaced() { try { direct(); } finally { throw new SyntaxError('replacement'); } }
export function suppressed() { try { direct(); } finally { return; } }
export function unreachable() { return; throw new Error('unreachable'); }
export function nested() { const unused = () => { throw new Error('deferred'); }; }
export function generic<T>(error: T): never { throw error; }
export function genericCaller() { generic(new RangeError('instantiated')); }
export function genericWrapper<T>(error: T) { generic(error); }
export function genericTransitive() { genericWrapper(new SyntaxError('transitive')); }
export function even(n: number): void { if (n) odd(n - 1); else throw new TypeError('even'); }
export function odd(n: number): void { even(n - 1); }
export async function rejection() { throw new RangeError('async'); }
export async function awaited() { await rejection(); }
export function unawaited() { rejection(); }
export async function caughtRejection() { try { await rejection(); } catch {} }
export async function returnedRejection() { return rejection(); }
export async function storedRejection() { const pending = rejection(); await pending; }
export async function promiseArray() { return [rejection()]; }
export async function awaitedArray() { await [rejection()]; }
export function promised() { return rejection(); }
export function* iterator() { throw new SyntaxError('deferred'); }
export function overload(value: string): void;
export function overload(value: number): void;
export function overload(value: string | number) { throw new TypeError('overload'); }
export function overloadedCaller() { overload('test'); }
export declare function external(): void;
export function unknownCaller() { external(); direct(); }
export function callbackCaller(callback: () => void) { callback(); }
export function callbackPure() { callbackCaller(() => {}); }
export function callbackThrows() { callbackCaller(() => { throw new Error('Nope!'); }); }
export function callbackNamed() { callbackCaller(direct); }
export function callbackForward(callback: () => void) { callbackCaller(callback); }
export function callbackForwardPure() { callbackForward(() => {}); }
export function callbackForwardThrows() { callbackForward(direct); }
export function callbackBody(callback: () => void) { callback(); throw new RangeError('body'); }
export function callbackBodyThrows() { callbackBody(direct); }
export function callbackIgnored(callback: () => void) {}
export function callbackIgnoredThrows() { callbackIgnored(direct); }
export function callbackHandled(callback: () => void) { try { callback(); } catch {} }
export function callbackHandledThrows() { callbackHandled(direct); }
export function callbackSelect(first: () => void, second: () => void) { second(); }
export function callbackSelectPure() { callbackSelect(direct, () => {}); }
export function callbackSelectThrows() { callbackSelect(() => {}, direct); }
export function callbackRecursive(callback: () => void, n: number): void {
  if (n) callbackRecursive(callback, n - 1); else callback();
}
export function callbackRecursivePure() { callbackRecursive(() => {}, 2); }
export function callbackRecursiveThrows() { callbackRecursive(direct, 2); }
export async function callbackAsync(callback: () => Promise<void>) { await callback(); }
export function callbackAsyncPure() { return callbackAsync(async () => {}); }
export function callbackAsyncThrows() { return callbackAsync(rejection); }
export function callbackDefault(callback: () => void = direct) { callback(); }
export function callbackDefaultThrows() { callbackDefault(); }
export function callbackDefaultPure() { callbackDefault(() => {}); }
export function callbackCaptured(callback: () => void) { callbackCaller(() => callback()); }
export function callbackCapturedPure() { callbackCaptured(() => {}); }
export function callbackCapturedThrows() { callbackCaptured(direct); }
export function callbackAliased(callback: () => void) { const alias = callback; alias(); }
export function callbackAliasedPure() { callbackAliased(() => {}); }
export function callbackAliasedThrows() { callbackAliased(direct); }
const typedCallback: () => void = () => { throw new SyntaxError('typed callback'); };
export function callbackTypedThrows() { callbackCaller(typedCallback); }
export function callbackMultiple(first: () => void, second: () => void) { first(); second(); }
export function callbackMultipleThrows() { callbackMultiple(direct, typedCallback); }
export function defaults(value = direct()) {}
export function jsdocCaller() { jsdocFailure(new SyntaxError('JSDoc')); }
export declare function externalPromise(): Promise<void>;
export async function awaitsExternal() { await externalPromise(); }
export async function awaitsAny(value: any) { await value; }
export function switchErrors(value: number) {
  switch (value) {
    case 0: return;
    case 1: throw new TypeError('case one');
    default: throw new RangeError('default');
  }
}
export function tag(strings: TemplateStringsArray) { throw new SyntaxError('tag'); }
export function taggedCaller() { tag`template`; }
export class GenericBox<T> {
  constructor(readonly error: T) {}
  raise() { throw this.error; }
}
export function instantiatedMethod(box: GenericBox<RangeError>) { box.raise(); }
export class WithField {
  field = direct();
  constructor() {}
}
export function fieldCaller() { new WithField(); }
export function getterCaller() {
  const object = { get value() { throw new TypeError('getter'); } };
  return object.value;
}
export class Methods {
  method() { throw new RangeError('method'); }
  constructor() { throw new TypeError('constructor'); }
}
generic(new RangeError('call'));
direct();
overload('call');
rejection();
