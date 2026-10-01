//! Steps for `features/index/report.feature` (PLAN.md §9 P1.8): the
//! resolution report and the edge-count regression checks of
//! `codetags report`.

use std::collections::BTreeSet;

use cucumber::gherkin::Step;
use cucumber::given;
use cucumber::then;
use serde_json::Value;

use crate::CodetagsWorld;
use crate::steps_index::table_rows;

#[given(expr = "the edge count of {string} in generation {int} is changed to {int}")]
fn change_edge_count(world: &mut CodetagsWorld, path: String, generation: u64, edges: i64) {
    let file = world
        .scratch()
        .join(".codetags")
        .join("index")
        .join(format!("gen-{generation}.duckdb"));
    let db = codetags_model::open_read_write(&file)
        .unwrap_or_else(|error| panic!("open {}: {error}", file.display()));
    let changed = db
        .execute(
            "UPDATE run_file SET edge_count = ? WHERE path = ?",
            [&edges as &dyn codetags_model::ToSql, &path],
        )
        .expect("update run_file");
    assert!(
        changed > 0,
        "generation {generation} has no edge count for {path}"
    );
}

#[given(expr = "the file {string} in the scenario's directory holds:")]
fn file_holds(world: &mut CodetagsWorld, path: String, step: &Step) {
    let text = step.docstring.as_ref().expect("the step has a docstring");
    let file = world.scratch().join(&path);
    std::fs::write(&file, text).unwrap_or_else(|error| panic!("write {}: {error}", file.display()));
}

/// A JSON value as a table cell: a string as is, null as "", anything else
/// as JSON.
fn cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// One counts row of the report, as a table row.
fn counts_row(scope: &str, counts: &Value) -> Vec<String> {
    vec![
        scope.to_string(),
        cell(counts.get("language")),
        cell(counts.get("package")),
        cell(counts.get("module")),
        cell(counts.get("call_sites")),
        cell(counts.get("resolved")),
        cell(counts.get("unresolved")),
    ]
}

#[then(expr = "the JSON resolution report counts:")]
fn json_report_counts(world: &mut CodetagsWorld, step: &Step) {
    let stdout = &world.last().stdout;
    let report: Value = serde_json::from_str(stdout)
        .unwrap_or_else(|error| panic!("stdout is not JSON ({error}):\n{stdout}"));
    let mut actual = BTreeSet::new();
    for (scope, key) in [("module", "modules"), ("language", "languages")] {
        let rows = report[key]
            .as_array()
            .unwrap_or_else(|| panic!("the report has no {key} array:\n{stdout}"));
        actual.extend(rows.iter().map(|row| counts_row(scope, row)));
    }
    actual.insert(counts_row("total", &report["total"]));
    let expected: BTreeSet<Vec<String>> = table_rows(step)
        .into_iter()
        .map(|row| {
            [
                "scope",
                "language",
                "package",
                "module",
                "call sites",
                "resolved",
                "unresolved",
            ]
            .iter()
            .map(|column| row[*column].clone())
            .collect()
        })
        .collect();
    assert_eq!(actual, expected);
}

#[then(expr = "the file {string} in the scenario's directory holds the JSON:")]
fn file_holds_json(world: &mut CodetagsWorld, path: String, step: &Step) {
    let expected: Value = serde_json::from_str(step.docstring.as_ref().expect("a docstring"))
        .expect("the docstring is JSON");
    let file = world.scratch().join(&path);
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
    let actual: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{path} is not JSON ({error}):\n{text}"));
    assert_eq!(actual, expected, "{path}");
}
