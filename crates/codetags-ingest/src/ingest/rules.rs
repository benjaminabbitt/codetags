//! What a symbol is: its kind, whether it is callable, how a call to it
//! dispatches, and its modules.
//!
//! The provider's `SymbolInformation.kind` is used when the index holds one
//! (symbols defined in the project); otherwise the descriptors decide.

use codetags_names::canonical::{CanonicalError, canonical_name};
use codetags_names::scip::{Descriptor, GlobalSymbol, Suffix, Symbol};

use crate::provider::scip::SymbolKind;

/// How a call reaches its target (the brief's `dispatch` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// The target is the code that runs: a free function, a method of an
    /// impl block (inherent or trait impl), or a macro.
    Static,
    /// The target is a trait's method: which implementation runs depends on
    /// the receiver's type, which SCIP does not record. This covers `dyn`
    /// calls (a vtable) and calls on generic or concrete receivers
    /// (monomorphized) alike.
    Virtual,
    /// Through a function pointer or closure. SCIP ingest never writes it:
    /// such calls reference a local (V65), and locals are filtered.
    Dynamic,
    /// Nothing tells which: any scheme other than rust-analyzer's, and
    /// descriptor shapes the rules below do not cover.
    Unknown,
}

impl Dispatch {
    /// The column value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Virtual => "virtual",
            Self::Dynamic => "dynamic",
            Self::Unknown => "unknown",
        }
    }
}

/// The scheme whose naming rules the dispatch rules rely on (V33, V64).
const RUST_ANALYZER: &str = "rust-analyzer";

/// Kinds that name something a reference can call.
const CALLABLE_KINDS: &[SymbolKind] = &[
    SymbolKind::AbstractMethod,
    SymbolKind::Constructor,
    SymbolKind::Function,
    SymbolKind::Getter,
    SymbolKind::Macro,
    SymbolKind::Method,
    SymbolKind::MethodAlias,
    SymbolKind::MethodSpecification,
    SymbolKind::ProtocolMethod,
    SymbolKind::PureVirtualMethod,
    SymbolKind::Setter,
    SymbolKind::SingletonMethod,
    SymbolKind::StaticMethod,
    SymbolKind::TraitMethod,
    SymbolKind::TypeClassMethod,
];

/// The last descriptor, which says what the symbol itself is.
fn last(symbol: &GlobalSymbol) -> Option<&Descriptor> {
    symbol.descriptors.last()
}

/// The descriptor before the last.
fn parent(symbol: &GlobalSymbol) -> Option<&Descriptor> {
    let n = symbol.descriptors.len();
    n.checked_sub(2).and_then(|i| symbol.descriptors.get(i))
}

/// Whether the symbol sits in a rust-analyzer impl block: a type descriptor
/// `impl` followed by type parameters (V33, V38).
fn in_rust_impl(symbol: &GlobalSymbol) -> bool {
    symbol.descriptors.windows(2).any(|pair| {
        pair[0].suffix == Suffix::Type
            && pair[0].name == "impl"
            && pair[1].suffix == Suffix::TypeParameter
    })
}

/// The `kind` column: the provider's kind in snake case, or one derived from
/// the descriptors.
pub fn kind(symbol: &GlobalSymbol, info: Option<SymbolKind>) -> String {
    if let Some(kind) = info {
        return snake_case(&format!("{kind:?}"));
    }
    let derived = match last(symbol).map(|d| d.suffix) {
        Some(Suffix::Namespace) => "namespace",
        Some(Suffix::Type) => "type",
        Some(Suffix::Term) => "term",
        Some(Suffix::Meta) => "meta",
        Some(Suffix::Macro) => "macro",
        Some(Suffix::TypeParameter) => "type_parameter",
        Some(Suffix::Parameter) => "parameter",
        Some(Suffix::Method) => match parent(symbol).map(|d| d.suffix) {
            None | Some(Suffix::Namespace) => "function",
            Some(_) => "method",
        },
        None => "unknown",
    };
    derived.to_string()
}

