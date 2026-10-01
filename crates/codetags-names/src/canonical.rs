//! Canonical symbol names (PLAN.md §2.7, D15).
//!
//! A canonical name is the dotted path of a symbol's SCIP descriptors,
//! outermost first, keeping the names' **real characters**:
//! `rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`]new().` is
//! `demo.billing.Charge<T>.new`. Nothing is escaped, and a canonical name is
//! never decoded: the exact SCIP symbol stays in DuckDB, so a canonical
//! name only has to be unique, and [`CanonicalNames`] reports any two
//! symbols that share one.
//!
//! Mapping decisions:
//!
//! - **Package and scheme are dropped,** except for Rust. The name is the
//!   descriptor path alone (brief §4.5: `ordering.OrderRepo.save`). Go
//!   descriptors already carry the import path, and TypeScript and Python
//!   descriptors the module path; two packages defining the same path
//!   collide, and the collision is reported.
//! - **A Rust name starts with its crate** (P1.3c; a default pending human
//!   review), as Rust's own full paths do (`crate::module::Item`): the SCIP
//!   package name with `-` mapped to `_`, as rustc does. So
//!   `rust-analyzer cargo codetags-model 0.1.0 store/GenerationStore#` is
//!   `codetags_model.store.GenerationStore`, and the crate's root module,
//!   `crate/`, is `codetags_model`. This holds for every Rust symbol, not
//!   only on a collision, so names are stable; without it, every crate's
//!   `crate/` and every binary's `main()` collided (V98). A symbol with an
//!   empty package name gets no prefix.
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
//! - **A Rust impl block is a container, not a symbol** (P1.3c; a default
//!   pending human review). rust-analyzer gives some impl blocks a symbol of
//!   their own, ``impl#[Tree]`` (V97), which would otherwise read `Tree`,
//!   the type's name. [`canonical_name`] returns `None` for one
//!   ([`is_rust_impl_block`]); its members keep `Type.method` and
//!   `Type.Trait.method`.
//! - **A value named like a type, or a field named like a method, gets its
//!   kind's suffix** (`+fn`, `+field`, `+const`, `+static`), and only then;
//!   see [`CanonicalNames`] (P1.3c; a default pending human review).
//! - **Other type parameters are canonical.** `[T]` becomes a segment `T`
//!   (scip-typescript's `Charge#[T]` is `Charge.T`).
//! - **Parameters and locals are not canonical symbols.** `local N`
//!   symbols are the discarded local scope (brief §4.4), and a symbol with
//!   any `(param)` descriptor is a function parameter, scoped like a local.
//!   [`canonical_name`] returns `None` for both, and for an impl block.
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

/// Returns `true` for a rust-analyzer impl block itself (``a/impl#[Tree]``,
/// ``a/impl#[`Charge<T>`][Apply]``): the `impl` type descriptor followed by
/// nothing but type parameters. An impl block is a container, not a symbol,
/// so it has no canonical name; see the module documentation.
pub fn is_rust_impl_block(symbol: &GlobalSymbol) -> bool {
    symbol
        .descriptors
        .iter()
        .enumerate()
        .rev()
        .find(|(_, d)| d.suffix != Suffix::TypeParameter)
        .is_some_and(|(index, d)| {
            index + 1 < symbol.descriptors.len() && is_rust_impl(symbol, index, d)
        })
}

/// Returns `true` for a field: a term directly under a type, such as
/// `Server#export_path.` (rust-analyzer, scip-go, scip-typescript all spell
/// fields and properties this way). A term in a Rust impl block, an
/// associated constant, is not one: its parent is a type parameter.
pub fn is_field(symbol: &GlobalSymbol) -> bool {
    match symbol.descriptors.as_slice() {
        [.., parent, last] => last.suffix == Suffix::Term && parent.suffix == Suffix::Type,
        _ => false,
    }
}

/// Returns `true` for rust-analyzer's crate root module descriptor,
/// `crate/`, which the crate segment replaces.
fn is_rust_crate_root(d: &Descriptor) -> bool {
    d.suffix == Suffix::Namespace && d.name == "crate"
}

