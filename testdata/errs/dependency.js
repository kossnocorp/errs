export function javascriptFailure() { throw new SyntaxError('JavaScript'); }
/** @param {SyntaxError} error */
export function jsdocFailure(error) { throw error; }
