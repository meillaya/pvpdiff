//! Stable rule catalog.
//!
//! Rule identifiers are part of the JSON and SARIF contract. Summaries are
//! static so a report can explain a change without embedding policy text in the
//! diff code.

/// One rule in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// Stable identifier used in JSON and SARIF.
    pub id: &'static str,
    /// Short explanation of the detection.
    pub summary: &'static str,
}

/// Every rule pvpdiff can emit.
pub const RULES: &[Rule] = &[
    Rule {
        id: "pvp/module-removed",
        summary: "an exposed module is absent from the new version",
    },
    Rule {
        id: "pvp/module-added",
        summary: "a new exposed module appears in the new version",
    },
    Rule {
        id: "pvp/reexport-changed",
        summary: "the Cabal reexported-modules list changed",
    },
    Rule {
        id: "pvp/entity-removed",
        summary: "a top-level exported declaration is absent",
    },
    Rule {
        id: "pvp/entity-added",
        summary: "a new top-level exported declaration appears",
    },
    Rule {
        id: "pvp/entity-kind-changed",
        summary: "a declaration changed kind, such as data to newtype",
    },
    Rule {
        id: "pvp/signature-changed",
        summary: "a normalized type signature changed",
    },
    Rule {
        id: "pvp/signature-unverifiable",
        summary: "a signature differs in a way the data cannot decide",
    },
    Rule {
        id: "pvp/data-declaration-changed",
        summary: "a data or newtype head changed",
    },
    Rule {
        id: "pvp/constructor-added",
        summary: "a data constructor was added",
    },
    Rule {
        id: "pvp/constructor-removed",
        summary: "a data constructor was removed",
    },
    Rule {
        id: "pvp/constructor-changed",
        summary: "a data constructor signature changed",
    },
    Rule {
        id: "pvp/constructor-order-changed",
        summary: "constructors kept their names but changed order",
    },
    Rule {
        id: "pvp/field-added",
        summary: "a record selector was added",
    },
    Rule {
        id: "pvp/field-removed",
        summary: "a record selector was removed",
    },
    Rule {
        id: "pvp/field-changed",
        summary: "a record selector type changed",
    },
    Rule {
        id: "pvp/type-synonym-changed",
        summary: "a type synonym definition changed",
    },
    Rule {
        id: "pvp/type-family-changed",
        summary: "a type or data family declaration changed",
    },
    Rule {
        id: "pvp/class-method-added",
        summary: "a signature looks like a new class method",
    },
    Rule {
        id: "pvp/class-method-removed",
        summary: "a class method signature was removed",
    },
    Rule {
        id: "pvp/superclass-changed",
        summary: "a class context changed",
    },
    Rule {
        id: "pvp/class-declaration-changed",
        summary: "a class head or associated type changed",
    },
    Rule {
        id: "pvp/pattern-added",
        summary: "a pattern synonym was added",
    },
    Rule {
        id: "pvp/pattern-removed",
        summary: "a pattern synonym was removed",
    },
    Rule {
        id: "pvp/pattern-changed",
        summary: "a pattern synonym signature changed",
    },
    Rule {
        id: "pvp/instance-added",
        summary: "an instance was added",
    },
    Rule {
        id: "pvp/instance-removed",
        summary: "an instance is no longer listed",
    },
    Rule {
        id: "pvp/instance-changed",
        summary: "an instance context or head changed",
    },
    Rule {
        id: "pvp/fixity-changed",
        summary: "an operator fixity changed",
    },
    Rule {
        id: "pvp/deprecation-added",
        summary: "a previously exported declaration is now deprecated",
    },
    Rule {
        id: "pvp/deprecation-removed",
        summary: "a declaration is no longer deprecated",
    },
    Rule {
        id: "pvp/whoops-stub",
        summary: "a declaration uses the Whoops error stub type",
    },
    Rule {
        id: "pvp/docs-unavailable",
        summary: "a Hoogle database is unavailable",
    },
    Rule {
        id: "pvp/renderer-drift-suspected",
        summary: "the two databases may come from different Haddock eras",
    },
    Rule {
        id: "pvp/re-export-only-module",
        summary: "a module has no named entries, so its surface is hidden",
    },
];

/// Look up a rule by identifier.
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.id == id)
}

/// Return every rule in catalog order.
pub fn all() -> &'static [Rule] {
    RULES
}
