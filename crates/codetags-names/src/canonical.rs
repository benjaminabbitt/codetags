//! Canonical symbol names (PLAN.md §2.7, D15).
//!
//! A canonical name is the dotted path of a symbol's SCIP descriptors,
//! outermost first, keeping the names' **real characters**:
//! `rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`]new().` is
//! `billing.Charge<T>.new`. Nothing is escaped, and a canonical name is
//! never decoded: the exact SCIP symbol stays in DuckDB, so a canonical
//! name only has to be unique, and [`CanonicalNames`] reports any two
//! symbols that share one.
//!
//! Mapping decisions:
//!
//! - **Package and scheme are dropped.** The name is the descriptor path
//!   alone (brief §4.5: `ordering.OrderRepo.save`). Two packages defining
//!   the same path therefore collide, and the collision is reported.
//! - **Namespaces, types, terms, methods, metas and macros** each become one
//!   segment carrying the descriptor's name. The suffix kind is dropped, so
//!   a macro `charge!` and a function `charge()` collide; that is reported.
//! - **Separators inside a name become `.`:** a package path's `/`
//!   (scip-go's `` `github.com/acme/billing`/ `` is
//!   `github.com.acme.billing`) and Rust's `::` (`` [`inner::Fee`] `` is
//!   `inner.Fee`). So no canonical name contains `/`.
//! - **A method disambiguator** follows its name: SCIP's `(+1)` becomes
//!   `+1`, so `apply2(+1).` is `apply2+1`. A disambiguator that does not
//!   start with `+` gets one. An empty `()` adds nothing.
//! - **Rust impl blocks read `Type.Trait.method`.** rust-analyzer names an
//!   impl block with a type descriptor `impl` followed by type-parameter
//!   descriptors for the self type and, for a trait impl, the trait:
//!   ``impl#[`Charge<T>`][Apply]apply().``. For the `rust-analyzer` scheme
//!   the `impl` segment is dropped, giving `Charge<T>.Apply.apply`, and an
//!   inherent method is `Charge<T>.new`. The self type keeps its generics
//!   as written, so it differs from the type's own name (`Charge`).
//! - **Other type parameters are canonical.** `[T]` becomes a segment `T`
//!   (scip-typescript's `Charge#[T]` is `Charge.T`).
//! - **Parameters and locals are not canonical symbols.** `local N`
//!   symbols are the discarded local scope (brief §4.4), and a symbol with
//!   any `(param)` descriptor is a function parameter, scoped like a local.
//!   [`canonical_name`] returns `None` for both.
//!
//! **Invariant:** a canonical name never contains `/` or a control
//! character. A descriptor name holding a control character has no
//! canonical spelling (it is never escaped), so it is an error.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::scip::{Descriptor, GlobalSymbol, Suffix, Symbol};

/// The scheme whose impl blocks get the `Type.Trait.method` rendering.
const RUST_ANALYZER: &str = "rust-analyzer";

/// One segment of a canonical name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Segment {
    /// The descriptor's unescaped name.
    pub name: String,
    /// A method's disambiguator as SCIP wrote it, e.g. `+1`.
    pub disambiguator: Option<String>,
}

impl Segment {
    /// This segment's text in a canonical name.
    fn render(&self) -> String {
        let mut out = self.name.clone();
        if let Some(d) = &self.disambiguator {
            if !d.starts_with('+') {
                out.push('+');
            }
            out.push_str(d);
        }
        out.replace("::", ".").replace('/', ".")
    }
}

/// Why a symbol has no canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// A descriptor name holds a control character, which a canonical name
    /// never contains and is never escaped.
    ControlCharacter {
        /// The SCIP symbol, as SCIP spells it.
        symbol: String,
    },
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalError::ControlCharacter { symbol } => write!(
                f,
                "SCIP symbol {symbol:?} has a control character in a name, \
                 so it has no canonical name"
            ),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// The canonical segments of `symbol`, outermost first, or `None` if the
/// symbol has no canonical name (a local or a parameter; see the module
/// documentation).
pub fn segments(symbol: &Symbol) -> Option<Vec<Segment>> {
    match symbol {
        Symbol::Local(_) => None,
        Symbol::Global(g) => global_segments(g),
    }
}

