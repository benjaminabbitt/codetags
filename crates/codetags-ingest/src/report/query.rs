//! Reading a generation's call sites and edge counts.

use std::collections::BTreeMap;

use codetags_model::{Connection, Error};

use super::{Counts, EdgeCounts, LanguageCounts, ModuleCounts};

/// Per call site: the language of its file, its caller's package and module,
/// and whether it is resolved (see [`super`]). Grouped by language, package
/// and module.
const BY_MODULE: &str = "
WITH site AS (
  SELECT f.language AS language,
         s.package AS package,
         s.module AS module,
         EXISTS (SELECT 1 FROM call_target t
                 WHERE t.site_id = c.site_id AND t.method <> 'name-match') AS resolved
  FROM call_site c
  LEFT JOIN file f ON f.path = c.file
  LEFT JOIN symbol s ON s.id = c.caller
)
SELECT language, package, module,
       count(*)::BIGINT,
       count(*) FILTER (WHERE resolved)::BIGINT
FROM site
GROUP BY language, package, module
ORDER BY language NULLS LAST, package NULLS LAST, module NULLS FIRST
";

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// The resolution counts per language and module, per language, and in
/// total.
pub(super) fn resolution(
    db: &Connection,
) -> Result<(Vec<ModuleCounts>, Vec<LanguageCounts>, Counts), Error> {
    let mut statement = db.prepare(BY_MODULE)?;
    let modules = statement
        .query_map([], |row| {
            let call_sites = count(row.get(3)?);
            let resolved = count(row.get(4)?);
            Ok(ModuleCounts {
                language: row.get(0)?,
                package: row.get(1)?,
                module: row.get(2)?,
                counts: Counts::new(call_sites, resolved),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut languages: Vec<LanguageCounts> = Vec::new();
    let mut total = Counts::default();
    for row in &modules {
        match languages.last_mut() {
            Some(last) if last.language == row.language => last.counts.add(&row.counts),
            _ => languages.push(LanguageCounts {
                language: row.language.clone(),
                counts: row.counts.clone(),
            }),
        }
        total.add(&row.counts);
    }
    Ok((modules, languages, total))
}

/// Every run's per-file edge counts, summed per provider and path.
pub(super) fn edge_counts(db: &Connection) -> Result<EdgeCounts, Error> {
    let mut statement = db.prepare(
        "SELECT r.provider, f.path, sum(f.edge_count)::BIGINT
         FROM run_file f JOIN run r ON r.run_id = f.run_id
         GROUP BY r.provider, f.path",
    )?;
    let mut counts: EdgeCounts = BTreeMap::new();
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (provider, path, edges) = row?;
        counts.entry(provider).or_default().insert(path, edges);
    }
    Ok(counts)
}
