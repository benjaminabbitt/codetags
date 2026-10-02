//! Expanding calls of trait methods to their implementations (PLAN.md D31;
//! brief §4.4 rules: "Interface calls resolve to the interface method.
//! Write the implementations as separate `call_target` rows, labeled with
//! the method that produced them").
//!
//! rust-analyzer writes no `is_implementation` relationships (V64), so the
//! implementations come from its `textDocument/implementation`
//! ([`crate::provider::rust::implementations`]). In two steps around that:
//!
//! 1. [`questions`] picks, from one ingested SCIP run, the trait methods of
//!    the project (`kind = trait_method`, with a definition) that a call
//!    site targets, and the position of each one's definition. A
//!    dependency's trait methods are not expanded: their implementations
//!    are mostly outside the project, and the answer would be every impl in
//!    the dependency graph.
//! 2. [`write`] maps each answer to the SCIP definition whose range holds
//!    it, and adds a `call_target` row with the method [`METHOD`] for every
//!    implementation, on every call site whose declared target is the trait
//!    method. The declared target stays; how many targets of [`METHOD`] a
//!    call site has is its candidate count (brief §4.5, `prov:candidates`),
//!    as for Python's name-match candidates.
//!
//! The pass is recorded as its own run, [`IMPL_PROVIDER`], whose per-file
//! edge counts are its call targets on each file's call sites, so the
//! edge-count checks catch a pass that silently finds less.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use codetags_model::{Connection, ToSql};

use super::write::{next_id, write_run};
use super::{IngestError, RunInfo};
#[cfg(doc)]
use crate::provider::rust::IMPL_PROVIDER;
use crate::provider::rust::implementations::{Answers, Position};
use crate::provider::scip::{Range, ScipIndex};

/// The `call_target.method` of an implementation found this way.
pub const METHOD: &str = "lsp-impl";

/// What to ask: the called trait methods and their definitions' positions.
#[derive(Debug, Clone, Default)]
pub struct Questions {
    /// The SCIP run whose call sites are expanded.
    scip_run: i64,
    /// The trait methods, in order; `positions[i]` is the definition of
    /// `trait_methods[i]`.
    trait_methods: Vec<String>,
    /// Where to ask.
    pub positions: Vec<Position>,
    /// Every non-local definition per file: its range and symbol.
    definitions: BTreeMap<String, Vec<(Range, String)>>,
}

impl Questions {
    /// Whether there is nothing to ask, so the server need not start.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// What [`write`] added.
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    /// The pass's run id.
    pub run_id: i64,
    /// Trait methods asked about.
    pub trait_methods: usize,
    /// Distinct implementations found.
    pub implementations: usize,
    /// `call_target` rows written.
    pub targets: usize,
    /// Call sites that gained targets.
    pub sites: usize,
    /// Answers inside the project that hold no definition the SCIP index
    /// knows (e.g. an impl a macro wrote).
    pub unmapped: Vec<Position>,
    /// Answers outside the project, dropped.
    pub outside_root: usize,
    /// How long the server took to load the workspace.
    pub load_time: Duration,
    /// How long it then took to answer every question.
    pub answer_time: Duration,
}

/// Whether `range` holds the position `line`, `character`.
fn holds(range: &Range, line: u32, character: u32) -> bool {
    (range.start_line, range.start_character) <= (line, character)
        && (line, character) < (range.end_line, range.end_character)
}