fn global_segments(symbol: &GlobalSymbol) -> Option<Vec<Segment>> {
    if is_rust_impl_block(symbol) {
        return None;
    }
    let mut out = Vec::with_capacity(symbol.descriptors.len() + 1);
    let rust = symbol.scheme == RUST_ANALYZER;
    if rust && !symbol.package.name.is_empty() {
        out.push(Segment {
            name: symbol.package.name.replace('-', "_"),
            disambiguator: None,
        });
    }
    for (index, d) in symbol.descriptors.iter().enumerate() {
        if d.suffix == Suffix::Parameter {
            return None;
        }
        if is_rust_impl(symbol, index, d) || (rust && index == 0 && is_rust_crate_root(d)) {
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

/// Rust's two namespaces, plus the kinds of symbol neither policy touches.
///
/// Rust lets a type-namespace item (a module, struct, enum, trait or type
/// alias) and a value-namespace item (a function, const, static, or a
/// field as a member) share a name, so their canonical names can collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    /// A module, type, trait or type parameter: keeps the plain name.
    Type,
    /// A value, which gets its kind's suffix on a collision.
    Value(ValueKind),
    /// A macro or a meta descriptor: never suffixed.
    Other,
}

/// What kind of value a value-namespace symbol is. The order is the
/// precedence for the plain name when no type collides (see
/// [`CanonicalNames`]): a function keeps it over a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueKind {
    /// A function or method: `+fn`.
    Fn,
    /// A constant: `+const`.
    Const,
    /// A static: `+static`.
    Static,
    /// A field, a member of a type: `+field`.
    Field,
}

impl ValueKind {
    /// The suffix this kind adds to a colliding name, e.g. `+field` in
    /// `spike.Server.export_path+field`.
    ///
    /// `+` and then letters cannot be mistaken for an overload's `+N`, which
    /// is digits (a non-numeric SCIP disambiguator such as `(x)` also renders
    /// as `+x`; no indexer we know of writes one, and a clash would be
    /// reported as a collision). The result stays a tagma bare token and a
    /// legal file name (V96).
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Fn => "+fn",
            Self::Const => "+const",
            Self::Static => "+static",
            Self::Field => "+field",
        }
    }
}

/// The namespace of `symbol` as its descriptors tell it: a namespace, type
/// or type parameter is a type; a method a function; a term directly under
/// a type a field ([`is_field`]); any other term a const, since descriptors
/// cannot tell a const from a static (a provider's kind can, through
/// [`CanonicalNames::insert_as`]); a macro or meta descriptor neither.
pub fn namespace(symbol: &GlobalSymbol) -> Namespace {
    match symbol.descriptors.last().map(|d| d.suffix) {
        Some(Suffix::Namespace | Suffix::Type | Suffix::TypeParameter) => Namespace::Type,
        Some(Suffix::Method) => Namespace::Value(ValueKind::Fn),
        Some(Suffix::Term) if is_field(symbol) => Namespace::Value(ValueKind::Field),
        Some(Suffix::Term) => Namespace::Value(ValueKind::Const),
        Some(Suffix::Meta | Suffix::Macro | Suffix::Parameter) | None => Namespace::Other,
    }
}

/// Canonical names assigned to a set of symbols, with every collision kept
/// and reported rather than merged. The result does not depend on the order
/// symbols are inserted in.
///
/// **Namespace policy** (P1.3c; a default pending human review). Rust keeps
/// types and values in separate namespaces, so a module and a function can
/// share a name (`checker/` and `checker().`), and a getter is often named
/// like the field it returns (`spike/Server#export_path.` and
/// `spike/impl#[Server]export_path().`). When symbols of more than one
/// [`Namespace`] class (a type, or one [`ValueKind`]) share a canonical
/// name, one class keeps the plain name and every value of another class
/// gets its kind's [suffix](ValueKind::suffix):
///
/// - a type keeps it, so `checker/` is `….checker` and `checker().` is
///   `….checker+fn`;
/// - with no type there, the first [`ValueKind`] present keeps it, so a
///   getter stays `….export_path` and its field is `….export_path+field`.
///
/// Only colliding names change. Macros and metas are never suffixed and do
/// not count as a class. Symbols of one class that share a name (two
/// functions, two fields) are still a collision: whatever still collides
/// afterwards is reported by [`CanonicalNames::collisions`], never merged.
#[derive(Debug, Clone, Default)]
pub struct CanonicalNames {
    /// Base canonical name to the distinct SCIP symbols that have it, each
    /// with its namespace.
    by_name: BTreeMap<String, BTreeMap<String, Namespace>>,
}

