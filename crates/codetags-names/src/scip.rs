//! SCIP symbol strings: parsing and formatting.
//!
//! The grammar is the one documented on `Symbol` in `scip.proto`
//! (github.com/scip-code/scip, checked in `docs/verification.md` V33):
//!
//! ```text
//! <symbol>      ::= <scheme> ' ' <package> ' ' (<descriptor>)+ | 'local ' <local-id>
//! <package>     ::= <manager> ' ' <package-name> ' ' <version>
//! <scheme>      ::= any UTF-8, spaces escaped as "  "; not empty, not starting with "local"
//! <manager>, <package-name>, <version>
//!               ::= same as above; "." is the placeholder for an empty value
//! <descriptor>  ::= <name> '/'        namespace
//!                 | <name> '#'        type
//!                 | <name> '.'        term
//!                 | <name> ':'        meta
//!                 | <name> '!'        macro
//!                 | <name> '(' (<method-disambiguator>)? ').'   method
//!                 | '[' <name> ']'    type parameter
//!                 | '(' <name> ')'    parameter
//! <name>        ::= <simple-identifier> | <escaped-identifier>
//! <simple-identifier>  ::= ( '_' | '+' | '-' | '$' | ASCII letter or digit )+
//! <escaped-identifier> ::= '`' (any UTF-8, "``" for a backtick)+ '`'
//! <local-id>    ::= <simple-identifier>
//! ```
//!
//! Two deliberate leniencies, both lossless: an escaped identifier is
//! accepted even when every character in it is an identifier character
//! (the grammar says it "must contain at least one non-identifier
//! character", but indexers do not always honour that), and a method
//! disambiguator may be escaped. [`Symbol`]'s `Display` writes the
//! canonical spelling back: names are escaped only when they must be.
//!
//! One ambiguity is in the grammar itself: a header field that *starts*
//! with a space cannot be told apart from the previous field ending in one,
//! because the separator and the doubled space run together. The parser
//! reads a run of spaces greedily, pairing them left to right, so such a
//! space lands at the end of the earlier field. No indexer we know of
//! writes one.

use std::fmt;

/// A parsed SCIP symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Symbol {
    /// `local <id>`: a symbol visible only inside one document.
    Local(String),
    /// A symbol with a package and at least one descriptor.
    Global(GlobalSymbol),
}

/// A non-local symbol: `<scheme> <manager> <name> <version> <descriptors>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlobalSymbol {
    /// The indexer's scheme, e.g. `rust-analyzer` or `scip-go`.
    pub scheme: String,
    /// The package the symbol belongs to.
    pub package: Package,
    /// The descriptors, outermost first. Never empty.
    pub descriptors: Vec<Descriptor>,
}

/// A symbol's package. The `.` placeholder parses to an empty string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Package {
    /// Package manager, e.g. `cargo`, `gomod`, `npm`, `python`.
    pub manager: String,
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
}

/// One descriptor of a global symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Descriptor {
    /// The unescaped name.
    pub name: String,
    /// What kind of descriptor this is.
    pub suffix: Suffix,
    /// A method's disambiguator, e.g. `+1` from `foo(+1).`. `None` for
    /// `foo().` and for every other suffix.
    pub disambiguator: Option<String>,
}

/// The descriptor kinds of `scip.proto`'s `Descriptor.Suffix`, minus
/// `UnspecifiedSuffix` and `Local` (which never appear inside a symbol
/// string's descriptor list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Suffix {
    /// `name/`: a package, module or namespace.
    Namespace,
    /// `name#`: a type.
    Type,
    /// `name.`: a term (field, constant, variable, function in some indexers).
    Term,
    /// `name(disambiguator).`: a method or function.
    Method,
    /// `[name]`: a type parameter.
    TypeParameter,
    /// `(name)`: a parameter.
    Parameter,
    /// `name:`: a meta descriptor.
    Meta,
    /// `name!`: a macro.
    Macro,
}

