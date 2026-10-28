export {};
type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends
  (<T>() => T extends B ? 1 : 2) ? true : false;
type Assert<T extends true> = T;

type RandomErrors = Assert<Equal<Throws<typeof Math.random>, never>>;
type NowErrors = Assert<Equal<Throws<typeof Date.now>, never>>;
type IsErrors = Assert<Equal<Throws<typeof Object.is>, never>>;
type FiniteErrors = Assert<Equal<Throws<typeof Number.isFinite>, never>>;
type IntegerErrors = Assert<Equal<Throws<typeof Number.isInteger>, never>>;
type NaNErrors = Assert<Equal<Throws<typeof Number.isNaN>, never>>;
type SafeIntegerErrors = Assert<Equal<Throws<typeof Number.isSafeInteger>, never>>;
type ConsoleErrors = { [K in keyof Console]: Throws<Console[K]> }[keyof Console];
type LoggingErrors = Assert<Equal<ConsoleErrors, never>>;
type UnannotatedErrors = Assert<Equal<Throws<typeof JSON.parse>, unknown>>;

function numeric(): Returns<number, never> {
  const random = Math.random;
  return random() + Date.now();
}
function predicates(value: unknown): Returns<boolean, never> {
  return Number.isFinite(value) || Number.isInteger(value) || Number.isNaN(value)
    || Number.isSafeInteger(value) || Object.is(value, value);
}
function logging(): Returns<void, never> {
  console.assert(false, 'assertion');
  console.clear();
  console.count('count');
  console.countReset('count');
  console.debug('debug');
  console.dir({ value: 1 });
  console.dirxml('dirxml');
  console.error('error');
  console.group('group');
  console.groupCollapsed('collapsed');
  console.groupEnd();
  console.info('info');
  console.log('log');
  console.table([{ value: 1 }]);
  console.time('timer');
  console.timeLog('timer');
  console.timeEnd('timer');
  console.timeStamp('stamp');
  console.trace('trace');
  console.warn('warn');
  const log = console.log;
  log('alias');
}
const number: number = numeric();
const boolean: boolean = predicates(number);
const nothing: void = logging();

// A library annotation must not hide effects from evaluating its arguments.
function fail(): never { throw 'argument'; }
function loggingArgument(): Returns<void, 'argument'> { console.error(fail()); }
function predicateArgument(): Returns<boolean, 'argument'> { return Number.isFinite(fail()); }
type ArgumentErrors = Assert<Equal<Throws<typeof loggingArgument>, 'argument'>>;

// Identically named user methods still use their own implementation or declaration.
namespace Local {
  const Math = { random() { throw 'local'; } };
  const console = { log() { throw 'log'; } };
  type RandomErrors = Assert<Equal<Throws<typeof Math.random>, 'local'>>;
  type LogErrors = Assert<Equal<Throws<typeof console.log>, 'log'>>;
}
