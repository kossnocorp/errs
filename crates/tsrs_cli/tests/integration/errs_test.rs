//! `ErrorScript` LSP protocol fixtures, including snapshot invalidation and UTF-16 hover positions.
use super::lsp_edit_resolutions::{json_string, response};
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};

const SOURCE: &str = "import { failure } from './dependency.js';\nexport function wrapper() { failure(); }\nconst emoji = '😀'; wrapper();\nexport function generic<T>(error: T) { throw error; }\ngeneric(new RangeError('error'));\nclass CustomError extends Error { constructor(message: string) { super(message); } }\nexport function combined(error: CustomError | Error) { throw error; }\nexport async function rejected() { throw new CustomError('error'); }\ntype Failure = CustomError | Error;\nexport function aliased(error: Failure) { throw error; }\n";

#[test]
fn errorscript_hover_tracks_unsaved_js_and_language_modes() {
    for (enabled, markdown) in [(false, true), (true, true), (true, false)] {
        run_lsp(enabled, markdown);
    }
}

#[test]
fn errorscript_native_catch_narrowing() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/errs-catches-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("catches.ts"),
        include_str!("../../../../testdata/errs/catch-types.ts"),
    )
    .unwrap();
    std::fs::write(
        dir.join("promises.ts"),
        include_str!("../../../../testdata/errs/promise-catch-types.ts"),
    )
    .unwrap();
    std::fs::write(
        dir.join("throws.ts"),
        include_str!("../../../../testdata/errs/throws-types.ts"),
    )
    .unwrap();
    for enabled in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
        if enabled {
            command.arg("--errorscript");
        }
        let output = command
            .args([
                "--ignoreConfig",
                "--noEmit",
                "--strict",
                "--target",
                "es2022",
                "--pretty",
                "false",
                "catches.ts",
                "promises.ts",
                "throws.ts",
            ])
            .current_dir(&dir)
            .output()
            .unwrap();
        let text =
            String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap();
        if enabled {
            assert!(output.status.success(), "{text}");
        } else {
            assert!(!output.status.success() && text.contains("TS18046"), "{text}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn errorscript_returns_contracts() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/errs-returns-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases: Vec<_> = include_str!("../../../../testdata/errs/returns-cases.txt")
        .split("// @case ")
        .skip(1)
        .map(|case| {
            let (header, body) = case.split_once('\n').unwrap();
            let parts: Vec<_> = header.split_whitespace().collect();
            let file = format!("{}.ts", parts[0]);
            std::fs::write(
                dir.join(&file),
                format!("export {{}}; declare const flag: boolean;\n{body}"),
            )
            .unwrap();
            (file, parts[1].parse::<usize>().unwrap(), parts[2].parse::<usize>().unwrap())
        })
        .collect();
    std::fs::write(dir.join("base.json"), r#"{"compilerOptions":{"noEmit":true,"strict":true,"target":"es2022","module":"esnext","moduleResolution":"bundler"},"errorOptions":{"noImplicitReturns":true}}"#).unwrap();
    let run = |config: &str, enabled: bool| {
        std::fs::write(dir.join("tsconfig.json"), config).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
        if enabled {
            command.arg("--errorscript");
        }
        let output =
            command.args(["-p", ".", "--pretty", "false"]).current_dir(&dir).output().unwrap();
        let text =
            String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap();
        (output.status.success(), text)
    };
    for required in [true, false] {
        let config = if required {
            r#"{"extends":"./base.json"}"#
        } else {
            r#"{"extends":"./base.json","errorOptions":{"noImplicitReturns":false}}"#
        };
        let (success, text) = run(config, true);
        assert!(!success, "{text}");
        if required {
            assert!(text.contains("Returns<\"string\", TypeError> return annotation"), "{text}");
            assert!(!text.contains("no Returns<Value, Error>"), "{text}");
        }
        assert!(
            text.lines()
                .filter(|line| line.contains("error TS"))
                .all(|line| line.contains("TS98002") || line.contains("TS98003")),
            "{text}"
        );
        for (file, mismatches, missing) in &cases {
            for (code, expected) in
                [(98002, *mismatches), (98003, if required { *missing } else { 0 })]
            {
                let actual = text
                    .lines()
                    .filter(|line| {
                        line.starts_with(&format!("{file}(")) && line.contains(&format!("TS{code}"))
                    })
                    .count();
                assert_eq!(actual, expected, "{file}, code {code}: {text}");
            }
        }
    }
    let (success, text) = run(r#"{"extends":"./base.json"}"#, false);
    assert!(success, "{text}");
    let (success, text) =
        run(r#"{"extends":"./base.json","errorOptions":{"noImplicitReturns":"true"}}"#, true);
    assert!(
        !success && text.contains("errorOptions.noImplicitReturns") && text.contains("boolean"),
        "{text}"
    );
    // Contracts are checked without the opt-in requirement, including across incremental mode switches.
    let config = r#"{"extends":"./base.json","compilerOptions":{"incremental":true},"errorOptions":{"noImplicitReturns":false},"files":["wrong.ts"]}"#;
    let (success, text) = run(config, true);
    assert!(!success && text.contains("TS98002"), "{text}");
    let (success, text) = run(config, false);
    assert!(success, "{text}");
    std::fs::write(
        dir.join("value.ts"),
        "function value(): Returns<number, never> { return 'incorrect'; }",
    )
    .unwrap();
    let (success, text) = run(r#"{"extends":"./base.json","files":["value.ts"]}"#, true);
    assert!(!success && text.contains("TS2322") && !text.contains("TS98002"), "{text}");
    std::fs::write(dir.join("value.ts"), "function value(): Returns<number> { return 123; }")
        .unwrap();
    let (success, text) = run(r#"{"extends":"./base.json","files":["value.ts"]}"#, true);
    assert!(!success && text.contains("requires 2 type argument"), "{text}");
    std::fs::write(dir.join("annotated.js"), "/** @returns {Returns<void, TypeError>} */\nexport function annotated() { throw new TypeError('x'); }\n").unwrap();
    let (success, text) = run(
        r#"{"extends":"./base.json","compilerOptions":{"checkJs":true},"files":["annotated.js"]}"#,
        true,
    );
    assert!(success, "{text}");
    std::fs::write(
        dir.join("annotated.js"),
        "export function missing() { throw new TypeError('x'); }\n",
    )
    .unwrap();
    let (success, text) = run(
        r#"{"extends":"./base.json","compilerOptions":{"checkJs":true},"files":["annotated.js"]}"#,
        true,
    );
    assert!(!success && text.contains("TS98003"), "{text}");
    std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    std::fs::write(
        dir.join("node_modules/pkg/index.ts"),
        "export function dependency() { throw new TypeError('x'); }",
    )
    .unwrap();
    std::fs::write(dir.join("vendor.ts"), "import { dependency } from 'pkg'; dependency();")
        .unwrap();
    let (success, text) = run(r#"{"extends":"./base.json","files":["vendor.ts"]}"#, true);
    assert!(success, "{text}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn errorscript_root_errors_and_config_inheritance() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/errs-roots-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("functions.ts"),
        include_str!("../../../../testdata/errs/roots-functions.ts"),
    )
    .unwrap();
    let cases: Vec<_> = include_str!("../../../../testdata/errs/roots-cases.txt")
        .split("// @case ").skip(1).map(|case| {
            let (header, source) = case.split_once('\n').unwrap();
            let (name, count) = header.split_once(' ').unwrap();
            let file = format!("{name}.ts");
            let source = format!("import {{ fail, rejected, pure, pureAsync, external, externalPromise, throwAny, rejectAny }} from './functions';\n{source}");
            std::fs::write(dir.join(&file), source).unwrap();
            (file, count.parse::<usize>().unwrap())
        }).collect();
    std::fs::write(dir.join("base.json"), r#"{"compilerOptions":{"noEmit":true,"strict":true,"target":"es2022","module":"esnext","moduleResolution":"bundler"},"errorOptions":{"noUnhandledRootErrors":true}}"#).unwrap();
    let run = |config: &str, enabled: bool| {
        std::fs::write(dir.join("tsconfig.json"), config).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
        if enabled {
            command.arg("--errorscript");
        }
        let output =
            command.args(["-p", ".", "--pretty", "false"]).current_dir(&dir).output().unwrap();
        let text =
            String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap();
        (output.status.success(), text)
    };
    let (success, text) = run(r#"{"extends":"./base.json"}"#, true);
    assert!(!success, "{text}");
    assert!(
        text.lines().filter(|line| line.contains("error TS")).all(|line| line.contains("TS98001")),
        "{text}"
    );
    for (file, expected) in cases {
        let actual = text
            .lines()
            .filter(|line| line.starts_with(&format!("{file}(")) && line.contains("TS98001"))
            .count();
        assert_eq!(actual, expected, "{file}: {text}");
    }
    for (config, enabled) in [
        (r#"{"extends":"./base.json","errorOptions":{"noUnhandledRootErrors":false}}"#, true),
        (r#"{"extends":"./base.json"}"#, false),
        (
            r#"{"compilerOptions":{"noEmit":true,"target":"es2022","module":"esnext","moduleResolution":"bundler"}}"#,
            true,
        ),
    ] {
        let (success, text) = run(config, enabled);
        assert!(success, "{text}");
    }
    let (success, text) =
        run(r#"{"extends":"./base.json","errorOptions":{"noUnhandledRootErrors":"true"}}"#, true);
    assert!(
        !success && text.contains("errorOptions.noUnhandledRootErrors") && text.contains("boolean"),
        "{text}"
    );
    // A body-only change keeps the exported TS signature, but must invalidate dependent root diagnostics.
    std::fs::write(dir.join("incremental.ts"), "import { fail } from './callee'; fail();").unwrap();
    let throwing = "export function fail(): void { throw new TypeError('error'); }";
    std::fs::write(dir.join("callee.ts"), throwing).unwrap();
    let incremental = r#"{"extends":"./base.json","compilerOptions":{"incremental":true},"files":["incremental.ts"]}"#;
    let (success, text) = run(incremental, true);
    assert!(!success && text.contains("TS98001"), "{text}");
    std::fs::write(dir.join("callee.ts"), "export function fail(): void {}").unwrap();
    let (success, text) = run(incremental, true);
    assert!(success, "{text}");
    std::fs::write(dir.join("callee.ts"), throwing).unwrap();
    let (success, text) = run(incremental, true);
    assert!(!success && text.contains("TS98001"), "{text}");
    let (success, text) = run(incremental, false);
    assert!(success, "{text}");
    std::fs::remove_dir_all(dir).unwrap();
}

fn run_lsp(enabled: bool, markdown: bool) {
    let source = format!(
        "{SOURCE}export function callback(cb: () => void): Returns<void, Throws<typeof cb>> {{ cb(); }}\ncallback(() => {{}});\ncallback(() => {{ throw new Error('Nope!'); }});\n"
    );
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/errs-lsp-{}-{enabled}-{markdown}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{"compilerOptions":{"noEmit":true,"allowJs":false,"checkJs":false,"strict":true,"target":"es2022","module":"nodenext"},"files":["index.ts"]}"#).unwrap();
    std::fs::write(dir.join("index.ts"), &source).unwrap();
    let dependency = "export function failure() { throw new TypeError('original'); }";
    std::fs::write(dir.join("dependency.js"), dependency).unwrap();
    let root =
        format!("file://{}", dir.canonicalize().unwrap().to_string_lossy().replace('\\', "/"));
    let uri = format!("{root}/index.ts");
    let js_uri = format!("{root}/dependency.js");
    let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    if enabled {
        command.arg("--errorscript");
    }
    let mut child = command
        .args(["--lsp", "-stdio"])
        .env("TSRS_ARENA_POISON", "1")
        .env(
            "TSRS_LIB_PATH",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tsrs_vfs/libs")
                .canonicalize()
                .unwrap(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut send = |body: String| {
        write!(input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        input.flush().unwrap();
    };
    let format = if markdown { "markdown" } else { "plaintext" };
    send(format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"processId":null,"rootUri":"{root}","capabilities":{{"textDocument":{{"hover":{{"contentFormat":["{format}"]}},"diagnostic":{{}}}}}}}}}}"#
    ));
    response(&mut output, 1, &mut send);
    send(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_string());
    send(format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"errorscript","version":1,"text":{}}}}}}}"#,
        json_string(&source)
    ));
    let mut id = 1;
    for (line, character, expected) in [
        (1, 16, "TypeError"),                                            // declaration
        (2, "const emoji = '😀'; ".encode_utf16().count(), "TypeError"), // call following a surrogate pair
        (4, 2, "RangeError"),   // generic selected signature
        (6, 16, "CustomError"), // native union constituents
        (7, 23, "CustomError"), // rejection channel
        (9, 16, "Failure"),     // native alias declaration
    ] {
        id += 1;
        send(format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":{line},"character":{character}}}}}}}"#
        ));
        let hover = response(&mut output, id, &mut send);
        assert!(hover.contains("contents"), "{hover}");
        assert_eq!(hover.contains("ErrorScript"), enabled, "{hover}");
        if enabled {
            let label =
                if markdown { format!("[`{expected}`](<file://") } else { expected.to_string() };
            assert!(hover.contains(&label), "{hover}");
            if markdown {
                if expected == "CustomError" {
                    assert!(hover.contains(&format!("[`CustomError`](<{uri}#L6,7>)")), "{hover}");
                }
                if line == 6 {
                    assert!(hover.contains("[`Error`](<file://"), "{hover}");
                    assert!(hover.contains("/lib.es5.d.ts#L"), "{hover}");
                }
                if expected == "Failure" {
                    assert!(hover.contains(&format!("[`Failure`](<{uri}#L9,6>)")), "{hover}");
                }
            } else {
                assert!(!hover.contains("](<"), "{hover}");
            }
        }
    }
    for (line, expected) in [(10, "Throws<typeof cb>"), (11, "never"), (12, "Error")] {
        id += 1;
        send(format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":{line},"character":{}}}}}}}"#,
            if line == 10 { 16 } else { 2 }
        ));
        let hover = response(&mut output, id, &mut send);
        assert_eq!(hover.contains("ErrorScript"), enabled, "{hover}");
        if enabled {
            let expected = if !markdown {
                format!("throws: {expected}")
            } else if line == 12 {
                "throws: [`Error`](<file://".to_string()
            } else {
                format!("throws: `{expected}`")
            };
            assert!(hover.contains(&expected), "{hover}");
        }
    }
    if enabled {
        send(format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{js_uri}","languageId":"javascript","version":1,"text":{}}}}}}}"#,
            json_string(dependency)
        ));
        for (version, error) in [(2, "SyntaxError"), (3, "RangeError"), (4, "TypeError")] {
            let edit = format!("export function failure() {{ throw new {error}('unsaved'); }}");
            send(format!(
                r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{js_uri}","version":{version}}},"contentChanges":[{{"text":{}}}]}}}}"#,
                json_string(&edit)
            ));
            id += 1;
            send(format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":1,"character":16}}}}}}"#
            ));
            let hover = response(&mut output, id, &mut send);
            let expected = if markdown {
                format!("throws: [`{error}`](<file://")
            } else {
                format!("throws: {error}")
            };
            assert!(hover.contains(&expected), "{hover}");
        }
        id += 1;
        send(format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/diagnostic","params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
        ));
        let diagnostics = response(&mut output, id, &mut send);
        assert!(!diagnostics.contains("\"severity\":1"), "{diagnostics}");
    }
    id += 1;
    send(format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"shutdown"}}"#));
    response(&mut output, id, &mut send);
    send(r#"{"jsonrpc":"2.0","method":"exit"}"#.to_string());
    drop(input);
    child.wait().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