impl CanonicalNames {
    /// Records `symbol` with the [`namespace`] its descriptors give, and
    /// returns whether it has a canonical name: a symbol with none (a local,
    /// a parameter, an impl block) is not recorded. Inserting the same
    /// symbol twice is not a collision. The final names, after the
    /// namespace policy, come from [`assigned`](Self::assigned).
    ///
    /// # Errors
    ///
    /// As [`canonical_name`].
    pub fn insert(&mut self, symbol: &Symbol) -> Result<bool, CanonicalError> {
        let space = match symbol {
            Symbol::Global(global) => namespace(global),
            Symbol::Local(_) => Namespace::Other,
        };
        self.insert_as(symbol, space)
    }

    /// As [`insert`](Self::insert), with the namespace given, e.g. from the
    /// provider's symbol kind.
    ///
    /// # Errors
    ///
    /// As [`canonical_name`].
    pub fn insert_as(&mut self, symbol: &Symbol, space: Namespace) -> Result<bool, CanonicalError> {
        let Some(name) = canonical_name(symbol)? else {
            return Ok(false);
        };
        self.by_name
            .entry(name)
            .or_default()
            .insert(symbol.to_string(), space);
        Ok(true)
    }

    /// Final name to the symbols that have it, after the namespace policy.
    fn resolved(&self) -> BTreeMap<String, BTreeSet<&str>> {
        let mut out: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for (name, symbols) in &self.by_name {
            let classes: BTreeSet<Namespace> = symbols
                .values()
                .copied()
                .filter(|space| *space != Namespace::Other)
                .collect();
            // `Type` sorts first, then the value kinds in precedence order.
            let plain = if classes.len() > 1 {
                classes.first().copied()
            } else {
                None
            };
            for (symbol, &space) in symbols {
                let name = match space {
                    Namespace::Value(kind) if plain.is_some_and(|p| p != space) => {
                        format!("{name}{}", kind.suffix())
                    }
                    _ => name.clone(),
                };
                out.entry(name).or_default().insert(symbol);
            }
        }
        out
    }

    /// Every recorded symbol's final name, by SCIP symbol. A colliding
    /// symbol is included with the name it shares.
    pub fn assigned(&self) -> BTreeMap<String, String> {
        self.resolved()
            .into_iter()
            .flat_map(|(name, symbols)| {
                symbols
                    .into_iter()
                    .map(move |symbol| (symbol.to_string(), name.clone()))
            })
            .collect()
    }

    /// Every final name shared by more than one symbol, sorted by name.
    pub fn collisions(&self) -> Vec<Collision> {
        self.resolved()
            .into_iter()
            .filter(|(_, symbols)| symbols.len() > 1)
            .map(|(name, symbols)| Collision {
                name,
                symbols: symbols.into_iter().map(str::to_string).collect(),
            })
            .collect()
    }

