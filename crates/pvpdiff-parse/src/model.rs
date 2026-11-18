//! Normalized model shared by the Hoogle parser, Cabal adapter, and diff.

use std::collections::BTreeMap;

/// One kind of declaration that can appear in a Hoogle database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKind {
    /// A function or value binding.
    Function,
    /// A `data` declaration head.
    Data,
    /// A `newtype` declaration head.
    Newtype,
    /// A type synonym.
    TypeSynonym,
    /// A type family declaration.
    TypeFamily,
    /// A data family declaration.
    DataFamily,
    /// A class declaration.
    Class,
    /// A pattern synonym.
    Pattern,
    /// A constructor owned by a data or newtype declaration.
    Constructor,
    /// A record field selector owned by a data or newtype declaration.
    RecordField,
}

impl EntityKind {
    /// Stable lowercase name used in reports and test expectations.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Data => "data",
            Self::Newtype => "newtype",
            Self::TypeSynonym => "type-synonym",
            Self::TypeFamily => "type-family",
            Self::DataFamily => "data-family",
            Self::Class => "class",
            Self::Pattern => "pattern",
            Self::Constructor => "constructor",
            Self::RecordField => "record-field",
        }
    }

    /// Whether this kind is a top-level declaration rather than a child of a
    /// data type.
    pub fn is_top_level(self) -> bool {
        !matches!(self, Self::Constructor | Self::RecordField)
    }
}

/// Identity of an entity inside one module.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityKey {
    /// Declaration kind.
    pub kind: EntityKind,
    /// Declared name. Operators are stored without surrounding parentheses or
    /// backticks.
    pub name: String,
    /// Owning data type for constructors and record fields.
    pub parent: Option<String>,
}

impl EntityKey {
    /// Build a top-level key.
    pub fn top(kind: EntityKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            parent: None,
        }
    }

    /// Build a key owned by a data type.
    pub fn child(kind: EntityKind, parent: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            parent: Some(parent.into()),
        }
    }
}

/// A declaration as rendered by Haddock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    /// Declared name for display.
    pub name: String,
    /// Type after the `::` separator for functions, constructors, fields, and
    /// pattern synonyms.
    pub signature: Option<String>,
    /// Declaration text after the keyword (and the class `where { ... }`
    /// block, when present).
    pub head: Option<String>,
    /// Whether a `Deprecated:` doc block precedes the declaration.
    pub deprecated: bool,
}

/// Associativity of an infix declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Associativity {
    /// `infixl`
    Left,
    /// `infixr`
    Right,
    /// `infix`
    None,
}

/// A fixity declaration for one operator or backticked function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixity {
    /// Associativity.
    pub associativity: Associativity,
    /// Precedence from 0 to 9.
    pub precedence: u8,
    /// Operator name without backticks.
    pub operator: String,
}

/// One `instance` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// Class name, including any module qualification Haddock printed.
    pub class: String,
    /// Instance head after the context.
    pub head: String,
    /// Top-level constraints before `=>`, split on commas.
    pub context: Vec<String>,
    /// Original line for diagnostics.
    pub raw: String,
}

/// One module section from a Hoogle database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleApi {
    /// Top-level entities and the children owned by data declarations.
    pub entities: BTreeMap<EntityKey, Entity>,
    /// Instances declared in this module.
    pub instances: Vec<Instance>,
    /// Fixity declarations in this module.
    pub fixities: BTreeMap<String, Fixity>,
    /// Whether the module itself is deprecated.
    pub deprecated: bool,
    /// Number of top-level declarations in the section.
    pub named_entries: usize,
    /// Insertion order of entity keys, used for constructor-order checks. The
    /// entity map is sorted by key and would lose declaration order.
    pub entity_order: Vec<EntityKey>,
}

impl ModuleApi {
    /// A module with a doc block but no named entries is usually a pure
    /// re-export module. Hoogle drops `module X` export items, so its public
    /// surface is not observable.
    pub fn is_reexport_only(&self) -> bool {
        self.named_entries == 0
    }
}

/// The API surface of one package version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiSurface {
    /// Package name from `@package`.
    pub package: String,
    /// Version from `@version`.
    pub version: String,
    /// Exposed modules keyed by module name.
    pub modules: BTreeMap<String, ModuleApi>,
    /// Non-fatal parser notes, such as an unrecognized line.
    pub warnings: Vec<String>,
}

impl ApiSurface {
    /// Iterate over the package module names, which the orphan-instance
    /// heuristic uses to decide whether a class or type belongs to this
    /// package.
    pub fn module_names(&self) -> impl Iterator<Item = &String> {
        self.modules.keys()
    }

    /// Count top-level entities across the package.
    pub fn top_level_entity_count(&self) -> usize {
        self.modules
            .values()
            .map(|module| module.named_entries)
            .sum()
    }
}

/// Component that declared a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentKind {
    /// A library component.
    Library,
    /// An executable component.
    Executable,
    /// A test suite.
    TestSuite,
    /// A benchmark.
    Benchmark,
}

impl ComponentKind {
    /// Stable lowercase name used in the index.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Executable => "executable",
            Self::TestSuite => "test-suite",
            Self::Benchmark => "benchmark",
        }
    }
}

/// One dependency after normalization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DependencyMeta {
    /// Dependency package name.
    pub package: String,
    /// Canonical Cabal version range, or `-any`.
    pub range: String,
    /// Component that declared the dependency.
    pub component: ComponentKind,
}

/// One library component.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryMeta {
    /// Library name; `None` is the default library.
    pub name: Option<String>,
    /// Modules listed in `exposed-modules`.
    pub exposed_modules: Vec<String>,
    /// Modules in `reexported-modules`, reduced to the source module name.
    pub reexported_modules: Vec<String>,
    /// Library dependencies.
    pub dependencies: Vec<DependencyMeta>,
    /// Whether a Cabal conditional guards the module surface.
    pub conditional_surface: bool,
}

/// Parsed Cabal metadata that the diff needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CabalMeta {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Latest `x-revision`, when present.
    pub revision: Option<u32>,
    /// All public and internal libraries.
    pub libraries: Vec<LibraryMeta>,
    /// Whether any conditional appears in the file.
    pub conditional: bool,
}

/// Result of parsing a Cabal file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedCabal {
    /// Metadata when the file had a usable name and version.
    pub meta: Option<CabalMeta>,
    /// Non-fatal parser and adapter diagnostics.
    pub diagnostics: Vec<String>,
}
