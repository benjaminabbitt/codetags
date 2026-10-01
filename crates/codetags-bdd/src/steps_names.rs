//! Steps for `features/names/`: canonical names, the Windows private-use
//! mapping, item ids, and collision suffixes (PLAN.md §2.3, §2.7, D15).

use std::str::FromStr;

use codetags_names::canonical::{CanonicalNames, canonical_name};
use codetags_names::collision::apply_collision_suffixes;
use codetags_names::item_id::ItemId;
use codetags_names::scip::{self, Symbol};
use codetags_names::windows::{from_windows_name, to_windows_name};
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
    /// The last canonical name taken: `None` for a symbol with none.
    canonical: Option<Option<String>>,
    /// SCIP symbols a table listed.
    symbols: Vec<Symbol>,
    /// The canonical names assigned to `symbols`.
    assigned: Option<CanonicalNames>,
    /// The last name mapped for Windows.
    windows: Option<String>,
}

fn parse_symbol(text: &str) -> Symbol {
    scip::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

#[when(expr = "the canonical name of the SCIP symbol {string} is taken")]
fn canonical_name_is_taken(world: &mut CodetagsWorld, symbol: String) {
    let name = canonical_name(&parse_symbol(&symbol)).unwrap_or_else(|e| panic!("{e}"));
    world.names.canonical = Some(name);
}

fn taken(world: &CodetagsWorld) -> Option<&str> {
    world
        .names
        .canonical
        .as_ref()
        .expect("an earlier step took a canonical name")
        .as_deref()
}

#[then(expr = "the canonical name is {string}")]
fn the_canonical_name_is(world: &mut CodetagsWorld, expected: String) {
    assert_eq!(taken(world), Some(expected.as_str()));
}

#[then(expr = "the symbol has no canonical name")]
fn the_symbol_has_no_canonical_name(world: &mut CodetagsWorld) {
    assert_eq!(taken(world), None);
}

#[given(expr = "these SCIP symbols:")]
fn these_scip_symbols(world: &mut CodetagsWorld, step: &Step) {
    world.names.symbols = rows(step)
        .iter()
        .map(|row| parse_symbol(&row["symbol"]))
        .collect();
}

#[when(expr = "their canonical names are assigned")]
fn their_canonical_names_are_assigned(world: &mut CodetagsWorld) {
    let mut names = CanonicalNames::default();
    for symbol in &world.names.symbols {
        names.insert(symbol).unwrap_or_else(|e| panic!("{e}"));
    }
    world.names.assigned = Some(names);
}

/// The reported collisions as sorted `(name, symbol)` pairs.
fn collisions(world: &CodetagsWorld) -> Vec<(String, String)> {
    let names = world.names.assigned.as_ref().expect("names were assigned");
    let mut pairs: Vec<(String, String)> = names
        .collisions()
        .into_iter()
        .flat_map(|c| {
            let name = c.name;
            c.symbols.into_iter().map(move |s| (name.clone(), s))
        })
        .collect();
    pairs.sort();
    pairs
}

#[then(expr = "the collisions are:")]
fn the_collisions_are(world: &mut CodetagsWorld, step: &Step) {
    let mut expected: Vec<(String, String)> = rows(step)
        .into_iter()
        .map(|row| (row["name"].clone(), row["symbol"].clone()))
        .collect();
    expected.sort();
    assert_eq!(collisions(world), expected);
}

#[then(expr = "there are no collisions")]
fn there_are_no_collisions(world: &mut CodetagsWorld) {
    assert_eq!(collisions(world), vec![]);
}

/// Expands `{U+XXXX}` to that code point, so features can show private-use
/// characters.
fn expand_code_points(text: &str) -> String {
    let re = regex::Regex::new(r"\{U\+([0-9A-Fa-f]{4,6})\}").expect("valid regex");
    re.replace_all(text, |caps: &regex::Captures<'_>| {
        let code = u32::from_str_radix(&caps[1], 16).expect("hex digits");
        char::from_u32(code)
            .expect("a Unicode scalar value")
            .to_string()
    })
    .into_owned()
}

#[when(expr = "the name {string} is mapped for Windows")]
fn the_name_is_mapped_for_windows(world: &mut CodetagsWorld, name: String) {
    world.names.windows = Some(to_windows_name(&name));
}

fn windows(world: &CodetagsWorld) -> &str {
    world
        .names
        .windows
        .as_deref()
        .expect("an earlier step mapped a name")
}

#[then(expr = "the Windows name is {string}")]
fn the_windows_name_is(world: &mut CodetagsWorld, expected: String) {
    assert_eq!(windows(world), expand_code_points(&expected));
}

#[then(expr = "the Windows name maps back to {string}")]
fn the_windows_name_maps_back_to(world: &mut CodetagsWorld, expected: String) {
    assert_eq!(from_windows_name(windows(world)), expected);
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