/// Why a SCIP symbol string did not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset in the input where the problem was found.
    pub offset: usize,
    /// What was expected or wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SCIP symbol: at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Returns `true` for SCIP's `<identifier-character>`.
pub fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '$')
}

/// Parses a SCIP symbol string.
///
/// # Errors
///
/// Returns a [`ParseError`] if `s` does not follow the grammar in the
/// module documentation.
pub fn parse(s: &str) -> Result<Symbol, ParseError> {
    Parser { s, pos: 0 }.symbol()
}

impl std::str::FromStr for Symbol {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse(s)
    }
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            offset: self.pos,
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn expect(&mut self, want: char) -> Result<(), ParseError> {
        match self.peek() {
            Some(c) if c == want => {
                self.pos += c.len_utf8();
                Ok(())
            }
            Some(c) => self.error(format!("expected {want:?}, found {c:?}")),
            None => self.error(format!("expected {want:?}, found the end")),
        }
    }

    fn symbol(mut self) -> Result<Symbol, ParseError> {
        if let Some(rest) = self.s.strip_prefix("local ") {
            self.pos = "local ".len();
            if rest.is_empty() || !rest.chars().all(is_identifier_char) {
                return self.error("a local id is one or more identifier characters");
            }
            return Ok(Symbol::Local(rest.to_string()));
        }
        let scheme = self.field()?;
        if scheme.is_empty() {
            return self.error("the scheme is empty");
        }
        if scheme.starts_with("local") {
            return self.error("a scheme must not start with \"local\"");
        }
        let manager = placeholder(self.field()?);
        let name = placeholder(self.field()?);
        let version = placeholder(self.field()?);
        let mut descriptors = Vec::new();
        while self.peek().is_some() {
            descriptors.push(self.descriptor()?);
        }
        if descriptors.is_empty() {
            return self.error("a global symbol has at least one descriptor");
        }
        Ok(Symbol::Global(GlobalSymbol {
            scheme,
            package: Package {
                manager,
                name,
                version,
            },
            descriptors,
        }))
    }

    /// One space-terminated header field; `"  "` is a literal space.
    fn field(&mut self) -> Result<String, ParseError> {
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.error("the symbol ended inside its header"),
                Some(' ') if self.peek() == Some(' ') => {
                    self.pos += 1;
                    out.push(' ');
                }
                Some(' ') => return Ok(out),
                Some(c) => out.push(c),
            }
        }
    }

    fn descriptor(&mut self) -> Result<Descriptor, ParseError> {
        let plain = |name, suffix| Descriptor {
            name,
            suffix,
            disambiguator: None,
        };
        match self.peek() {
            Some('[') => {
                self.pos += 1;
                let name = self.name()?;
                self.expect(']')?;
                return Ok(plain(name, Suffix::TypeParameter));
            }
            Some('(') => {
                self.pos += 1;
                let name = self.name()?;
                self.expect(')')?;
                return Ok(plain(name, Suffix::Parameter));
            }
            _ => {}
        }
        let name = self.name()?;
        let suffix = match self.bump() {
            Some('/') => Suffix::Namespace,
            Some('#') => Suffix::Type,
            Some('.') => Suffix::Term,
            Some(':') => Suffix::Meta,
            Some('!') => Suffix::Macro,
            Some('(') => {
                let disambiguator = if self.peek() == Some(')') {
                    None
                } else {
                    Some(self.name()?)
                };
                self.expect(')')?;
                self.expect('.')?;
                return Ok(Descriptor {
                    name,
                    suffix: Suffix::Method,
                    disambiguator,
                });
            }
            Some(c) => return self.error(format!("{c:?} is not a descriptor suffix")),
            None => return self.error("a descriptor name has no suffix"),
        };
        Ok(plain(name, suffix))
    }

    fn name(&mut self) -> Result<String, ParseError> {
        if self.peek() == Some('`') {
            self.pos += 1;
            let mut out = String::new();
            loop {
                match self.bump() {
                    None => return self.error("unterminated escaped identifier"),
                    Some('`') if self.peek() == Some('`') => {
                        self.pos += 1;
                        out.push('`');
                    }
                    Some('`') => break,
                    Some(c) => out.push(c),
                }
            }
            if out.is_empty() {
                return self.error("an escaped identifier is empty");
            }
            return Ok(out);
        }
        let start = self.pos;
        while self.peek().is_some_and(is_identifier_char) {
            self.pos += 1;
        }
        if self.pos == start {
            return self.error("expected an identifier");
        }
        Ok(self.s[start..self.pos].to_string())
    }
}

