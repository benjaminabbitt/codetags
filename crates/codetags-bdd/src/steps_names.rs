//! Steps for `features/names/`: item ids and collision suffixes
//! (PLAN.md §2.3, §2.7).

use std::str::FromStr;

use codetags_names::collision::apply_collision_suffixes;
use codetags_names::item_id::ItemId;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};

use crate::CodetagsWorld;

/// State the names steps share within one scenario.
#[derive(Debug, Default)]
pub struct NamesState {
    /// The last item id written.
    item_id: Option<String>,
    /// Directory entries as `(name, id)`, in table order.
    entries: Vec<(String, String)>,
    /// The names collision suffixing produced, aligned with `entries`.
    named: Vec<String>,
}

/// `{kind}` of an item id: `file` or `symbol`.
fn item_id(kind: &str, value: String) -> ItemId {
    match kind {
        "file" => ItemId::File(value),
        "symbol" => ItemId::Symbol(value),
        other => panic!("unknown item kind {other:?}: use file or symbol"),
    }
}

#[when(expr = "the item id for {word} {string} is written")]
fn item_id_is_written(world: &mut CodetagsWorld, kind: String, value: String) {
    world.names.item_id = Some(item_id(&kind, value).to_string());
}

#[then(expr = "the written id is {string}")]
fn the_written_id_is(world: &mut CodetagsWorld, expected: String) {
    let written = world.names.item_id.as_deref().expect("an id was written");
    assert_eq!(written, expected);
}

#[then(expr = "the written id reads back as {word} {string}")]
fn the_written_id_reads_back(world: &mut CodetagsWorld, kind: String, value: String) {
    let written = world.names.item_id.as_deref().expect("an id was written");
    let read = ItemId::from_str(written).unwrap_or_else(|e| panic!("{written:?}: {e}"));
    assert_eq!(read, item_id(&kind, value));
}

/// Reads a data table's rows as maps from header to cell.
fn rows(step: &Step) -> Vec<std::collections::HashMap<String, String>> {
    let table = step.table.as_ref().expect("the step has a data table");
    let (header, body) = table.rows.split_first().expect("the table has a header");
    body.iter()
        .map(|row| header.iter().cloned().zip(row.iter().cloned()).collect())
        .collect()
}

#[given(expr = "a directory holding these entries:")]
fn a_directory_holding(world: &mut CodetagsWorld, step: &Step) {
    world.names.entries = rows(step)
        .into_iter()
        .map(|row| (row["name"].clone(), row["id"].clone()))
        .collect();
}

fn suffixed(entries: &[(String, String)]) -> Vec<String> {
    let pairs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(name, id)| (name.as_str(), id.as_str()))
        .collect();
    apply_collision_suffixes(&pairs).unwrap_or_else(|e| panic!("{e}"))
}

#[when(expr = "collision suffixes are applied")]
fn collision_suffixes_are_applied(world: &mut CodetagsWorld) {
    world.names.named = suffixed(&world.names.entries);
}

#[then(expr = "the entries are named:")]
fn the_entries_are_named(world: &mut CodetagsWorld, step: &Step) {
    let expected = rows(step);
    assert_eq!(
        expected.len(),
        world.names.entries.len(),
        "one row per entry"
    );
    for row in expected {
        let index = world
            .names
            .entries
            .iter()
            .position(|(_, id)| *id == row["id"])
            .unwrap_or_else(|| panic!("no entry has id {:?}", row["id"]));
        assert_eq!(world.names.named[index], row["name"], "id {:?}", row["id"]);
    }
}

#[then(expr = "applying them to the entries in reverse order gives the same names")]
fn reverse_order_gives_the_same_names(world: &mut CodetagsWorld) {
    let mut reversed = world.names.entries.clone();
    reversed.reverse();
    let mut names = suffixed(&reversed);
    names.reverse();
    assert_eq!(names, world.names.named);
}