/// Returns `true` for rust-analyzer's impl-block descriptor: a type named
/// `impl` followed by a type parameter.
fn is_rust_impl(symbol: &GlobalSymbol, index: usize, d: &Descriptor) -> bool {
    symbol.scheme == RUST_ANALYZER
        && d.suffix == Suffix::Type
        && d.name == "impl"
        && symbol
            .descriptors
            .get(index + 1)
            .is_some_and(|next| next.suffix == Suffix::TypeParameter)
}

fn global_segments(symbol: &GlobalSymbol) -> Option<Vec<Segment>> {
    let mut out = Vec::with_capacity(symbol.descriptors.len());
    for (index, d) in symbol.descriptors.iter().enumerate() {
        if d.suffix == Suffix::Parameter {
            return None;
        }
        if is_rust_impl(symbol, index, d) {
            continue;
        }
        out.push(Segment {
            name: d.name.clone(),
            disambiguator: d.disambiguator.clone(),
        });
    }
    Some(out)
}

/// The canonical name of `symbol`, or `Ok(None)` for a local or a
/// parameter.
///
/// # Errors
///
/// [`CanonicalError::ControlCharacter`] if a name holds a control
/// character.
pub fn canonical_name(symbol: &Symbol) -> Result<Option<String>, CanonicalError> {
    let Some(segments) = segments(symbol) else {
        return Ok(None);
    };
    let name = segments
        .iter()
        .map(Segment::render)
        .collect::<Vec<_>>()
        .join(".");
    if name.chars().any(char::is_control) {
        return Err(CanonicalError::ControlCharacter {
            symbol: symbol.to_string(),
        });
    }
    Ok(Some(name))
}

/// Two or more distinct SCIP symbols with one canonical name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The shared canonical name.
    pub name: String,
    /// The symbols, as SCIP spells them, sorted.
    pub symbols: Vec<String>,
}

/// Canonical names assigned to a set of symbols, with every collision kept
/// and reported rather than merged. The result does not depend on the order
/// symbols are inserted in.
#[derive(Debug, Clone, Default)]
pub struct CanonicalNames {
    /// Canonical name to the distinct SCIP symbols that have it.
    by_name: BTreeMap<String, BTreeSet<String>>,
}

impl CanonicalNames {
    /// Records `symbol` and returns its canonical name, or `Ok(None)` for a
    /// symbol that has none (which is not recorded). Inserting the same
    /// symbol twice is not a collision.
    ///
    /// # Errors
    ///
    /// As [`canonical_name`].
    pub fn insert(&mut self, symbol: &Symbol) -> Result<Option<String>, CanonicalError> {
        let name = canonical_name(symbol)?;
        if let Some(name) = &name {
            self.by_name
                .entry(name.clone())
                .or_default()
                .insert(symbol.to_string());
        }
        Ok(name)
    }

    /// Every canonical name shared by more than one symbol, sorted by name.
    pub fn collisions(&self) -> Vec<Collision> {
        self.by_name
            .iter()
            .filter(|(_, symbols)| symbols.len() > 1)
            .map(|(name, symbols)| Collision {
                name: name.clone(),
                symbols: symbols.iter().cloned().collect(),
            })
            .collect()
    }

