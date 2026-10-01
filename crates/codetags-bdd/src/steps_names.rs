//! Steps for `features/names/`: the path profile, item ids, and collision
//! suffixes (PLAN.md §2.3, §2.7).

use std::str::FromStr;

use codetags_names::collision::apply_collision_suffixes;
use codetags_names::item_id::ItemId;
use codetags_names::path_profile::{decode_component, encode_component};
use codetags_names::platform::{Platform, is_legal_filename};
use cucumber::gherkin::Step;
use cucumber::{Parameter, given, then, when};

use crate::CodetagsWorld;

/// State the names steps share within one scenario.
#[derive(Debug, Default)]
pub struct NamesState {
    /// The last path component the encoder produced.
    component: Option<String>,
    /// The last item id written.
    item_id: Option<String>,
    /// Directory entries as `(name, id)`, in table order.
    entries: Vec<(String, String)>,
    /// The names collision suffixing produced, aligned with `entries`.
    named: Vec<String>,
}

/// `{platforms}`: one or more of `linux`, `macos`, `windows`, joined by
/// `, ` or ` and `.
#[derive(Debug, Parameter)]
#[param(
    name = "platforms",
    regex = "(?:linux|macos|windows)(?:(?:, | and )(?:linux|macos|windows))*"
)]
struct Platforms(Vec<Platform>);

impl FromStr for Platforms {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.replace(" and ", ", ")
            .split(", ")
            .map(Platform::from_str)
            .collect::<Result<_, _>>()
            .map(Platforms)
    }
}

/// `{kind}` of an item id: `file` or `symbol`.
fn item_id(kind: &str, value: String) -> ItemId {
    match kind {
        "file" => ItemId::File(value),
        "symbol" => ItemId::Symbol(value),
        other => panic!("unknown item kind {other:?}: use file or symbol"),
    }
}

fn component(world: &CodetagsWorld) -> &str {
    world
        .names
        .component
        .as_deref()
        .expect("an earlier step encoded a component")
}

#[when(expr = "the query element {string} is encoded as a path component")]
fn element_is_encoded(world: &mut CodetagsWorld, element: String) {
    let encoded = encode_component(&element).unwrap_or_else(|e| panic!("{element:?}: {e}"));
    world.names.component = Some(encoded);
}

#[then(expr = "the path component is {string}")]
fn the_component_is(world: &mut CodetagsWorld, expected: String) {
    assert_eq!(component(world), expected);
}

#[then(expr = "the path component is a legal file name on {platforms}")]
fn the_component_is_legal(world: &mut CodetagsWorld, platforms: Platforms) {
    let name = component(world);
    for platform in platforms.0 {
        assert!(
            is_legal_filename(name, platform),
            "{name:?} is not a legal file name on {platform}"
        );
    }
}

#[then(expr = "the path component decodes to {string} on {platforms}")]
fn the_component_decodes_to(world: &mut CodetagsWorld, element: String, platforms: Platforms) {
    let name = component(world).to_string();
    decodes_to(&name, &element, platforms);
}

#[then(expr = "the path component {string} decodes to {string} on {platforms}")]
fn the_named_component_decodes_to(
    _world: &mut CodetagsWorld,
    name: String,
    element: String,
    platforms: Platforms,
) {
    decodes_to(&name, &element, platforms);
}

fn decodes_to(name: &str, element: &str, platforms: Platforms) {
    for platform in platforms.0 {
        let decoded = decode_component(name, platform)
            .unwrap_or_else(|e| panic!("{name:?} on {platform}: {e}"));
        assert_eq!(decoded, element, "decoding {name:?} on {platform}");
    }
}

#[then(expr = "the path component {string} does not decode on {platforms}")]
fn does_not_decode(_world: &mut CodetagsWorld, name: String, platforms: Platforms) {
    for platform in platforms.0 {
        let decoded = decode_component(&name, platform);
        assert!(
            decoded.is_err(),
            "{name:?} decoded on {platform} to {decoded:?}"
        );
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
