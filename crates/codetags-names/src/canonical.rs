//! Which SCIP descriptors make up a canonical symbol name (PLAN.md §2.7).
//!
//! A canonical name is the dotted path of a symbol's descriptors,
//! outermost first: `rust-analyzer cargo demo 0.1.0 billing/Charge#new().`
//! has the segments `billing`, `Charge`, `new`. This module decides the
//! *segments*; spelling them as one string is a separate step.
//!
//! Mapping decisions:
//!
//! - **Package and scheme are dropped.** The name is the descriptor path
//!   alone (brief §4.5: `ordering.OrderRepo.save`). The package stays
//!   available on the parsed [`GlobalSymbol`].
//! - **Namespaces, types, terms, methods, metas and macros** each become one
//!   segment carrying the descriptor's name. The suffix kind is dropped, so
//!   a module and a function of the same name in the same scope give the
//!   same segment path; the `kind` facet tells them apart.
//! - **A method disambiguator** is kept on its segment: SCIP's `(+1)` is
//!   spelled `+1` after the name. An empty `()` has none.
//! - **Type parameters are canonical.** `[T]` becomes a segment `T`, both
//!   as the last descriptor (a generic parameter is a symbol a query may
//!   name) and in the middle, where rust-analyzer uses them to name impl
//!   blocks: `impl#[`Charge<T>`][Apply]apply().` has the segments `impl`,
//!   `Charge<T>`, `Apply`, `apply`, so methods of different impls differ.
//! - **Parameters and locals are not canonical symbols.** `local N`
//!   symbols are the discarded local scope (brief §4.4), and a symbol with
//!   any `(param)` descriptor is a function parameter, scoped like a local.
//!   [`segments`] returns `None` for both.

use crate::scip::{GlobalSymbol, Suffix, Symbol};

/// One segment of a canonical name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Segment {
    /// The descriptor's unescaped name.
    pub name: String,
    /// A method's disambiguator as SCIP wrote it, e.g. `+1`.
    pub disambiguator: Option<String>,
}

/// The canonical segments of `symbol`, outermost first, or `None` if the
/// symbol has no canonical name (a local or a parameter; see the module
/// documentation).
pub fn segments(symbol: &Symbol) -> Option<Vec<Segment>> {
    match symbol {
        Symbol::Local(_) => None,
        Symbol::Global(g) => global_segments(g),
    }
}

fn global_segments(symbol: &GlobalSymbol) -> Option<Vec<Segment>> {
    symbol
        .descriptors
        .iter()
        .map(|d| match d.suffix {
            Suffix::Parameter => None,
            _ => Some(Segment {
                name: d.name.clone(),
                disambiguator: d.disambiguator.clone(),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scip::parse;

    fn names(s: &str) -> Option<Vec<String>> {
        let symbol = parse(s).unwrap_or_else(|e| panic!("{s:?}: {e}"));
        segments(&symbol).map(|segs| {
            segs.into_iter()
                .map(|seg| match seg.disambiguator {
                    Some(d) => format!("{}|{d}", seg.name),
                    None => seg.name,
                })
                .collect()
        })
    }

    fn v(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn descriptors_become_segments() {
        assert_eq!(
            names("rust-analyzer cargo demo 0.1.0 billing/Charge#new()."),
            v(&["billing", "Charge", "new"])
        );
        assert_eq!(
            names("rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`][Apply]apply()."),
            v(&["billing", "impl", "Charge<T>", "Apply", "apply"])
        );
        assert_eq!(
            names("rust-analyzer cargo demo 0.1.0 charge!"),
            v(&["charge"])
        );
        assert_eq!(
            names("scip-python python demo 0.1 `demo.util`/__init__:"),
            v(&["demo.util", "__init__"])
        );
    }

    #[test]
    fn disambiguators_are_kept() {
        assert_eq!(
            names("scip-typescript npm demo 1.0.0 src/`a.ts`/C#m(+1)."),
            v(&["src", "a.ts", "C", "m|+1"])
        );
    }

    #[test]
    fn type_parameters_are_canonical() {
        assert_eq!(
            names("rust-analyzer cargo demo 0.1.0 billing/Charge#[T]"),
            v(&["billing", "Charge", "T"])
        );
    }

    #[test]
    fn locals_and_parameters_are_not() {
        assert_eq!(names("local 3"), None);
        assert_eq!(names("scip-go gomod demo v1 `demo/x`/F().(ctx)"), None);
    }
}