    /// The names that belong to exactly one symbol, as `(name, symbol)`,
    /// sorted by name. Colliding names are left out.
    pub fn unique(&self) -> impl Iterator<Item = (&str, &str)> {
        self.by_name.iter().filter_map(|(name, symbols)| {
            let mut iter = symbols.iter();
            match (iter.next(), iter.next()) {
                (Some(symbol), None) => Some((name.as_str(), symbol.as_str())),
                _ => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scip::{Package, parse};
    use proptest::prelude::*;

    fn name(s: &str) -> Option<String> {
        let symbol = parse(s).unwrap_or_else(|e| panic!("{s:?}: {e}"));
        canonical_name(&symbol).unwrap_or_else(|e| panic!("{e}"))
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn rust_impl_blocks_read_type_trait_method() {
        let p = "rust-analyzer cargo demo 0.1.0 ";
        assert_eq!(
            name(&format!("{p}billing/impl#[`Charge<T>`][Apply]apply().")),
            some("billing.Charge<T>.Apply.apply")
        );
        assert_eq!(
            name(&format!("{p}billing/impl#[`Charge<T>`]new().")),
            some("billing.Charge<T>.new")
        );
        assert_eq!(
            name(&format!("{p}billing/impl#[`inner::Fee`][Apply]apply().")),
            some("billing.inner.Fee.Apply.apply")
        );
        // Only rust-analyzer's impl blocks are rewritten.
        assert_eq!(name("other m n v impl#[T]f()."), some("impl.T.f"));
    }

    #[test]
    fn package_paths_and_disambiguators() {
        assert_eq!(
            name(
                "scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/New()."
            ),
            some("github.com.acme.billing.billing.New")
        );
        assert_eq!(name("s m n v C#m(+2)."), some("C.m+2"));
        assert_eq!(name("s m n v C#m(x)."), some("C.m+x"));
    }

    #[test]
    fn locals_and_parameters_have_none() {
        assert_eq!(name("local 3"), None);
        assert_eq!(name("scip-go gomod demo v1 `demo/x`/F().(ctx)"), None);
    }

    #[test]
    fn control_characters_are_an_error() {
        let symbol = parse("s m n v `a\u{1}b`#").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            canonical_name(&symbol),
            Err(CanonicalError::ControlCharacter { .. })
        ));
    }

    #[test]
    fn collisions_are_reported_not_merged() {
        let mut names = CanonicalNames::default();
        for s in [
            "s m a v x/Y#",
            "s m b v x/Y#",
            "s m a v x/Y#",
            "s m a v x/Z#",
            "local 1",
        ] {
            names
                .insert(&parse(s).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(
            names.collisions(),
            vec![Collision {
                name: "x.Y".into(),
                symbols: vec!["s m a v x/Y#".into(), "s m b v x/Y#".into()],
            }]
        );
        assert_eq!(
            names.unique().collect::<Vec<_>>(),
            vec![("x.Z", "s m a v x/Z#")]
        );
    }

    fn descriptor() -> impl Strategy<Value = Descriptor> {
        let suffix = prop_oneof![
            Just(Suffix::Namespace),
            Just(Suffix::Type),
            Just(Suffix::Term),
            Just(Suffix::Method),
            Just(Suffix::TypeParameter),
            Just(Suffix::Meta),
            Just(Suffix::Macro),
        ];
        // Printable names, `/` and `::` included; no control characters.
        let name = prop_oneof!["impl", "[a-z/:<>$#]{1,8}", "\\PC{1,8}"];
        (name.clone(), suffix, proptest::option::of(name)).prop_map(|(name, suffix, d)| {
            Descriptor {
                name,
                suffix,
                disambiguator: if suffix == Suffix::Method { d } else { None },
            }
        })
    }

    proptest! {
        #[test]
        fn names_never_hold_a_slash_or_a_control_character(
            scheme in prop_oneof![Just("rust-analyzer".to_string()), "[a-k]{1,4}".prop_map(String::from)],
            descriptors in proptest::collection::vec(descriptor(), 1..6),
        ) {
            let symbol = Symbol::Global(GlobalSymbol {
                scheme,
                package: Package::default(),
                descriptors,
            });
            let name = canonical_name(&symbol);
            prop_assert!(name.is_ok(), "{:?}", name);
            if let Ok(Some(name)) = name {
                prop_assert!(!name.contains('/'), "{name:?}");
                prop_assert!(!name.chars().any(char::is_control), "{name:?}");
            }
        }

        #[test]
        fn collisions_do_not_depend_on_insertion_order(
            descriptors in proptest::collection::vec(proptest::collection::vec(descriptor(), 1..3), 1..12),
        ) {
            let symbols: Vec<Symbol> = descriptors
                .into_iter()
                .map(|descriptors| Symbol::Global(GlobalSymbol {
                    scheme: "s".into(),
                    package: Package::default(),
                    descriptors,
                }))
                .collect();
            let mut forward = CanonicalNames::default();
            let mut backward = CanonicalNames::default();
            for symbol in &symbols {
                prop_assert!(forward.insert(symbol).is_ok());
            }
            for symbol in symbols.iter().rev() {
                prop_assert!(backward.insert(symbol).is_ok());
            }
            prop_assert_eq!(forward.collisions(), backward.collisions());
        }
    }
}