fn placeholder(field: String) -> String {
    if field == "." { String::new() } else { field }
}

/// Writes a header field: empty as `.`, spaces doubled.
fn write_field(f: &mut fmt::Formatter<'_>, field: &str) -> fmt::Result {
    if field.is_empty() {
        f.write_str(".")
    } else {
        f.write_str(&field.replace(' ', "  "))
    }
}

/// Writes a name, escaping it with backticks only when it must be.
fn write_name(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    if !name.is_empty() && name.chars().all(is_identifier_char) {
        f.write_str(name)
    } else {
        write!(f, "`{}`", name.replace('`', "``"))
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::Local(id) => write!(f, "local {id}"),
            Symbol::Global(g) => g.fmt(f),
        }
    }
}

impl fmt::Display for GlobalSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_field(f, &self.scheme)?;
        for field in [
            &self.package.manager,
            &self.package.name,
            &self.package.version,
        ] {
            f.write_str(" ")?;
            write_field(f, field)?;
        }
        f.write_str(" ")?;
        for d in &self.descriptors {
            d.fmt(f)?;
        }
        Ok(())
    }
}

impl fmt::Display for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.suffix {
            Suffix::TypeParameter => {
                f.write_str("[")?;
                write_name(f, &self.name)?;
                return f.write_str("]");
            }
            Suffix::Parameter => {
                f.write_str("(")?;
                write_name(f, &self.name)?;
                return f.write_str(")");
            }
            _ => {}
        }
        write_name(f, &self.name)?;
        match self.suffix {
            Suffix::Namespace => f.write_str("/"),
            Suffix::Type => f.write_str("#"),
            Suffix::Term => f.write_str("."),
            Suffix::Meta => f.write_str(":"),
            Suffix::Macro => f.write_str("!"),
            Suffix::Method => {
                f.write_str("(")?;
                if let Some(d) = &self.disambiguator {
                    write_name(f, d)?;
                }
                f.write_str(").")
            }
            Suffix::TypeParameter | Suffix::Parameter => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn d(name: &str, suffix: Suffix) -> Descriptor {
        Descriptor {
            name: name.into(),
            suffix,
            disambiguator: None,
        }
    }

    fn global(s: &str) -> GlobalSymbol {
        match parse(s) {
            Ok(Symbol::Global(g)) => g,
            other => panic!("{s:?} parsed to {other:?}"),
        }
    }

    #[test]
    fn locals() {
        assert_eq!(parse("local 12"), Ok(Symbol::Local("12".into())));
        assert!(parse("local ").is_err());
        assert!(parse("local a b").is_err());
    }

    #[test]
    fn rust_analyzer_impl_method() {
        // Emitted by rust-analyzer 1.96.1 for `impl<T> Apply for Charge<T> { fn apply }`.
        let g = global("rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`][Apply]apply().");
        assert_eq!(g.scheme, "rust-analyzer");
        assert_eq!(
            g.package,
            Package {
                manager: "cargo".into(),
                name: "demo".into(),
                version: "0.1.0".into()
            }
        );
        assert_eq!(
            g.descriptors,
            vec![
                d("billing", Suffix::Namespace),
                d("impl", Suffix::Type),
                d("Charge<T>", Suffix::TypeParameter),
                d("Apply", Suffix::TypeParameter),
                d("apply", Suffix::Method),
            ]
        );
    }

    #[test]
    fn every_suffix() {
        let g = global("s m n v a/b#c.d:e!f(+1).[g](h)");
        let kinds: Vec<_> = g.descriptors.iter().map(|d| d.suffix).collect();
        assert_eq!(
            kinds,
            vec![
                Suffix::Namespace,
                Suffix::Type,
                Suffix::Term,
                Suffix::Meta,
                Suffix::Macro,
                Suffix::Method,
                Suffix::TypeParameter,
                Suffix::Parameter,
            ]
        );
        assert_eq!(g.descriptors[5].disambiguator.as_deref(), Some("+1"));
    }

    #[test]
    fn header_escapes_and_placeholders() {
        let g = global("my  scheme . my  pkg . x.");
        assert_eq!(g.scheme, "my scheme");
        assert_eq!(g.package.manager, "");
        assert_eq!(g.package.name, "my pkg");
        assert_eq!(g.package.version, "");
    }

    #[test]
    fn backtick_escapes() {
        let g = global("s m n v `a``b c`#");
        assert_eq!(g.descriptors, vec![d("a`b c", Suffix::Type)]);
    }

    #[test]
    fn malformed() {
        for bad in [
            "",
            "rust-analyzer cargo demo 0.1.0 ",
            "rust-analyzer cargo demo",
            "localx m n v a.",
            " m n v a.",
            "s m n v a",
            "s m n v a?",
            "s m n v `a",
            "s m n v ``.",
            "s m n v a(.",
            "s m n v a().x",
            "s m n v [a",
            "s m n v (a",
            "s m n v a b.",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn display_is_canonical() {
        for s in [
            "local 7",
            "rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`][Apply]apply().",
            "scip-go gomod github.com/a/b v1.2.3 `github.com/a/b/pkg`/Server#Serve().",
            "s . . . a/b#c.d:e!f(+1).[g](h)",
            "my  scheme m n v `a``b c`#",
        ] {
            assert_eq!(global_or_local(s).to_string(), s);
        }
    }

    fn global_or_local(s: &str) -> Symbol {
        parse(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
    }

    fn name() -> impl Strategy<Value = String> {
        prop_oneof![
            "[A-Za-z0-9_+$-]{1,8}",
            "\\PC{1,8}",
            "[ `.#/():!\\[\\]]{1,4}"
        ]
    }

    fn descriptor() -> impl Strategy<Value = Descriptor> {
        let suffix = prop_oneof![
            Just(Suffix::Namespace),
            Just(Suffix::Type),
            Just(Suffix::Term),
            Just(Suffix::Method),
            Just(Suffix::TypeParameter),
            Just(Suffix::Parameter),
            Just(Suffix::Meta),
            Just(Suffix::Macro),
        ];
        (name(), suffix, proptest::option::of(name())).prop_map(|(name, suffix, dis)| Descriptor {
            name,
            suffix,
            disambiguator: if suffix == Suffix::Method { dis } else { None },
        })
    }

    fn header() -> impl Strategy<Value = String> {
        // Any text but a lone "." (the empty placeholder) or a leading
        // space (ambiguous in the grammar; see the module documentation).
        "([a-z.@/-][a-z .@/-]{0,5})?".prop_filter("not the placeholder", |s| s != ".")
    }

    proptest! {
        #[test]
        fn display_then_parse_round_trips(
            scheme in "[a-km-z][a-z .-]{0,6}",
            manager in header(),
            pkg in header(),
            version in header(),
            descriptors in proptest::collection::vec(descriptor(), 1..6),
        ) {
            let symbol = Symbol::Global(GlobalSymbol {
                scheme,
                package: Package { manager, name: pkg, version },
                descriptors,
            });
            prop_assert_eq!(parse(&symbol.to_string()), Ok(symbol));
        }
    }
}
