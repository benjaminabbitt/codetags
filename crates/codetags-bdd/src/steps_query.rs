//! Steps for tagma queries. The wording deliberately matches tagma's own
//! conformance steps (tagma `docs/steps.md`).

use cucumber::{given, then, when};
use tagma_core::Tag;

use crate::CodetagsWorld;

#[given(expr = "an item {string} tagged {string}")]
fn an_item_tagged(world: &mut CodetagsWorld, id: String, tags: String) {
    let tags = tags
        .split_whitespace()
        .map(|tag| Tag::parse(tag).unwrap_or_else(|e| panic!("invalid tag {tag:?}: {e}")))
        .collect();
    world.tagma.add_item(&id, tags);
}

#[when(expr = "the postfix query {string} is run")]
fn the_postfix_query_is_run(world: &mut CodetagsWorld, query: String) {
    world.matched = Some(world.tagma.query_postfix(&query));
}

#[then(expr = "it matches exactly {string}")]
fn it_matches_exactly(world: &mut CodetagsWorld, expected: String) {
    let matched = world.matched.as_ref().expect("a query ran");
    let mut actual = matched
        .clone()
        .unwrap_or_else(|e| panic!("query failed: {e}"));
    actual.sort();
    let mut expected: Vec<String> = expected.split_whitespace().map(str::to_string).collect();
    expected.sort();
    assert_eq!(actual, expected);
}

#[then(expr = "the query fails")]
fn the_query_fails(world: &mut CodetagsWorld) {
    let matched = world.matched.as_ref().expect("a query ran");
    assert!(matched.is_err(), "expected an error, got {matched:?}");
}