    /// The final names that belong to exactly one symbol, as
    /// `(name, symbol)`, sorted by name. Colliding names are left out.
    pub fn unique(&self) -> Vec<(String, String)> {
        self.resolved()
            .into_iter()
            .filter_map(|(name, symbols)| {
                let mut iter = symbols.into_iter();
                match (iter.next(), iter.next()) {
                    (Some(symbol), None) => Some((name, symbol.to_string())),
                    _ => None,
                }
            })
            .collect()
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
            some("demo.billing.Charge<T>.Apply.apply")
        );
        assert_eq!(
            name(&format!("{p}billing/impl#[`Charge<T>`]new().")),
            some("demo.billing.Charge<T>.new")
        );
        assert_eq!(
            name(&format!("{p}billing/impl#[`inner::Fee`][Apply]apply().")),
            some("demo.billing.inner.Fee.Apply.apply")
        );
        // Only rust-analyzer's impl blocks are rewritten.
        assert_eq!(name("other m n v impl#[T]f()."), some("impl.T.f"));
    }

    #[test]
    fn rust_names_start_with_the_crate() {
        let p = "rust-analyzer cargo codetags-model 0.1.0 ";
        assert_eq!(
            name(&format!("{p}store/GenerationStore#")),
            some("codetags_model.store.GenerationStore")
        );
        assert_eq!(name(&format!("{p}crate/")), some("codetags_model"));
        assert_eq!(name(&format!("{p}main().")), some("codetags_model.main"));
        // Only a leading `crate/` is the root.
        assert_eq!(
            name(&format!("{p}a/crate/")),
            some("codetags_model.a.crate")
        );
        // No package name, no prefix; other schemes keep none.
        assert_eq!(name("rust-analyzer cargo . . a/f()."), some("a.f"));
        assert_eq!(name("scip-go gomod demo v1 `x/y`/F()."), some("x.y.F"));
    }

    #[test]
    fn rust_impl_blocks_themselves_have_no_name() {
        let p = "rust-analyzer cargo demo 0.1.0 ";
        assert_eq!(name(&format!("{p}spike/impl#[Tree]")), None);
        assert_eq!(name(&format!("{p}charge/impl#[`Charge<T>`][Apply]")), None);
        assert_eq!(name(&format!("{p}impl#[Tree]")), None);
        // Members of an impl block keep theirs.
        assert_eq!(
            name(&format!("{p}spike/impl#[Tree]Output#")),
            some("demo.spike.Tree.Output")
        );
        // Only rust-analyzer's.
        assert_eq!(name("other m n v impl#[T]"), some("impl.T"));
    }

    fn assign(symbols: &[&str]) -> CanonicalNames {
        let mut names = CanonicalNames::default();
        for s in symbols {
            names
                .insert(&parse(s).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        names
    }

    #[test]
    fn a_field_colliding_with_a_method_is_suffixed() {
        let p = "rust-analyzer cargo demo 0.1.0 ";
        let field = format!("{p}spike/Server#export_path.");
        let getter = format!("{p}spike/impl#[Server]export_path().");
        let plain = format!("{p}spike/Server#root.");
        let names = assign(&[&field, &getter, &plain]);
        let assigned = names.assigned();
        assert_eq!(assigned[&field], "demo.spike.Server.export_path+field");
        assert_eq!(assigned[&getter], "demo.spike.Server.export_path");
        assert_eq!(assigned[&plain], "demo.spike.Server.root");
        assert_eq!(names.collisions(), vec![]);
        assert_eq!(
            names
                .unique()
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>(),
            [
                "demo.spike.Server.export_path",
                "demo.spike.Server.export_path+field",
                "demo.spike.Server.root"
            ]
        );
    }

    #[test]
    fn a_value_colliding_with_a_type_gets_its_kind() {
        let p = "rust-analyzer cargo demo 0.1.0 ";
        let module = format!("{p}checker/");
        let function = format!("{p}checker().");
        let constant = format!("{p}checker.");
        let names = assign(&[&module, &function, &constant]);
        let assigned = names.assigned();
        assert_eq!(assigned[&module], "demo.checker");
        assert_eq!(assigned[&function], "demo.checker+fn");
        assert_eq!(assigned[&constant], "demo.checker+const");
        assert_eq!(names.collisions(), vec![]);
    }

    #[test]
    fn a_provider_kind_overrides_the_descriptors() {
        let p = "rust-analyzer cargo demo 0.1.0 ";
        let module = parse(&format!("{p}limit/")).unwrap_or_else(|e| panic!("{e}"));
        let fixed = parse(&format!("{p}limit.")).unwrap_or_else(|e| panic!("{e}"));
        let mut names = CanonicalNames::default();
        assert_eq!(names.insert(&module), Ok(true));
        assert_eq!(
            names.insert_as(&fixed, Namespace::Value(ValueKind::Static)),
            Ok(true)
        );
        assert_eq!(names.assigned()[&fixed.to_string()], "demo.limit+static");
    }

    #[test]
    fn macros_are_never_suffixed_and_still_collide() {
        let names = assign(&["s m n v charge!", "s m n v charge()."]);
        assert_eq!(names.collisions()[0].name, "charge");
    }

    #[test]
    fn namespaces_follow_the_descriptors() {
        let ns = |s: &str| match parse(s).unwrap_or_else(|e| panic!("{e}")) {
            Symbol::Global(g) => namespace(&g),
            Symbol::Local(_) => panic!("local"),
        };
        assert_eq!(ns("s m n v a/"), Namespace::Type);
        assert_eq!(ns("s m n v a/B#"), Namespace::Type);
        assert_eq!(ns("s m n v a/f()."), Namespace::Value(ValueKind::Fn));
        assert_eq!(ns("s m n v a/B#f."), Namespace::Value(ValueKind::Field));
        assert_eq!(ns("s m n v a/F."), Namespace::Value(ValueKind::Const));
        assert_eq!(ns("s m n v a/f!"), Namespace::Other);
    }

    #[test]
    fn fields_alone_are_not_suffixed() {
        // Two packages' fields collide with each other only: no method, so
        // no suffix, and the collision is reported.
        let names = assign(&["s m a v x/Y#f.", "s m b v x/Y#f."]);
        assert_eq!(names.collisions()[0].name, "x.Y.f");
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
            names.unique(),
            vec![("x.Z".to_string(), "s m a v x/Z#".to_string())]
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
