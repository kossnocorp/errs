export function fail() { throw new TypeError('failure'); }
export async function rejected() { throw new RangeError('rejection'); }
export function pure() { return 123; }
export async function pureAsync() { return 123; }
export declare function external(): void;
export declare function externalPromise(): Promise<void>;
export function throwAny(value: any) { throw value; }
export async function rejectAny(value: any) { throw value; }
