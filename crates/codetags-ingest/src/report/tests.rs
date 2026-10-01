#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use codetags_model::GenerationStore;

use super::*;

/// Writes a complete generation holding `sql` (after a `run` row 1 of
/// provider `p`), and returns its number.
fn generation(index: &Path, sql: &str) -> u64 {
    let writer = GenerationStore::new(index).begin().unwrap();
    writer
        .connection()
        .execute_batch(&format!(
            "INSERT INTO run VALUES (1, 'p', 'v', [], now(), now(), 'succeeded', 't');
             {sql}"
        ))
        .unwrap();
    writer.complete().unwrap()
}

/// Callers in two languages: module `m` of two Python packages, and a Rust
/// package root; call sites with and without resolved targets.
const SITES: &str = "
INSERT INTO file VALUES ('a.py', 'python', 'h', 1), ('b.py', 'python', 'h', 1),
                        ('c.rs', 'rust', 'h', 1);
INSERT INTO symbol (id, name, lang, kind, module, modules, package, external) VALUES
  ('a', 'm.a', 'python', 'function', 'm', ['m'], 'pa', false),
  ('b', 'm.b', 'python', 'function', 'm', ['m'], 'pb', false),
  ('c', 'c', 'rust', 'function', NULL, [], 'rc', false);
INSERT INTO call_site VALUES
  (1, 1, 'a', 'a.py', 1, 1, 1, 'call', NULL, 't', 'unknown', 's'),
  (2, 1, 'a', 'a.py', 2, 1, 2, 'call', NULL, 't', 'unknown', 's'),
  (3, 1, 'a', 'a.py', 3, 1, 3, 'call', NULL, 't', 'unknown', 's'),
  (4, 1, 'b', 'b.py', 1, 1, 1, 'call', NULL, 't', 'unknown', 's'),
  (5, 1, 'c', 'c.rs', 1, 1, 1, 'call', NULL, 't', 'static', 's');
INSERT INTO call_target VALUES
  (1, 't', 'declared'),
  (2, 'x', 'name-match'), (2, 'y', 'name-match'),
  (4, 't', 'name-match'), (4, 't', 'vta'),
  (5, 't', 'declared');
INSERT INTO run_file VALUES (1, 'a.py', 3), (1, 'b.py', 1), (1, 'c.rs', 1);
";

#[test]
fn sites_without_a_target_or_with_only_name_matches_are_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(&index, SITES);
    let report = report(dir.path(), &index, &ReportOptions::default()).unwrap();
    type Row<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>, Counts);
    let modules: Vec<Row<'_>> = report
        .modules
        .iter()
        .map(|m| {
            (
                m.language.as_deref(),
                m.package.as_deref(),
                m.module.as_deref(),
                m.counts.clone(),
            )
        })
        .collect();
    assert_eq!(
        modules,
        [
            (Some("python"), Some("pa"), Some("m"), Counts::new(3, 1)),
            (Some("python"), Some("pb"), Some("m"), Counts::new(1, 1)),
            (Some("rust"), Some("rc"), None, Counts::new(1, 1)),
        ]
    );
    let languages: Vec<(Option<&str>, Counts)> = report
        .languages
        .iter()
        .map(|l| (l.language.as_deref(), l.counts.clone()))
        .collect();
    assert_eq!(
        languages,
        [
            (Some("python"), Counts::new(4, 2)),
            (Some("rust"), Counts::new(1, 1)),
        ]
    );
    assert_eq!(report.total.0, Counts::new(5, 3));
    assert_eq!(report.total.0.unresolved, 2);
    assert_eq!(report.total.0.unresolved_rate(), Some(0.4));
    assert!(report.passed());
}

#[test]
fn no_call_sites_have_no_rate() {
    assert_eq!(Counts::default().unresolved_rate(), None);
}

#[test]
fn the_json_report_carries_rates_and_nulls() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(&index, SITES);
    let report = report(dir.path(), &index, &ReportOptions::default()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["generation"], 1);
    assert_eq!(json["modules"][0]["package"], "pa");
    assert_eq!(json["modules"][0]["unresolved_rate"], 2.0 / 3.0);
    assert_eq!(json["modules"][2]["module"], serde_json::Value::Null);
    assert_eq!(json["languages"][0]["language"], "python");
    assert!(json["languages"][0].get("module").is_none());
    assert_eq!(json["total"]["unresolved"], 2);
    assert_eq!(json["regression"]["previous"], serde_json::Value::Null);
    assert_eq!(json["regression"]["threshold_percent"], 50);
    assert_eq!(json["edge_counts"]["p"]["a.py"], 3);
}

