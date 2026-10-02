//! The D32 spike's evidence: the context and literals of known call sites
//! in each provider fixture, the UTF-16 join, and the grammars' ABI.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use super::Control::{Await, Conditional, Defer, Goroutine, Loop, Try};
use super::*;

fn fixture(path: &str) -> Vec<u8> {
    let file: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "tests",
        "fixtures",
        path,
    ]
    .iter()
    .collect();
    // A Windows checkout may have CRLF line ends: columns before the `\r`
    // are unchanged, and tree-sitter parses either.
    std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

/// The UTF-8 column of `needle` on 1-based `line` of `source`, as a
/// 0-based SCIP position (the fixtures' lines are ASCII, where UTF-8 and
/// UTF-16 columns agree).
fn position(source: &[u8], line: u32, needle: &str) -> (u32, u32) {
    let text = std::str::from_utf8(source).unwrap();
    let row = text.lines().nth(line as usize - 1).unwrap();
    let column = row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not on line {line}: {row:?}"));
    (line - 1, u32::try_from(column).unwrap())
}

/// A site's call kind, context and literals.
type Expected<'a> = (
    u32,
    &'a str,
    Option<&'static str>,
    &'a [Control],
    &'a [&'a str],
);

fn check(language: Language, path: &str, source: &[u8], sites: &[Expected<'_>]) {
    let tree = parse(language, source).unwrap();
    assert!(!tree.root_node().has_error(), "{path} parses cleanly");
    for &(line, needle, call, control, literals) in sites {
        let (row, column) = position(source, line, needle);
        let syntax = site_syntax_at(language, &tree, source, row, column, Encoding::Utf8)
            .unwrap_or_else(|| panic!("{path}:{line} {needle:?}: no node starts there"));
        assert_eq!(
            syntax,
            SiteSyntax {
                call,
                control: control.to_vec(),
                literals: literals.iter().map(ToString::to_string).collect(),
            },
            "{path}:{line} {needle:?}"
        );
    }
}

#[test]
fn rust_fixture_sites() {
    let pipeline = fixture("rust/src/pipeline.rs");
    check(
        Language::Rust,
        "pipeline.rs",
        &pipeline,
        &[
            (10, "apply_dyn", Some("call_expression"), &[Loop], &[]),
            (11, "record", Some("call_expression"), &[Loop], &[]),
            (15, "transform", Some("call_expression"), &[], &[]),
            (20, "apply", Some("call_expression"), &[], &[]),
            // `map(double)`: a function used as a value is not called here.
            (30, "double", None, &[], &[]),
        ],
    );
    let lib = fixture("rust/src/lib.rs");
    check(
        Language::Rust,
        "lib.rs",
        &lib,
        &[(22, "log_line", Some("macro_invocation"), &[], &["settled"])],
    );
}

#[test]
fn go_fixture_sites() {
    let main = fixture("go/cmd/shop/main.go");
    check(
        Language::Go,
        "main.go",
        &main,
        &[
            (13, "Settle", Some("call_expression"), &[Loop], &[]),
            (13, "Println", Some("call_expression"), &[Loop], &[]),
            (18, "charge", Some("call_expression"), &[], &[]),
            (21, "apply", Some("call_expression"), &[], &[]),
            // Inside `go func() { ... }()`: through the function literal.
            (25, "Sum", Some("call_expression"), &[Goroutine], &[]),
        ],
    );
}

#[test]
fn typescript_fixture_sites() {
    let checkout = fixture("ts/src/checkout.ts");
    check(
        Language::TypeScript,
        "checkout.ts",
        &checkout,
        &[
            (9, "amount", Some("call_expression"), &[Loop], &[]),
            // Inside an arrow function passed to `map`.
            (15, "describe", Some("call_expression"), &[], &["item"]),
            (19, "map", Some("call_expression"), &[], &[]),
            (19, "step", None, &[], &[]),
        ],
    );
    let main = fixture("ts/src/main.ts");
    check(
        Language::TypeScript,
        "main.ts",
        &main,
        &[
            (9, "dispatch", Some("call_expression"), &[], &["card"]),
            (9, "Card", Some("new_expression"), &[], &[]),
        ],
    );
    let legacy = fixture("ts/src/legacy.js");
    check(
        Language::TypeScript,
        "legacy.js",
        &legacy,
        &[(6, "amount", Some("call_expression"), &[], &[])],
    );
}

#[test]
fn python_fixture_sites() {
    let worker = fixture("python/shop/worker.py");
    check(
        Language::Python,
        "worker.py",
        &worker,
        &[
            // A list comprehension's element runs per item; `with` is not
            // control context.
            (20, "submit", Some("call"), &[Loop], &[]),
            (20, "process", None, &[Loop], &[]),
            (21, "result", Some("call"), &[Loop], &[]),
            (26, "send", Some("call"), &[], &[]),
        ],
    );
    let main = fixture("python/shop/__main__.py");
    check(
        Language::Python,
        "__main__.py",
        &main,
        &[
            // The inner loop's iterable runs once per outer iteration.
            (14, "run", Some("call"), &[Loop], &[]),
            (15, "notify", Some("call"), &[Loop, Loop], &[]),
            (15, "Printer", Some("call"), &[Loop, Loop], &[]),
            (26, "main", Some("call"), &[Conditional], &[]),
        ],
    );
}

#[test]
fn go_conditionals_defers_and_literals() {
    let source = b"package main\n\
        \n\
        func f(bus Bus, xs []int) {\n\
        \tfor _, x := range xs {\n\
        \t\tif ok(x) {\n\
        \t\t\tbus.Publish(\"orders.created\", `raw`, x)\n\
        \t\t}\n\
        \t}\n\
        \tdefer bus.Close(\"done\")\n\
        }\n";
    check(
        Language::Go,
        "inline.go",
        source,
        &[
            (5, "ok", Some("call_expression"), &[Loop], &[]),
            (
                6,
                "Publish",
                Some("call_expression"),
                &[Conditional, Loop],
                &["orders.created", "raw"],
            ),
            (9, "Close", Some("call_expression"), &[Defer], &["done"]),
        ],
    );
}

#[test]
fn typescript_try_await_and_templates() {
    let source = b"async function f(bus: any, ok: boolean) {\n\
        \x20 try {\n\
        \x20   await bus.send(`orders`, `x${ok}`, 'events');\n\
        \x20 } catch (e) {\n\
        \x20   bus.log(\"failed\");\n\
        \x20 }\n\
        \x20 return ok ? bus.a() : bus.b();\n\
        }\n";
    check(
        Language::TypeScript,
        "inline.ts",
        source,
        &[
            (
                3,
                "send",
                Some("call_expression"),
                &[Await, Try],
                &["orders", "events"],
            ),
            (5, "log", Some("call_expression"), &[], &["failed"]),
            (7, "a", Some("call_expression"), &[Conditional], &[]),
        ],
    );
}

#[test]
fn python_try_await_and_fstrings() {
    let source = b"async def f(bus, ok):\n\
        \x20   try:\n\
        \x20       await bus.send(\"orders\", f\"x{ok}\", r'raw')\n\
        \x20   except Exception:\n\
        \x20       bus.log('failed')\n\
        \x20   return bus.a() if ok else None\n";
    check(
        Language::Python,
        "inline.py",
        source,
        &[
            (3, "send", Some("call"), &[Await, Try], &["orders", "raw"]),
            (5, "log", Some("call"), &[], &["failed"]),
            (6, "a", Some("call"), &[Conditional], &[]),
        ],
    );
}

#[test]
fn rust_await_and_raw_strings() {
    let source = b"async fn f(bus: Bus) {\n\
        \x20   if ready() {\n\
        \x20       bus.send(r#\"orders\"#, \"x\").await;\n\
        \x20   }\n\
        }\n";
    check(
        Language::Rust,
        "inline.rs",
        source,
        &[
            (2, "ready", Some("call_expression"), &[], &[]),
            (
                3,
                "send",
                Some("call_expression"),
                &[Await, Conditional],
                &["orders", "x"],
            ),
        ],
    );
}

/// Columns scip-typescript 0.4.0 and scip-python 0.6.6 wrote for `go` on
/// these lines (V162): UTF-16 code units. In UTF-8 bytes `go` starts at 35
/// and 22.
#[test]
fn utf16_columns_join_to_the_same_name() {
    let ts = "export function go(s: string): string { return s; }\n\
              export function f(): string {\n  return \"\\u{1F600}\" + (\"😀é\" + go(\"topic\"));\n}\n";
    let tree = parse(Language::TypeScript, ts.as_bytes()).unwrap();
    let at = |column, encoding| {
        site_syntax_at(
            Language::TypeScript,
            &tree,
            ts.as_bytes(),
            2,
            column,
            encoding,
        )
    };
    let syntax = at(32, Encoding::Utf16).unwrap();
    assert_eq!(syntax.call, Some("call_expression"));
    assert_eq!(syntax.literals, ["topic"]);
    assert_eq!(at(35, Encoding::Utf8), Some(syntax));
    assert_ne!(
        at(32, Encoding::Utf8).and_then(|s| s.call),
        Some("call_expression"),
        "read as bytes, column 32 is inside the string, not at `go`"
    );

    let py = "def go(s):\n    return s\n\n\ndef f():\n    return (\"😀é\", go(\"topic\"))\n";
    let tree = parse(Language::Python, py.as_bytes()).unwrap();
    let syntax = site_syntax_at(
        Language::Python,
        &tree,
        py.as_bytes(),
        5,
        19,
        Encoding::Utf16,
    )
    .unwrap();
    assert_eq!(syntax.call, Some("call"));
    assert_eq!(syntax.literals, ["topic"]);
}

#[test]
fn byte_offsets_count_each_encoding() {
    let source = "a\n😀é go\n".as_bytes();
    assert_eq!(byte_offset(source, 0, 0, Encoding::Utf8), Some(0));
    assert_eq!(byte_offset(source, 1, 0, Encoding::Utf8), Some(2));
    // 😀 is 4 bytes and 2 UTF-16 units; é is 2 bytes and 1 unit.
    assert_eq!(
        byte_offset(source, 1, 4, Encoding::Utf16),
        Some(2 + 4 + 2 + 1)
    );
    assert_eq!(byte_offset(source, 1, 7, Encoding::Utf8), Some(2 + 7));
    assert_eq!(
        byte_offset(source, 1, 1, Encoding::Utf16),
        None,
        "inside 😀"
    );
    assert_eq!(byte_offset(source, 1, 6, Encoding::Utf16), Some(2 + 9));
    assert_eq!(
        byte_offset(source, 1, 7, Encoding::Utf16),
        None,
        "past the line"
    );
    assert_eq!(byte_offset(source, 5, 0, Encoding::Utf8), None);
}

#[test]
fn every_grammar_is_within_the_cores_abi_range() {
    let range = tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION;
    for language in [
        Language::Rust,
        Language::Go,
        Language::TypeScript,
        Language::Python,
    ] {
        let abi = language.grammar().abi_version();
        assert!(
            range.contains(&abi),
            "{language:?}: ABI {abi}, core reads {range:?}"
        );
        println!("{language:?}: ABI {abi}; core reads {range:?}");
    }
}

#[test]
fn quotes_and_prefixes_are_removed() {
    assert_eq!(unquote("\"a\""), "a");
    assert_eq!(unquote("r#\"a\"#"), "a");
    assert_eq!(unquote("b\"a\""), "a");
    assert_eq!(unquote("'''a'''"), "a");
    assert_eq!(unquote("`a`"), "a");
    assert_eq!(unquote("rb'a'"), "a");
}
