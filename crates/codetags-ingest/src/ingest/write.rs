//! Writing one run's rows into a generation.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use codetags_model::{Connection, ToSql};

use super::analyze::Analysis;
use super::{IngestError, RunInfo};

/// Seconds since the Unix epoch, for DuckDB's `to_timestamp`.
fn epoch_seconds(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    }
}

fn int(value: u32) -> i64 {
    i64::from(value)
}

/// The id the next row of `column` in `table` gets: one more than the
/// highest, so several runs can share a generation.
pub(crate) fn next_id(db: &Connection, table: &str, column: &str) -> Result<i64, IngestError> {
    Ok(db.query_row(
        &format!("SELECT coalesce(max({column}), 0) + 1 FROM {table}"),
        [],
        |row| row.get(0),
    )?)
}

/// Writes `analysis` as one run, in one transaction. Returns the run's id.
pub(crate) fn write(
    db: &Connection,
    analysis: &Analysis,
    run: &RunInfo,
    language: &str,
    source: &str,
) -> Result<i64, IngestError> {
    db.execute_batch("BEGIN TRANSACTION")?;
    match write_rows(db, analysis, run, language, source) {
        Ok(run_id) => {
            db.execute_batch("COMMIT")?;
            Ok(run_id)
        }
        Err(error) => {
            // The original error matters more than a failed rollback.
            let _ = db.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn write_rows(
    db: &Connection,
    analysis: &Analysis,
    run: &RunInfo,
    language: &str,
    source: &str,
) -> Result<i64, IngestError> {
    let run_id = next_id(db, "run", "run_id")?;
    write_run(db, run_id, run)?;

    let mut statement = db.prepare(
        "INSERT INTO file (path, language, content_hash, size)
         VALUES (?, ?, 'sha256:' || sha256(?::BLOB), ?)
         ON CONFLICT (path) DO NOTHING",
    )?;
    for file in &analysis.files {
        let size = i64::try_from(file.contents.len()).unwrap_or(i64::MAX);
        statement.execute([&file.path as &dyn ToSql, &language, &file.contents, &size])?;
    }

    // Module names never contain `/` (D15), so it joins the list safely.
    let mut statement = db.prepare(
        "INSERT INTO symbol (id, name, lang, kind, file, start_line, end_line, signature,
                             module, modules, package, external)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?,
                 coalesce(string_split(nullif(?, ''), '/'), []::TEXT[]), ?, ?)
         ON CONFLICT (id) DO NOTHING",
    )?;
    for symbol in &analysis.symbols {
        let (start, end) = match symbol.lines {
            Some((start, end)) => (Some(int(start)), Some(int(end))),
            None => (None, None),
        };
        statement.execute([
            &symbol.id as &dyn ToSql,
            &symbol.name,
            &language,
            &symbol.kind,
            &symbol.file,
            &start,
            &end,
            &symbol.modules.last(),
            &symbol.modules.join("/"),
            &symbol.package,
            &symbol.external,
        ])?;
    }

    let first_site = next_id(db, "call_site", "site_id")?;
    let mut site_statement = db.prepare(
        "INSERT INTO call_site (site_id, run_id, caller, file, line, col, ordinal, ref_kind,
                                receiver_text, declared_target, dispatch, source)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
    )?;
    let mut target_statement =
        db.prepare("INSERT INTO call_target (site_id, target, method) VALUES (?, ?, 'declared')")?;
    let mut edges: BTreeMap<&str, i64> = analysis
        .files
        .iter()
        .map(|file| (file.path.as_str(), 0))
        .collect();
    for (site_id, site) in (first_site..).zip(&analysis.sites) {
        site_statement.execute([
            &site_id as &dyn ToSql,
            &run_id,
            &site.caller,
            &site.file,
            &int(site.line),
            &int(site.col),
            &int(site.ordinal),
            &site.ref_kind.as_str(),
            &site.target,
            &site.dispatch.as_str(),
            &source,
        ])?;
        target_statement.execute([&site_id as &dyn ToSql, &site.target])?;
        *edges.entry(site.file.as_str()).or_insert(0) += 1;
    }

    let mut statement =
        db.prepare("INSERT INTO run_file (run_id, path, edge_count) VALUES (?, ?, ?)")?;
    for (path, count) in &edges {
        statement.execute([&run_id as &dyn ToSql, path, count])?;
    }
    // No git lookup yet: the source tree is a content hash of the run's
    // files (PLAN.md §2.2 allows either).
    db.execute(
        "UPDATE run SET source_tree = (
             SELECT 'sha256:' || sha256(coalesce(
                 string_agg(f.path || ' ' || f.content_hash, chr(10) ORDER BY f.path), ''))
             FROM file f JOIN run_file r ON r.path = f.path AND r.run_id = ?)
         WHERE run_id = ?",
        [&run_id, &run_id],
    )?;
    Ok(run_id)
}

/// Writes the `run` row, its source tree still to be filled in.
pub(crate) fn write_run(db: &Connection, run_id: i64, run: &RunInfo) -> Result<(), IngestError> {
    let placeholders = vec!["?"; run.args.len()].join(", ");
    let sql = format!(
        "INSERT INTO run (run_id, provider, provider_version, args, started_at, finished_at,
                          status, source_tree)
         VALUES (?, ?, ?, [{placeholders}]::TEXT[], to_timestamp(?), to_timestamp(?),
                 'succeeded', '')"
    );
    let started = epoch_seconds(run.started_at);
    let finished = epoch_seconds(run.finished_at);
    let mut params: Vec<&dyn ToSql> = vec![&run_id, &run.provider, &run.provider_version];
    params.extend(run.args.iter().map(|arg| arg as &dyn ToSql));
    params.push(&started);
    params.push(&finished);
    db.execute(&sql, params.as_slice())?;
    Ok(())
}