/// The questions for the call sites of the ingested SCIP run `scip_run`,
/// whose index is `index`.
pub fn questions(
    db: &Connection,
    index: &ScipIndex,
    scip_run: i64,
) -> Result<Questions, IngestError> {
    let mut statement = db.prepare(
        "SELECT DISTINCT s.declared_target
         FROM call_site s JOIN symbol t ON t.id = s.declared_target
         WHERE s.run_id = ? AND t.kind = 'trait_method' AND t.file IS NOT NULL
         ORDER BY 1",
    )?;
    let called: Vec<String> = statement
        .query_map([scip_run], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut definitions: BTreeMap<String, Vec<(Range, String)>> = BTreeMap::new();
    let mut first_definition: BTreeMap<&str, Position> = BTreeMap::new();
    for document in &index.documents {
        for occurrence in &document.occurrences {
            if !occurrence.is_definition() || occurrence.is_local() {
                continue;
            }
            let range = occurrence.range;
            first_definition
                .entry(occurrence.symbol.as_str())
                .or_insert_with(|| Position {
                    path: document.relative_path.clone(),
                    line: range.start_line,
                    character: range.start_character,
                });
            // An implementation is a method: only method descriptors
            // (`name().`) can be answers, never the module or impl block
            // whose range also holds the name.
            if occurrence.symbol.ends_with(").") {
                definitions
                    .entry(document.relative_path.clone())
                    .or_default()
                    .push((range, occurrence.symbol.clone()));
            }
        }
    }
    let mut trait_methods = Vec::new();
    let mut positions = Vec::new();
    for symbol in called {
        if let Some(position) = first_definition.get(symbol.as_str()) {
            positions.push(position.clone());
            trait_methods.push(symbol);
        }
    }
    Ok(Questions {
        scip_run,
        trait_methods,
        positions,
        definitions,
    })
}

/// Writes the implementations in `answers` to `questions` as call targets,
/// and the pass as a run described by `run`, in one transaction.
pub fn write(
    db: &Connection,
    questions: &Questions,
    answers: &Answers,
    run: &RunInfo,
) -> Result<Expansion, IngestError> {
    db.execute_batch("BEGIN TRANSACTION")?;
    match write_rows(db, questions, answers, run) {
        Ok(expansion) => {
            db.execute_batch("COMMIT")?;
            Ok(expansion)
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn write_rows(
    db: &Connection,
    questions: &Questions,
    answers: &Answers,
    run: &RunInfo,
) -> Result<Expansion, IngestError> {
    let run_id = next_id(db, "run", "run_id")?;
    write_run(db, run_id, run)?;

    let mut expansion = Expansion {
        run_id,
        trait_methods: questions.trait_methods.len(),
        outside_root: answers.outside_root,
        load_time: answers.load_time,
        answer_time: answers.answer_time,
        ..Expansion::default()
    };
    let mut implementations = BTreeSet::new();
    let mut known = db.prepare("SELECT count(*) FROM symbol WHERE id = ?")?;
    let mut insert = db.prepare(
        "INSERT INTO call_target (site_id, target, method)
         SELECT site_id, ?, ? FROM call_site WHERE run_id = ? AND declared_target = ?",
    )?;
    for (trait_method, found) in questions.trait_methods.iter().zip(&answers.locations) {
        let mut targets = BTreeSet::new();
        for position in found {
            let symbol = questions
                .definitions
                .get(&position.path)
                .and_then(|definitions| {
                    // The innermost: a module's definition can span its file.
                    definitions
                        .iter()
                        .filter(|(range, _)| holds(range, position.line, position.character))
                        .max_by_key(|(range, _)| (range.start_line, range.start_character))
                })
                .map(|(_, symbol)| symbol)
                .filter(|&symbol| symbol != trait_method);
            let known_symbol = match symbol {
                Some(symbol) => {
                    let count: i64 = known.query_row([symbol], |row| row.get(0))?;
                    (count > 0).then_some(symbol)
                }
                None => None,
            };
            match known_symbol {
                Some(symbol) => {
                    targets.insert(symbol.clone());
                }
                None => expansion.unmapped.push(position.clone()),
            }
        }
        for target in targets {
            expansion.targets += insert.execute([
                &target as &dyn ToSql,
                &METHOD,
                &questions.scip_run,
                trait_method,
            ])?;
            implementations.insert(target);
        }
    }
    expansion.implementations = implementations.len();
    expansion.sites = db.query_row(
        "SELECT count(DISTINCT t.site_id) FROM call_target t
         JOIN call_site s USING (site_id)
         WHERE s.run_id = ? AND t.method = ?",
        [&questions.scip_run as &dyn ToSql, &METHOD],
        |row| row.get(0),
    )?;

    // Per-file edges: this pass's call targets on each file's call sites,
    // for every file of the SCIP run.
    db.execute(
        "INSERT INTO run_file (run_id, path, edge_count)
         SELECT ?, f.path, (
             SELECT count(*) FROM call_target t JOIN call_site s USING (site_id)
             WHERE s.run_id = f.run_id AND s.file = f.path AND t.method = ?)
         FROM run_file f WHERE f.run_id = ?",
        [&run_id as &dyn ToSql, &METHOD, &questions.scip_run],
    )?;
    db.execute(
        "UPDATE run SET source_tree = (SELECT source_tree FROM run WHERE run_id = ?)
         WHERE run_id = ?",
        [&questions.scip_run, &run_id],
    )?;
    Ok(expansion)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::SystemTime;

    use codetags_model::GenerationStore;

    use super::*;
    use crate::provider::rust::IMPL_PROVIDER;
    use crate::provider::scip::{Document, Occurrence};

    fn occurrence(symbol: &str, line: u32, start: u32, end: u32, definition: bool) -> Occurrence {
        Occurrence {
            symbol: symbol.to_string(),
            range: Range {
                start_line: line,
                start_character: start,
                end_line: line,
                end_character: end,
            },
            enclosing_range: None,
            roles: i32::from(definition),
        }
    }

    /// A trait method `t` (line 1), two implementations `a` (line 5) and
    /// `b` (line 9), and two call sites of `t` and one of `a`.
    fn fixture(db: &Connection) -> ScipIndex {
        db.execute_batch(
            "INSERT INTO run VALUES (1, 'scip', 'v', [], now(), now(), 'succeeded', 'tree');
             INSERT INTO symbol (id, name, kind, file, modules, external) VALUES
               ('t().', 'T.m', 'trait_method', 'x.rs', [], false),
               ('a().', 'A.T.m', 'method', 'x.rs', [], false),
               ('b().', 'B.T.m', 'method', 'x.rs', [], false),
               ('f', 'f', 'function', 'y.rs', [], false),
               ('m', 'm', 'module', 'x.rs', [], false),
               ('ext', 'core.E.m', 'trait_method', NULL, [], true);
             INSERT INTO call_site VALUES
               (1, 1, 'f', 'y.rs', 1, 1, 1, 'call', NULL, 't().', 'virtual', 's'),
               (2, 1, 'f', 'y.rs', 2, 1, 2, 'call', NULL, 't().', 'virtual', 's'),
               (3, 1, 'f', 'y.rs', 3, 1, 3, 'call', NULL, 'a().', 'static', 's'),
               (4, 1, 'f', 'y.rs', 4, 1, 4, 'call', NULL, 'ext', 'virtual', 's');
             INSERT INTO call_target SELECT site_id, declared_target, 'declared' FROM call_site;
             INSERT INTO run_file VALUES (1, 'x.rs', 0), (1, 'y.rs', 4);",
        )
        .unwrap();
        ScipIndex {
            tool: "t().".to_string(),
            project_root: "file:///p".to_string(),
            documents: vec![Document {
                relative_path: "x.rs".to_string(),
                occurrences: vec![
                    // A file module's definition can span the file.
                    Occurrence {
                        range: Range {
                            start_line: 0,
                            start_character: 0,
                            end_line: 40,
                            end_character: 0,
                        },
                        ..occurrence("m", 0, 0, 0, true)
                    },
                    occurrence("t().", 1, 7, 8, true),
                    occurrence("a().", 5, 7, 8, true),
                    occurrence("b().", 9, 7, 8, true),
                    occurrence("t().", 12, 3, 4, false),
                ],
                symbols: Vec::new(),
            }],
            external_symbols: Vec::new(),
        }
    }

    fn run() -> RunInfo {
        RunInfo {
            provider: IMPL_PROVIDER.to_string(),
            provider_version: "v".to_string(),
            args: vec!["lsp".to_string()],
            started_at: SystemTime::now(),
            finished_at: SystemTime::now(),
        }
    }

    fn at(path: &str, line: u32, character: u32) -> Position {
        Position {
            path: path.to_string(),
            line,
            character,
        }
    }

    #[test]
    fn only_called_project_trait_methods_are_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let writer = GenerationStore::new(dir.path()).begin().unwrap();
        let index = fixture(writer.connection());
        let questions = questions(writer.connection(), &index, 1).unwrap();
        assert_eq!(questions.trait_methods, ["t()."]);
        assert_eq!(questions.positions, [at("x.rs", 1, 7)]);
    }

    #[test]
    fn implementations_become_candidate_targets_of_every_call() {
        let dir = tempfile::tempdir().unwrap();
        let writer = GenerationStore::new(dir.path()).begin().unwrap();
        let db = writer.connection();
        let index = fixture(db);
        let questions = questions(db, &index, 1).unwrap();
        let answers = Answers {
            locations: vec![vec![
                at("x.rs", 5, 7),
                at("x.rs", 9, 8),
                at("x.rs", 9, 7),
                at("x.rs", 30, 0),
                at("x.rs", 1, 7),
            ]],
            outside_root: 2,
            load_time: Duration::from_millis(5),
            answer_time: Duration::from_millis(1),
        };
        let expansion = write(db, &questions, &answers, &run()).unwrap();
        assert_eq!(expansion.run_id, 2);
        assert_eq!(expansion.trait_methods, 1);
        assert_eq!(expansion.implementations, 2);
        assert_eq!(expansion.targets, 4);
        assert_eq!(expansion.sites, 2);
        assert_eq!(expansion.outside_root, 2);
        // Line 9 column 8 is past `b`'s one-character name; line 30 holds
        // nothing; the trait method itself is not its own implementation.
        assert_eq!(
            expansion.unmapped,
            [at("x.rs", 9, 8), at("x.rs", 30, 0), at("x.rs", 1, 7)]
        );
        let mut statement = db
            .prepare(
                "SELECT site_id, target, method FROM call_target
                 ORDER BY site_id, method, target",
            )
            .unwrap();
        let rows: Vec<(i64, String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let rows: Vec<(i64, &str, &str)> = rows
            .iter()
            .map(|(s, t, m)| (*s, t.as_str(), m.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (1, "t().", "declared"),
                (1, "a().", "lsp-impl"),
                (1, "b().", "lsp-impl"),
                (2, "t().", "declared"),
                (2, "a().", "lsp-impl"),
                (2, "b().", "lsp-impl"),
                (3, "a().", "declared"),
                (4, "ext", "declared"),
            ]
        );
        let mut statement = db
            .prepare(
                "SELECT r.provider, f.path, f.edge_count, r.source_tree
                 FROM run_file f JOIN run r USING (run_id) WHERE run_id = 2 ORDER BY f.path",
            )
            .unwrap();
        let files: Vec<(String, String, i64, String)> = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            files,
            [
                (IMPL_PROVIDER.into(), "x.rs".into(), 0, "tree".into()),
                (IMPL_PROVIDER.into(), "y.rs".into(), 4, "tree".into()),
            ]
        );
    }
}
