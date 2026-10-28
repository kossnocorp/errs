export {};
type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends
  (<T>() => T extends B ? 1 : 2) ? true : false;
type Assert<T extends true> = T;

function fail() { throw 'failure'; }
function other() { throw 'other'; }
function pure() {}
async function rejected() { throw 'rejection'; }
function* deferred() { yield 1; throw 'iteration'; }

type Direct = Assert<Equal<Throws<typeof fail>, 'failure'>>;
type Pure = Assert<Equal<Throws<typeof pure>, never>>;
type Async = Assert<Equal<Throws<typeof rejected>, 'rejection'>>;
type Generator = Assert<Equal<Throws<typeof deferred>, 'iteration'>>;
type Errors<F> = Throws<F>;
type Aliased = Assert<Equal<Errors<typeof fail>, 'failure'>>;
type Union = Assert<Equal<Errors<typeof fail | typeof other>, 'failure' | 'other'>>;
type NonCallable = Assert<Equal<Throws<number>, never>>;
type Bottom = Assert<Equal<Throws<never>, never>>;
type Uncertain = Assert<Equal<Throws<any>, unknown>>;
type Declared = Assert<Equal<Throws<() => Returns<void, 'declared'>>, 'declared'>>;
type Filtered = Assert<Equal<Exclude<Errors<typeof fail | typeof other>, 'other'>, 'failure'>>;
type Mapped<F> = { [K in keyof F]: Throws<F[K]> };
type Manipulated = Assert<Equal<Mapped<{ a: typeof fail; b: typeof pure }>, { a: 'failure'; b: never }>>;
const typed: () => void = () => { throw 'typed'; };
type AnnotatedValue = Assert<Equal<Throws<typeof typed>, 'typed'>>;

export function fn(cb: () => void): Returns<void, Throws<typeof cb>> { cb(); }
export function caught(cb: () => void): Returns<void, never> {
  try { cb(); } catch (error) {
    const sameNativeType: Throws<typeof cb> = error;
  }
}
export function forward(cb: () => void): Returns<void, Throws<typeof cb>> { fn(cb); }
export function body(cb: () => void): Returns<void, Throws<typeof cb> | 'body'> {
  cb();
  throw 'body';
}
export function generic<F extends () => void>(cb: F): Returns<void, Throws<F>> { cb(); }
export function selected(a: () => void, b: () => void): Returns<void, Throws<typeof b>> { b(); }
export async function asyncCallback(cb: () => Promise<void>): Returns<Promise<void>, Throws<typeof cb>> {
  await cb();
}
try { fn(() => {}); } catch (error) { const value: never = error; }
try { fn(fail); } catch (error) { const value: 'failure' = error; }
try { generic(fail); } catch (error) { const value: 'failure' = error; }

// A user alias with the same spelling is not the intrinsic.
namespace Local {
  type Throws<F> = F;
  type Shadowed = Assert<Equal<Throws<number>, number>>;
}
