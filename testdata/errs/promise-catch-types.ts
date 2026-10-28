export {};
declare const flag: boolean;
class First extends Error { readonly first = true; }
class Second extends Error { readonly second = true; }
declare const first: First;
declare const second: Second;
async function source() { if (flag) throw first; return 123; }
function next() { if (flag) throw second; return 'value'; }

source().catch(error => {
    const known: First = error;
    // @ts-expect-error The callback parameter is not any.
    error.second;
});
source().then(() => next()).catch(error => {
    const union: First | Second = error;
    // @ts-expect-error The error must be narrowed.
    error.first;
    if (error instanceof First) { const narrowed: First = error; }
    else { const narrowed: Second = error; }
});
source().then(undefined, error => {
    const known: First = error;
    // @ts-expect-error .then rejection handlers use the same inferred type.
    error.second;
});
source().then(value => { const number: number = value; });
const promise = source();
promise.catch(({ first }) => { const known: true = first; });
Promise.reject('failure').catch(error => { const literal: 'failure' = error; });
Promise.resolve(123).catch(error => { const empty: never = error; });
source().catch((error: any) => { error.anything; });
source().catch((error: unknown) => {
    // @ts-expect-error Explicit annotations are preserved.
    error.first;
});
declare const external: Promise<number>;
external.catch(error => {
    // @ts-expect-error Unknown rejection effects stay unknown.
    error.message;
});
source().catch(function (error) { const known: First = error; });
source().catch(((error) => { const known: First = error; }));
function rethrows(): Returns<Promise<void | number>, First> {
    return source().catch(error => { throw error; });
}
function narrows(): Returns<Promise<string | number | void>, Second> {
    return source().then(() => next()).catch(error => {
        if (error instanceof First) return;
        throw error;
    });
}
const fake = { catch(callback: (value: string) => void) { callback('value'); } };
fake.catch(value => { const text: string = value; });
