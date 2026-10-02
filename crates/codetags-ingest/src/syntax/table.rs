//! The per-language node-kind tables the walker reads.
//!
//! Each language names, in its grammar's node kinds: the call nodes and the
//! field holding the callee; the nodes that set a call's control context,
//! and which of their children count; the definitions that end the walk; and
//! the string literals.

use super::{Control, Language};

/// Which children of a control node put a call in its context.
#[derive(Debug, Clone, Copy)]
pub(super) enum Counted {
    /// Every child.
    All,
    /// Every child but these fields: an `if`'s condition runs whether or
    /// not the branch does; a `for`'s iterable runs once.
    Except(&'static [&'static str]),
    /// Only these fields: a `try`'s body, not its handlers.
    Only(&'static [&'static str]),
}

/// A call node kind, and the field holding its callee (`None`: the first
/// named child, for Rust's `macro_invocation`).
pub(super) type CallKind = (&'static str, Option<&'static str>, &'static str);

/// One language's table.
#[derive(Debug)]
pub(super) struct Table {
    /// Call kinds: kind, callee field, arguments field (or child kind).
    pub calls: &'static [CallKind],
    /// Control kinds, what they mean, and which children count.
    pub controls: &'static [(&'static str, Control, Counted)],
    /// Definitions: the walk stops at the first one, the call's caller as
    /// SCIP attributes it. Closures and lambdas are not here: SCIP gives
    /// them no definition of their own, so the walk goes through them.
    pub definitions: &'static [&'static str],
    /// String literal kinds.
    pub strings: &'static [&'static str],
    /// Child kinds whose presence makes a string not a constant
    /// (interpolation).
    pub interpolations: &'static [&'static str],
}

const RUST: Table = Table {
    calls: &[
        ("call_expression", Some("function"), "arguments"),
        ("macro_invocation", Some("macro"), "token_tree"),
    ],
    controls: &[
        (
            "if_expression",
            Control::Conditional,
            Counted::Except(&["condition"]),
        ),
        (
            "match_expression",
            Control::Conditional,
            Counted::Except(&["value"]),
        ),
        ("for_expression", Control::Loop, Counted::Except(&["value"])),
        ("while_expression", Control::Loop, Counted::All),
        ("loop_expression", Control::Loop, Counted::All),
        ("await_expression", Control::Await, Counted::All),
    ],
    definitions: &["function_item", "function_signature_item"],
    strings: &["string_literal", "raw_string_literal"],
    interpolations: &[],
};

const GO: Table = Table {
    calls: &[("call_expression", Some("function"), "arguments")],
    controls: &[
        (
            "if_statement",
            Control::Conditional,
            Counted::Except(&["condition", "initializer"]),
        ),
        (
            "expression_switch_statement",
            Control::Conditional,
            Counted::Except(&["value", "initializer"]),
        ),
        (
            "type_switch_statement",
            Control::Conditional,
            Counted::Except(&["value", "initializer", "alias"]),
        ),
        ("select_statement", Control::Conditional, Counted::All),
        ("for_statement", Control::Loop, Counted::Only(&["body"])),
        ("go_statement", Control::Goroutine, Counted::All),
        ("defer_statement", Control::Defer, Counted::All),
    ],
    definitions: &["function_declaration", "method_declaration"],
    strings: &["interpreted_string_literal", "raw_string_literal"],
    interpolations: &[],
};

const TYPESCRIPT: Table = Table {
    calls: &[
        ("call_expression", Some("function"), "arguments"),
        ("new_expression", Some("constructor"), "arguments"),
    ],
    controls: &[
        (
            "if_statement",
            Control::Conditional,
            Counted::Except(&["condition"]),
        ),
        (
            "switch_statement",
            Control::Conditional,
            Counted::Except(&["value"]),
        ),
        (
            "ternary_expression",
            Control::Conditional,
            Counted::Except(&["condition"]),
        ),
        (
            "for_statement",
            Control::Loop,
            Counted::Except(&["initializer"]),
        ),
        (
            "for_in_statement",
            Control::Loop,
            Counted::Except(&["right"]),
        ),
        ("while_statement", Control::Loop, Counted::All),
        ("do_statement", Control::Loop, Counted::All),
        ("try_statement", Control::Try, Counted::Only(&["body"])),
        ("await_expression", Control::Await, Counted::All),
    ],
    definitions: &[
        "function_declaration",
        "generator_function_declaration",
        "method_definition",
        "function_expression",
    ],
    strings: &["string", "template_string"],
    interpolations: &["template_substitution"],
};

const PYTHON: Table = Table {
    calls: &[("call", Some("function"), "arguments")],
    controls: &[
        (
            "if_statement",
            Control::Conditional,
            Counted::Except(&["condition"]),
        ),
        ("conditional_expression", Control::Conditional, Counted::All),
        (
            "match_statement",
            Control::Conditional,
            Counted::Except(&["subject"]),
        ),
        ("for_statement", Control::Loop, Counted::Except(&["right"])),
        ("while_statement", Control::Loop, Counted::All),
        (
            "list_comprehension",
            Control::Loop,
            Counted::Only(&["body"]),
        ),
        ("set_comprehension", Control::Loop, Counted::Only(&["body"])),
        (
            "dictionary_comprehension",
            Control::Loop,
            Counted::Only(&["body"]),
        ),
        (
            "generator_expression",
            Control::Loop,
            Counted::Only(&["body"]),
        ),
        ("try_statement", Control::Try, Counted::Only(&["body"])),
        ("await", Control::Await, Counted::All),
    ],
    definitions: &["function_definition", "class_definition"],
    strings: &["string"],
    interpolations: &["interpolation"],
};

/// The table of `language`.
pub(super) fn table(language: Language) -> &'static Table {
    match language {
        Language::Rust => &RUST,
        Language::Go => &GO,
        Language::TypeScript => &TYPESCRIPT,
        Language::Python => &PYTHON,
    }
}