#[test]
fn the_text_report_lists_modules_languages_and_the_total() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(&index, SITES);
    let text = report(dir.path(), &index, &ReportOptions::default())
        .unwrap()
        .to_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "generation 1",
            "language  package  module  call sites  resolved  unresolved  unresolved rate",
            "python    pa       m       3           1         2           66.7%",
            "python    pb       m       1           1         0           0.0%",
            "python    (all)            4           2         2           50.0%",
            "rust      rc       (root)  1           1         0           0.0%",
            "rust      (all)            1           1         0           0.0%",
            "total                      5           3         2           40.0%",
            "edge counts: no previous generation to compare with",
        ]
    );
}

#[test]
fn a_drop_since_the_previous_generation_fails() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(
        &index,
        "INSERT INTO run_file VALUES (1, 'a.rs', 10), (1, 'b.rs', 4);",
    );
    generation(
        &index,
        "INSERT INTO run_file VALUES (1, 'a.rs', 4), (1, 'b.rs', 4);",
    );
    let report = report(dir.path(), &index, &ReportOptions::default()).unwrap();
    assert_eq!(report.generation, 2);
    assert_eq!(report.regression.previous, Some(1));
    assert_eq!(report.regression.drops.len(), 1);
    assert_eq!(report.regression.drops[0].path, "a.rs");
    assert!(!report.passed());
    let text = report.to_text();
    assert!(text.contains("p a.rs: 10 -> 4 edges"), "{text}");
    assert!(text.contains("likely a provider failure"), "{text}");
}

#[test]
fn a_lost_file_fails_and_a_deleted_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    std::fs::write(dir.path().join("kept.rs"), "").unwrap();
    generation(
        &index,
        "INSERT INTO run_file VALUES (1, 'kept.rs', 2), (1, 'gone.rs', 2);",
    );
    generation(&index, "");
    let report = report(dir.path(), &index, &ReportOptions::default()).unwrap();
    let drops: Vec<(&str, Option<i64>)> = report
        .regression
        .drops
        .iter()
        .map(|d| (d.path.as_str(), d.current))
        .collect();
    assert_eq!(drops, [("kept.rs", None)]);
    assert!(
        report
            .to_text()
            .contains("2 -> none (the file still exists) edges")
    );
}

#[test]
fn an_earlier_generation_is_compared_with_its_own_predecessor() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(&index, "INSERT INTO run_file VALUES (1, 'a.rs', 10);");
    generation(&index, "INSERT INTO run_file VALUES (1, 'a.rs', 0);");
    let options = ReportOptions {
        generation: Some(1),
        ..ReportOptions::default()
    };
    let report = report(dir.path(), &index, &options).unwrap();
    assert_eq!(report.generation, 1);
    assert_eq!(report.regression.previous, None);
    assert!(report.passed());
}

#[test]
fn a_baseline_is_compared_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(
        &index,
        "INSERT INTO run_file VALUES (1, 'a.rs', 3), (1, 'b.rs', 0);",
    );
    let current = report(dir.path(), &index, &ReportOptions::default())
        .unwrap()
        .edge_counts;
    let text = baseline_json(&current);
    assert_eq!(
        text,
        "{\n  \"p\": {\n    \"a.rs\": 3,\n    \"b.rs\": 0\n  }\n}\n"
    );
    assert_eq!(parse_baseline(&text).unwrap(), current);

    let matching = dir.path().join("match.json");
    std::fs::write(&matching, &text).unwrap();
    let options = ReportOptions {
        baseline: Some(matching),
        ..ReportOptions::default()
    };
    let report_ok = report(dir.path(), &index, &options).unwrap();
    assert!(report_ok.passed());
    assert!(report_ok.to_text().contains(": matches"));

    let differing = dir.path().join("differ.json");
    std::fs::write(&differing, "{\"p\": {\"a.rs\": 4, \"b.rs\": 0}}").unwrap();
    let options = ReportOptions {
        baseline: Some(differing),
        ..ReportOptions::default()
    };
    let report_bad = report(dir.path(), &index, &options).unwrap();
    assert!(!report_bad.passed());
    let text = report_bad.to_text();
    assert!(text.contains(": 1 difference ("), "{text}");
    assert!(text.contains("\n- p a.rs 4\n+ p a.rs 3\n"), "{text}");
}

#[test]
fn a_malformed_or_missing_baseline_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    generation(&index, "");
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{\"p\": [1]}").unwrap();
    for (path, expect_format) in [(bad, true), (dir.path().join("absent.json"), false)] {
        let options = ReportOptions {
            baseline: Some(path),
            ..ReportOptions::default()
        };
        match report(dir.path(), &index, &options) {
            Err(ReportError::BaselineFormat { .. }) if expect_format => {}
            Err(ReportError::BaselineRead { .. }) if !expect_format => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn an_empty_index_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = report(
        dir.path(),
        &dir.path().join("index"),
        &ReportOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, ReportError::Store(StoreError::NoGeneration { .. })),
        "{error}"
    );
}
