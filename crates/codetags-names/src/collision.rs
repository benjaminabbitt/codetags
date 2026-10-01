//! Case-fold collision suffixes (PLAN.md §2.7).
//!
//! A materialized tree may land on a case-insensitive file system (APFS
//! and NTFS by default). Within one directory, the names that collide
//! under case folding, and only those, get the suffix `~` plus 6 lower-case
//! hex digits of a hash of the entry's item id.
//!
//! - **Stable.** The hash is 64-bit FNV-1a over the id's UTF-8 bytes,
//!   folded to 24 bits. It is part of the on-disk naming, so it must never
//!   change.
//! - **Order-independent.** Entries are resolved in id order, so the result
//!   does not depend on the order of the input.
//! - **Collision-free.** If a suffixed name still collides (with a name
//!   that needed no suffix, or two ids whose hashes agree), the entry
//!   tries the hash of its id with an attempt number appended, until the
//!   name is free.
//!
//! Case folding here is Unicode lower-casing (`str::to_lowercase`). It is
//! close to, not identical with, NTFS's upcase table and APFS's folding,
//! and it ignores Unicode normalization (macOS treats NFC and NFD
//! spellings as one name).
//!
//! The suffix is appended to the whole name given. Callers that want it
//! before an extension (`Order~1a2b3c.sym`) pass the stem and add the
//! extension afterwards.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Why suffixes could not be assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollisionError {
    /// Two entries in one directory have this same id.
    DuplicateId(String),
}

impl fmt::Display for CollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CollisionError::DuplicateId(id) => {
                write!(f, "two entries in one directory have the id {id:?}")
            }
        }
    }
}

impl std::error::Error for CollisionError {}

/// The case-folded form two names collide under.
pub fn case_fold(name: &str) -> String {
    name.to_lowercase()
}

/// The 6-hex-digit suffix body for `id` on the given attempt (0 first).
pub fn suffix_hash(id: &str, attempt: u32) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    feed(id.as_bytes());
    if attempt > 0 {
        feed(&[0xFF]);
        feed(&attempt.to_le_bytes());
    }
    let folded = (hash ^ (hash >> 24) ^ (hash >> 48)) & 0xFF_FFFF;
    format!("{folded:06x}")
}

/// Gives each `(name, id)` entry of one directory its final name, in input
/// order. Names that collide with another under [`case_fold`] get
/// `~<6 hex>`; every other name is returned unchanged.
///
/// # Errors
///
/// [`CollisionError::DuplicateId`] if two entries share an id.
pub fn apply_collision_suffixes(entries: &[(&str, &str)]) -> Result<Vec<String>, CollisionError> {
    let mut by_id = BTreeMap::new();
    for (index, (_, id)) in entries.iter().enumerate() {
        if by_id.insert(*id, index).is_some() {
            return Err(CollisionError::DuplicateId((*id).to_string()));
        }
    }
    let folds: Vec<String> = entries.iter().map(|(name, _)| case_fold(name)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for fold in &folds {
        *counts.entry(fold).or_default() += 1;
    }
    let collides = |index: usize| counts[folds[index].as_str()] > 1;

    let mut names: Vec<String> = entries.iter().map(|(name, _)| name.to_string()).collect();
    let mut taken: HashSet<String> = (0..entries.len())
        .filter(|&i| !collides(i))
        .map(|i| folds[i].clone())
        .collect();
    // BTreeMap iterates in id order, which makes the result order-independent.
    for (id, index) in by_id {
        if !collides(index) {
            continue;
        }
        let name = entries[index].0;
        let mut attempt = 0;
        loop {
            let candidate = format!("{name}~{}", suffix_hash(id, attempt));
            if taken.insert(case_fold(&candidate)) {
                names[index] = candidate;
                break;
            }
            attempt += 1;
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn suffix_hash_is_stable() {
        // Pinned: these spellings are on-disk names and must never change.
        assert_eq!(suffix_hash("", 0), "be0c53");
        assert_eq!(suffix_hash("sym:billing.Order", 0), "d2e75b");
        assert_eq!(suffix_hash("sym:billing.Order", 0).len(), 6);
        assert_ne!(suffix_hash("x", 0), suffix_hash("x", 1));
    }

    #[test]
    fn only_colliding_names_change() {
        let names = apply_collision_suffixes(&[("A", "1"), ("a", "2"), ("b", "3")]).unwrap();
        assert_eq!(names[0], format!("A~{}", suffix_hash("1", 0)));
        assert_eq!(names[1], format!("a~{}", suffix_hash("2", 0)));
        assert_eq!(names[2], "b");
    }

    #[test]
    fn a_suffixed_name_never_takes_an_existing_one() {
        let taken = format!("a~{}", suffix_hash("1", 0));
        let names =
            apply_collision_suffixes(&[("A", "1"), ("a", "2"), (taken.as_str(), "3")]).unwrap();
        assert_eq!(names[0], format!("A~{}", suffix_hash("1", 1)));
        assert_eq!(names[2], taken);
    }

    #[test]
    fn duplicate_ids_are_an_error() {
        assert_eq!(
            apply_collision_suffixes(&[("a", "1"), ("b", "1")]),
            Err(CollisionError::DuplicateId("1".into()))
        );
    }

    fn entries() -> impl Strategy<Value = Vec<(String, String)>> {
        // Few letters, so collisions are common.
        proptest::collection::btree_map("[a-z0-9]{1,6}", "[aAbB~]{1,3}", 0..20).prop_map(|m| {
            m.into_iter()
                .map(|(id, name)| (name, format!("sym:{id}")))
                .collect()
        })
    }

    fn run(entries: &[(String, String)]) -> Vec<String> {
        let pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(n, i)| (n.as_str(), i.as_str()))
            .collect();
        apply_collision_suffixes(&pairs).unwrap()
    }

    proptest! {
        #[test]
        fn no_case_fold_collisions_remain(entries in entries()) {
            let names = run(&entries);
            let folds: HashSet<String> = names.iter().map(|n| case_fold(n)).collect();
            prop_assert_eq!(folds.len(), names.len());
            for ((original, _), name) in entries.iter().zip(&names) {
                let collided = entries
                    .iter()
                    .filter(|(other, _)| case_fold(other) == case_fold(original))
                    .count() > 1;
                prop_assert_eq!(collided, name != original);
            }
        }

        #[test]
        fn input_order_does_not_matter(entries in entries(), seed in any::<u64>()) {
            let names = run(&entries);
            let mut shuffled: Vec<(usize, (String, String))> =
                entries.iter().cloned().enumerate().collect();
            // Deterministic shuffle from the seed.
            shuffled.sort_by_key(|(i, _)| (*i as u64).wrapping_mul(seed | 1).rotate_left(17));
            let reordered: Vec<(String, String)> =
                shuffled.iter().map(|(_, e)| e.clone()).collect();
            let renamed = run(&reordered);
            for ((index, _), name) in shuffled.iter().zip(renamed) {
                prop_assert_eq!(&names[*index], &name);
            }
        }
    }
}