fn snake_case(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether the symbol is a module: no reference is attributed to a module,
/// so imports and re-exports are not call sites.
pub fn is_module(symbol: &GlobalSymbol, info: Option<SymbolKind>) -> bool {
    match info {
        Some(kind) => matches!(
            kind,
            SymbolKind::Module | SymbolKind::Namespace | SymbolKind::Package | SymbolKind::File
        ),
        None => last(symbol).is_some_and(|d| d.suffix == Suffix::Namespace),
    }
}

/// Whether a reference to the symbol is a call site: a function, method or
/// macro, by kind when known and by descriptor otherwise.
pub fn is_callable(symbol: &GlobalSymbol, info: Option<SymbolKind>) -> bool {
    match info {
        Some(kind) => CALLABLE_KINDS.contains(&kind),
        None => last(symbol).is_some_and(|d| matches!(d.suffix, Suffix::Method | Suffix::Macro)),
    }
}

/// Whether the symbol is a macro.
pub fn is_macro(symbol: &GlobalSymbol, info: Option<SymbolKind>) -> bool {
    info == Some(SymbolKind::Macro) || last(symbol).is_some_and(|d| d.suffix == Suffix::Macro)
}

/// How a call to the symbol dispatches. Only rust-analyzer's naming is
/// relied on (V33, V64): there, an inherent or trait-impl method lives in an
/// `impl#[Self][Trait]` block, a free function under a namespace, and a
/// method directly under a type is a trait's method. Everything else is
/// [`Dispatch::Unknown`].
pub fn dispatch(symbol: &GlobalSymbol) -> Dispatch {
    if symbol.scheme != RUST_ANALYZER {
        return Dispatch::Unknown;
    }
    match last(symbol).map(|d| d.suffix) {
        Some(Suffix::Macro) => Dispatch::Static,
        Some(Suffix::Method) => {
            if in_rust_impl(symbol) {
                return Dispatch::Static;
            }
            match parent(symbol).map(|d| d.suffix) {
                None | Some(Suffix::Namespace) => Dispatch::Static,
                Some(Suffix::Type) => Dispatch::Virtual,
                Some(_) => Dispatch::Unknown,
            }
        }
        _ => Dispatch::Unknown,
    }
}

/// The modules enclosing the symbol, outermost first, as dotted canonical
/// names: the leading namespace descriptors before the symbol's own, so
/// `a/b/f().` is in `a` and `a.b`, and the module `a/b/` is in `a`. A Rust
/// module's canonical name starts with its crate (`billing.a`, P1.3c); the
/// crate root itself is not listed, so a symbol at the root has no module.
pub fn modules(symbol: &GlobalSymbol) -> Result<Vec<String>, CanonicalError> {
    let own = symbol.descriptors.len().saturating_sub(1);
    let depth = symbol.descriptors[..own]
        .iter()
        .take_while(|d| d.suffix == Suffix::Namespace)
        .count();
    let mut out = Vec::with_capacity(depth);
    for n in 1..=depth {
        let prefix = Symbol::Global(GlobalSymbol {
            scheme: symbol.scheme.clone(),
            package: symbol.package.clone(),
            descriptors: symbol.descriptors[..n].to_vec(),
        });
        if let Some(name) = canonical_name(&prefix)? {
            out.push(name);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use codetags_names::scip::parse;

    use super::*;

    fn global(descriptors: &str) -> GlobalSymbol {
        global_in("rust-analyzer", descriptors)
    }

    fn global_in(scheme: &str, descriptors: &str) -> GlobalSymbol {
        match parse(&format!("{scheme} cargo billing 0.1.0 {descriptors}")).unwrap() {
            Symbol::Global(g) => g,
            Symbol::Local(_) => panic!("local"),
        }
    }

    #[test]
    fn rust_dispatch_follows_rust_analyzer_naming() {
        let cases = [
            ("charge/double().", Dispatch::Static),
            ("settle().", Dispatch::Static),
            ("charge/impl#[Fee][Apply]apply().", Dispatch::Static),
            ("ledger/impl#[Ledger]record().", Dispatch::Static),
            ("macros/log_line!", Dispatch::Static),
            ("charge/Apply#apply().", Dispatch::Virtual),
            ("iter/traits/iterator/Iterator#map().", Dispatch::Virtual),
            ("charge/Charge#amount.", Dispatch::Unknown),
            ("a/B#[T]f().", Dispatch::Unknown),
        ];
        for (descriptors, expected) in cases {
            assert_eq!(dispatch(&global(descriptors)), expected, "{descriptors}");
        }
    }

    #[test]
    fn other_schemes_dispatch_unknown() {
        assert_eq!(
            dispatch(&global_in("scip-go", "billing/New().")),
            Dispatch::Unknown
        );
    }

    #[test]
    fn kinds_come_from_the_provider_or_the_descriptors() {
        assert_eq!(
            kind(
                &global("charge/Apply#apply()."),
                Some(SymbolKind::TraitMethod)
            ),
            "trait_method"
        );
        assert_eq!(kind(&global("charge/double()."), None), "function");
        assert_eq!(kind(&global("boxed/impl#[`Box<T>`]new()."), None), "method");
        assert_eq!(kind(&global("macros/vec!"), None), "macro");
        assert_eq!(kind(&global("vec/Vec#"), None), "type");
    }

    #[test]
    fn callables_are_functions_methods_and_macros() {
        assert!(is_callable(&global("charge/double()."), None));
        assert!(is_callable(&global("macros/vec!"), None));
        assert!(!is_callable(&global("vec/Vec#"), None));
        assert!(!is_callable(&global("charge/Fee#0."), None));
        assert!(!is_callable(&global("default/Default:"), None));
        // A provider kind overrides the descriptor.
        assert!(is_callable(
            &global("x/Applier#Apply."),
            Some(SymbolKind::Method)
        ));
        assert!(!is_callable(&global("x/f()."), Some(SymbolKind::Struct)));
    }

    #[test]
    fn modules_are_the_leading_namespaces_before_the_symbol() {
        // Canonical names, so they start with the crate (P1.3c); the crate
        // root itself is not listed.
        assert_eq!(
            modules(&global("a/b/f().")).unwrap(),
            ["billing.a", "billing.a.b"]
        );
        assert_eq!(modules(&global("a/b/")).unwrap(), ["billing.a"]);
        assert_eq!(modules(&global("crate/")).unwrap(), Vec::<String>::new());
        assert_eq!(modules(&global("settle().")).unwrap(), Vec::<String>::new());
        assert_eq!(
            modules(&global("charge/impl#[`Charge<T>`][Apply]apply().")).unwrap(),
            ["billing.charge"]
        );
        assert!(is_module(&global("a/b/"), None));
        assert!(!is_module(&global("a/b/f()."), None));
    }
}
