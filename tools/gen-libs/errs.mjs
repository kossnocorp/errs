// Fork-specific declarations, applied after copying the pinned TypeScript libraries.
import { requireValue } from "../lib/cli.mjs";

const consoleMethods = [
    "assert", "clear", "count", "countReset", "debug", "dir", "dirxml", "error",
    "group", "groupCollapsed", "groupEnd", "info", "log", "table", "time", "timeEnd",
    "timeLog", "timeStamp", "trace", "warn",
];
const annotations = {
    "lib.es5.d.ts": { Math: ["random"], DateConstructor: ["now"] },
    "lib.es2015.core.d.ts": {
        NumberConstructor: ["isFinite", "isInteger", "isNaN", "isSafeInteger"],
        ObjectConstructor: ["is"],
    },
    "lib.dom.d.ts": { Console: consoleMethods },
    "lib.webworker.d.ts": { Console: consoleMethods },
};

export function errorscriptLibrary(name, source) {
    if (name === "lib.es5.d.ts") {
        const marker = "type NoInfer<T> = intrinsic;";
        requireValue(source.includes(marker), `${name}: missing NoInfer intrinsic`);
        source = source.replace(marker, `${marker}

/** ErrorScript: return value type with an exactly checked inferred error type. */
type Returns<ReturnType, ErrorType> = intrinsic;

/** ErrorScript: inferred error types of a callable, deferred for callback parameters and generics. */
type Throws<F> = intrinsic;`);
    }
    for (const [owner, methods] of Object.entries(annotations[name] ?? {})) {
        const pattern = new RegExp(`^interface ${owner} \\{\\n[\\s\\S]*?^\\}`, "m");
        requireValue(pattern.test(source), `${name}: missing interface ${owner}`);
        source = source.replace(pattern, (body) => {
            for (const method of methods) {
                const signature = new RegExp(`^(    ${method}\\([^\\n]*\\): )(number|boolean|void);$`, "gm");
                const matches = [...body.matchAll(signature)];
                requireValue(matches.length === 1, `${name}: expected one ${owner}.${method} signature`);
                body = body.replace(signature, "$1Returns<$2, never>;");
            }
            return body;
        });
    }
    return source;
}
