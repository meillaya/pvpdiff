//! API diff and PVP classification.
//!
//! The diff compares two parsed surfaces, groups duplicate changes caused by
//! re-exporting modules, applies the default or strict policy, and produces the
//! change list and diagnostics consumed by the report layer.

use std::collections::{BTreeMap, BTreeSet};

use pvpdiff_parse::{ApiSurface, CabalMeta, Entity, EntityKey, EntityKind, Instance, ModuleApi};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::classify::{Classification, Confidence, Severity};
use crate::normalize::{
    Aliases, Comparison, compare_heads, compare_signatures, normalize_instance,
    normalize_instance_head,
};
use crate::rules;

/// Policy switches for the classifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Apply the literal PVP reading: count exposed Internal modules and treat
    /// instance removals, deprecations, fixity changes, and added class
    /// methods as breaking.
    pub strict: bool,
    /// Count changes in exposed `Internal`/`Private` modules without changing
    /// the other conservative rules.
    pub include_internal: bool,
}

impl DiffOptions {
    fn count_internal(self) -> bool {
        self.strict || self.include_internal
    }

    fn instance_removal_class(self) -> Classification {
        if self.strict {
            Classification::Breaking
        } else {
            Classification::Unknown
        }
    }

    fn fixity_class(self) -> Classification {
        if self.strict {
            Classification::Breaking
        } else {
            Classification::Unknown
        }
    }

    fn deprecation_class(self) -> Classification {
        if self.strict {
            Classification::Breaking
        } else {
            Classification::Unknown
        }
    }

    fn method_addition_class(self) -> Classification {
        if self.strict {
            Classification::Breaking
        } else {
            Classification::Unknown
        }
    }
}

/// One package side: a parsed surface and optional Cabal metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageApi {
    /// Parsed Hoogle surface.
    pub surface: ApiSurface,
    /// Parsed Cabal metadata, when available.
    pub cabal: Option<CabalMeta>,
}

/// One reported change.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// Stable identifier assigned after sorting and grouping.
    pub id: String,
    /// Rule identifier from the catalog.
    pub rule: String,
    /// Classification of this change.
    pub classification: Classification,
    /// Confidence in the classification.
    pub confidence: Confidence,
    /// Entity kind or `module`, `instance`, `fixity`, `reexport`.
    pub kind: String,
    /// Modules affected by the change.
    pub modules: Vec<String>,
    /// Entity or module name.
    pub name: Option<String>,
    /// Parent data type for constructors and fields.
    pub parent: Option<String>,
    /// Old signature or declaration head.
    pub before: Option<String>,
    /// New signature or declaration head.
    pub after: Option<String>,
    /// Whether the change contributes to the verdict.
    pub counted: bool,
    /// Short explanation for the report.
    pub message: String,
}

/// One report diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Stable code.
    pub code: String,
    /// Diagnostic severity.
    pub severity: Severity,
    /// Explanation.
    pub message: String,
}

/// Per-class change counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Breaking changes.
    pub breaking: usize,
    /// Additive changes.
    pub additive: usize,
    /// No-op changes.
    pub none: usize,
    /// Unknown changes.
    pub unknown: usize,
    /// Changes in internal-named modules that were not counted.
    pub internal: usize,
}

/// Result of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResult {
    /// Sorted, grouped changes.
    pub changes: Vec<Change>,
    /// Diagnostics, including parser warnings and policy notes.
    pub diagnostics: Vec<Diagnostic>,
}

impl DiffResult {
    /// Count changes, separating counted classes from internal notes.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for change in &self.changes {
            if !change.counted {
                counts.internal += 1;
                continue;
            }
            match change.classification {
                Classification::Breaking => counts.breaking += 1,
                Classification::Additive => counts.additive += 1,
                Classification::None => counts.none += 1,
                Classification::Unknown => counts.unknown += 1,
            }
        }
        counts
    }

    /// Aggregate counted classification.
    pub fn classification(&self) -> Classification {
        let mut classification = Classification::None;
        for change in &self.changes {
            if change.counted {
                classification = classification.max(change.classification);
            }
        }
        classification
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    rule: String,
    kind: String,
    name: Option<String>,
    parent: Option<String>,
    before: Option<String>,
    after: Option<String>,
    classification: Classification,
    confidence: Confidence,
}

#[derive(Debug, Clone)]
struct RawChange {
    change: Change,
    group: bool,
    apply_internal_policy: bool,
}

struct Builder<'a> {
    options: DiffOptions,
    old_aliases: &'a Aliases,
    new_aliases: &'a Aliases,
    changes: Vec<RawChange>,
    diagnostics: Vec<Diagnostic>,
    era_mismatch: bool,
}

/// Diff two package sides.
pub fn diff(old: &PackageApi, new: &PackageApi, options: DiffOptions) -> DiffResult {
    let old_aliases = Aliases::from_surface(&old.surface);
    let new_aliases = Aliases::from_surface(&new.surface);
    let mut builder = Builder {
        options,
        old_aliases: &old_aliases,
        new_aliases: &new_aliases,
        changes: Vec::new(),
        diagnostics: Vec::new(),
        era_mismatch: era_mismatch(&old.surface, &new.surface),
    };

    builder.diff_modules(old, new);
    builder.diff_instances(old, new);
    builder.diff_reexports(old.cabal.as_ref(), new.cabal.as_ref());
    builder.finish()
}

impl Builder<'_> {
    fn finish(mut self) -> DiffResult {
        let mut grouped: BTreeMap<GroupKey, Change> = BTreeMap::new();
        let mut ungrouped = Vec::new();

        for raw in &self.changes {
            if !raw.group {
                ungrouped.push(raw.change.clone());
                continue;
            }
            let key = GroupKey {
                rule: raw.change.rule.clone(),
                kind: raw.change.kind.clone(),
                name: raw.change.name.clone(),
                parent: raw.change.parent.clone(),
                before: raw.change.before.clone(),
                after: raw.change.after.clone(),
                classification: raw.change.classification,
                confidence: raw.change.confidence,
            };
            grouped
                .entry(key)
                .and_modify(|change| change.modules.extend(raw.change.modules.clone()))
                .or_insert_with(|| raw.change.clone());
        }

        let mut changes: Vec<Change> = grouped.into_values().chain(ungrouped).collect();
        for change in &mut changes {
            change.modules.sort();
            change.modules.dedup();
            if self
                .changes
                .iter()
                .find(|raw| {
                    raw.change.rule == change.rule
                        && raw.change.kind == change.kind
                        && raw.change.name == change.name
                        && raw.change.parent == change.parent
                        && raw.change.before == change.before
                        && raw.change.after == change.after
                })
                .is_some_and(|raw| raw.apply_internal_policy)
            {
                change.counted = self.options.count_internal()
                    || change
                        .modules
                        .iter()
                        .any(|module| !is_internal_module(module));
            }
        }

        self.apply_era_guard(&mut changes);
        changes.sort_by(|left, right| {
            left.rule
                .cmp(&right.rule)
                .then_with(|| left.modules.first().cmp(&right.modules.first()))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.parent.cmp(&right.parent))
                .then_with(|| left.before.cmp(&right.before))
        });
        for (index, change) in changes.iter_mut().enumerate() {
            change.id = format!("chg-{:04}", index + 1);
        }

        let counts = counts_of(&changes);
        if counts.internal > 0 {
            self.diagnostics.push(Diagnostic {
                code: "internal-modules-excluded".to_string(),
                severity: Severity::Info,
                message: format!(
                    "{} change(s) in Internal/Private modules were reported but not counted; use --include-internal or --strict",
                    counts.internal
                ),
            });
        }

        self.diagnostics.sort();
        self.diagnostics.dedup();
        DiffResult {
            changes,
            diagnostics: self.diagnostics,
        }
    }

    fn apply_era_guard(&mut self, changes: &mut [Change]) {
        if !self.era_mismatch {
            return;
        }
        self.diagnostics.push(Diagnostic {
            code: "renderer-drift-suspected".to_string(),
            severity: Severity::Warning,
            message: "the databases use different instance qualification styles; signature changes are downgraded to unknown"
                .to_string(),
        });
        for change in changes {
            if change.rule == "pvp/signature-changed" {
                change.rule = "pvp/signature-unverifiable".to_string();
                change.classification = Classification::Unknown;
                change.confidence = Confidence::Low;
                change.message.push_str("; renderer era differs");
            }
        }
    }

    fn diff_modules(&mut self, old: &PackageApi, new: &PackageApi) {
        let old_names: BTreeSet<&String> = old.surface.modules.keys().collect();
        let new_names: BTreeSet<&String> = new.surface.modules.keys().collect();

        for name in old_names.difference(&new_names) {
            self.push_module_change(
                name,
                "pvp/module-removed",
                Classification::Breaking,
                Confidence::High,
                "exposed module removed",
            );
        }
        for name in new_names.difference(&old_names) {
            self.push_module_change(
                name,
                "pvp/module-added",
                Classification::Additive,
                Confidence::High,
                "exposed module added",
            );
        }
        for name in old_names.intersection(&new_names) {
            let old_module = &old.surface.modules[*name];
            let new_module = &new.surface.modules[*name];
            if old_module.is_reexport_only() && new_module.is_reexport_only() {
                self.diagnostics.push(Diagnostic {
                    code: "re-export-only-module".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "module {name} has no named entries in either version; its module re-exports are not in Hoogle data"
                    ),
                });
                continue;
            }
            if old_module.named_entries > 0
                && new_module.named_entries == 0
                && entities_exist_elsewhere(old_module, &new.surface)
            {
                self.diagnostics.push(Diagnostic {
                    code: "re-export-only-module".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "module {name} became a re-export-only module; the named entries moved but remain in the package"
                    ),
                });
                continue;
            }
            let old_top = top_level(&old.surface.modules[*name]);
            let new_top = top_level(&new.surface.modules[*name]);
            self.diff_entities(name, old_module, new_module, &old_top, &new_top);
            self.diff_fixities(name, old_module, new_module);
        }
    }

    fn push_module_change(
        &mut self,
        module: &str,
        rule: &str,
        classification: Classification,
        confidence: Confidence,
        message: &str,
    ) {
        let change = Change {
            id: String::new(),
            rule: rule.to_string(),
            classification,
            confidence,
            kind: "module".to_string(),
            modules: vec![module.to_string()],
            name: Some(module.to_string()),
            parent: None,
            before: None,
            after: None,
            counted: true,
            message: message.to_string(),
        };
        self.push(module, change, false, true);
    }

    fn diff_entities(
        &mut self,
        module: &str,
        old_module: &ModuleApi,
        new_module: &ModuleApi,
        old_top: &BTreeMap<&EntityKey, &Entity>,
        new_top: &BTreeMap<&EntityKey, &Entity>,
    ) {
        let old_by_name: BTreeMap<&str, &EntityKey> = old_top
            .keys()
            .map(|key| (key.name.as_str(), *key))
            .collect();
        let new_by_name: BTreeMap<&str, &EntityKey> = new_top
            .keys()
            .map(|key| (key.name.as_str(), *key))
            .collect();

        for (key, old_entity) in old_top {
            if let Some(new_entity) = new_top.get(key) {
                self.compare_common(module, old_module, new_module, key, old_entity, new_entity);
                continue;
            }
            if let Some(new_key) = new_by_name.get(key.name.as_str()) {
                if new_key.kind != key.kind {
                    let change = Change {
                        id: String::new(),
                        rule: "pvp/entity-kind-changed".to_string(),
                        classification: Classification::Breaking,
                        confidence: Confidence::High,
                        kind: key.kind.as_str().to_string(),
                        modules: Vec::new(),
                        name: Some(key.name.clone()),
                        parent: key.parent.clone(),
                        before: entity_text(old_entity).map(ToString::to_string),
                        after: entity_text_for(new_top, new_key),
                        counted: true,
                        message: format!("{} changed kind", key.name),
                    };
                    self.push(module, change, true, true);
                }
                continue;
            }
            self.push_removed(module, old_module, key, old_entity);
        }

        for (key, new_entity) in new_top {
            if old_top.contains_key(key) || old_by_name.contains_key(key.name.as_str()) {
                continue;
            }
            self.push_added(module, new_module, key, new_entity);
        }

        for (key, old_entity) in old_top {
            if let Some(new_entity) = new_top.get(key) {
                self.compare_children(module, old_module, new_module, key, old_entity, new_entity);
            }
        }
    }

    fn compare_common(
        &mut self,
        module: &str,
        _old_module: &ModuleApi,
        _new_module: &ModuleApi,
        key: &EntityKey,
        old_entity: &Entity,
        new_entity: &Entity,
    ) {
        match key.kind {
            EntityKind::Function => {
                if has_whoops(old_entity) || has_whoops(new_entity) {
                    self.push_rule(
                        module,
                        key,
                        "pvp/whoops-stub",
                        Classification::Unknown,
                        Confidence::Medium,
                        entity_text(old_entity),
                        entity_text(new_entity),
                        "declaration uses the Whoops error stub",
                    );
                } else if let (Some(old_signature), Some(new_signature)) =
                    (&old_entity.signature, &new_entity.signature)
                {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/signature-changed",
                        "pvp/signature-unverifiable",
                        compare_signatures(
                            old_signature,
                            new_signature,
                            self.old_aliases,
                            self.new_aliases,
                        ),
                        old_signature,
                        new_signature,
                    );
                }
            }
            EntityKind::Pattern => {
                if let (Some(old_signature), Some(new_signature)) =
                    (&old_entity.signature, &new_entity.signature)
                {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/pattern-changed",
                        "pvp/signature-unverifiable",
                        compare_signatures(
                            old_signature,
                            new_signature,
                            self.old_aliases,
                            self.new_aliases,
                        ),
                        old_signature,
                        new_signature,
                    );
                }
            }
            EntityKind::Data | EntityKind::Newtype => {
                if let (Some(old_head), Some(new_head)) = (&old_entity.head, &new_entity.head) {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/data-declaration-changed",
                        "pvp/data-declaration-changed",
                        compare_heads(old_head, new_head, self.old_aliases, self.new_aliases),
                        old_head,
                        new_head,
                    );
                }
            }
            EntityKind::TypeSynonym => {
                if let (Some(old_head), Some(new_head)) = (&old_entity.head, &new_entity.head) {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/type-synonym-changed",
                        "pvp/type-synonym-changed",
                        compare_heads(old_head, new_head, self.old_aliases, self.new_aliases),
                        old_head,
                        new_head,
                    );
                }
            }
            EntityKind::Class => {
                if let (Some(old_head), Some(new_head)) = (&old_entity.head, &new_entity.head) {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/class-declaration-changed",
                        "pvp/class-declaration-changed",
                        compare_heads(old_head, new_head, self.old_aliases, self.new_aliases),
                        old_head,
                        new_head,
                    );
                }
            }
            EntityKind::TypeFamily | EntityKind::DataFamily => {
                if let (Some(old_head), Some(new_head)) = (&old_entity.head, &new_entity.head) {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/type-family-changed",
                        "pvp/type-family-changed",
                        compare_heads(old_head, new_head, self.old_aliases, self.new_aliases),
                        old_head,
                        new_head,
                    );
                }
            }
            EntityKind::Constructor | EntityKind::RecordField => {}
        }

        if old_entity.deprecated != new_entity.deprecated {
            let (rule, classification, confidence, message) = if new_entity.deprecated {
                (
                    "pvp/deprecation-added",
                    self.options.deprecation_class(),
                    Confidence::Medium,
                    "declaration is now deprecated",
                )
            } else {
                (
                    "pvp/deprecation-removed",
                    Classification::None,
                    Confidence::Low,
                    "declaration is no longer deprecated",
                )
            };
            self.push_rule(
                module,
                key,
                rule,
                classification,
                confidence,
                None,
                None,
                message,
            );
        }
    }

    fn compare_children(
        &mut self,
        module: &str,
        old_module: &ModuleApi,
        new_module: &ModuleApi,
        key: &EntityKey,
        _old_entity: &Entity,
        _new_entity: &Entity,
    ) {
        if !matches!(key.kind, EntityKind::Data | EntityKind::Newtype) {
            return;
        }
        self.diff_constructors(module, old_module, new_module, &key.name);
        self.diff_fields(module, old_module, new_module, &key.name);
    }

    fn diff_constructors(
        &mut self,
        module: &str,
        old_module: &ModuleApi,
        new_module: &ModuleApi,
        parent: &str,
    ) {
        let old_keys = ordered_children(old_module, parent, EntityKind::Constructor);
        let new_keys = ordered_children(new_module, parent, EntityKind::Constructor);
        let old_map: BTreeMap<&str, &EntityKey> = old_keys
            .iter()
            .map(|key| (key.name.as_str(), *key))
            .collect();
        let new_map: BTreeMap<&str, &EntityKey> = new_keys
            .iter()
            .map(|key| (key.name.as_str(), *key))
            .collect();

        for key in &old_keys {
            if let Some(new_key) = new_map.get(key.name.as_str()) {
                let Some(old_entity) = old_module.entities.get(*key) else {
                    continue;
                };
                let Some(new_entity) = new_module.entities.get(*new_key) else {
                    continue;
                };
                if let (Some(old_signature), Some(new_signature)) =
                    (&old_entity.signature, &new_entity.signature)
                {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/constructor-changed",
                        "pvp/constructor-changed",
                        compare_signatures(
                            old_signature,
                            new_signature,
                            self.old_aliases,
                            self.new_aliases,
                        ),
                        old_signature,
                        new_signature,
                    );
                }
            } else {
                self.push_rule(
                    module,
                    key,
                    "pvp/constructor-removed",
                    Classification::Breaking,
                    Confidence::High,
                    old_module
                        .entities
                        .get(*key)
                        .and_then(|entity| entity.signature.as_deref()),
                    None,
                    "constructor removed",
                );
            }
        }
        for key in &new_keys {
            if !old_map.contains_key(key.name.as_str()) {
                self.push_rule(
                    module,
                    key,
                    "pvp/constructor-added",
                    Classification::Breaking,
                    Confidence::High,
                    None,
                    new_module
                        .entities
                        .get(*key)
                        .and_then(|entity| entity.signature.as_deref()),
                    "constructor added",
                );
            }
        }

        if old_keys.len() == new_keys.len() {
            let old_order: Vec<&str> = old_keys.iter().map(|key| key.name.as_str()).collect();
            let new_order: Vec<&str> = new_keys.iter().map(|key| key.name.as_str()).collect();
            if old_order != new_order {
                let key = &old_keys[0];
                let old_order = old_order.join(", ");
                let new_order = new_order.join(", ");
                self.push_rule(
                    module,
                    key,
                    "pvp/constructor-order-changed",
                    Classification::Unknown,
                    Confidence::Medium,
                    Some(&old_order),
                    Some(&new_order),
                    "constructor order changed",
                );
            }
        }
    }

    fn diff_fields(
        &mut self,
        module: &str,
        old_module: &ModuleApi,
        new_module: &ModuleApi,
        parent: &str,
    ) {
        let old_keys = ordered_children(old_module, parent, EntityKind::RecordField);
        let new_keys = ordered_children(new_module, parent, EntityKind::RecordField);
        let old_map: BTreeMap<&str, &EntityKey> = old_keys
            .iter()
            .map(|key| (key.name.as_str(), *key))
            .collect();
        let new_map: BTreeMap<&str, &EntityKey> = new_keys
            .iter()
            .map(|key| (key.name.as_str(), *key))
            .collect();

        for key in &old_keys {
            if let Some(new_key) = new_map.get(key.name.as_str()) {
                let Some(old_entity) = old_module.entities.get(*key) else {
                    continue;
                };
                let Some(new_entity) = new_module.entities.get(*new_key) else {
                    continue;
                };
                if let (Some(old_signature), Some(new_signature)) =
                    (&old_entity.signature, &new_entity.signature)
                {
                    self.push_comparison(
                        module,
                        key,
                        "pvp/field-changed",
                        "pvp/field-changed",
                        compare_signatures(
                            old_signature,
                            new_signature,
                            self.old_aliases,
                            self.new_aliases,
                        ),
                        old_signature,
                        new_signature,
                    );
                }
            } else {
                self.push_rule(
                    module,
                    key,
                    "pvp/field-removed",
                    Classification::Breaking,
                    Confidence::High,
                    old_module
                        .entities
                        .get(*key)
                        .and_then(|entity| entity.signature.as_deref()),
                    None,
                    "record field removed",
                );
            }
        }
        for key in &new_keys {
            if !old_map.contains_key(key.name.as_str()) {
                self.push_rule(
                    module,
                    key,
                    "pvp/field-added",
                    Classification::Breaking,
                    Confidence::High,
                    None,
                    new_module
                        .entities
                        .get(*key)
                        .and_then(|entity| entity.signature.as_deref()),
                    "record field added",
                );
            }
        }
    }

    fn push_removed(
        &mut self,
        module: &str,
        old_module: &ModuleApi,
        key: &EntityKey,
        entity: &Entity,
    ) {
        let is_method =
            key.kind == EntityKind::Function && looks_like_class_method(entity, old_module);
        let (rule, classification, confidence, message) = if has_whoops(entity) {
            (
                "pvp/whoops-stub",
                Classification::Unknown,
                Confidence::Medium,
                "Whoops stub removed",
            )
        } else if is_method {
            (
                "pvp/class-method-removed",
                Classification::Breaking,
                Confidence::Medium,
                "class method removed",
            )
        } else {
            (
                "pvp/entity-removed",
                Classification::Breaking,
                Confidence::High,
                "exported declaration removed",
            )
        };
        self.push_rule(
            module,
            key,
            rule,
            classification,
            confidence,
            entity_text(entity),
            None,
            message,
        );
    }

    fn push_added(
        &mut self,
        module: &str,
        new_module: &ModuleApi,
        key: &EntityKey,
        entity: &Entity,
    ) {
        if has_whoops(entity) {
            self.push_rule(
                module,
                key,
                "pvp/whoops-stub",
                Classification::Unknown,
                Confidence::Medium,
                None,
                entity_text(entity),
                "Whoops stub added",
            );
            return;
        }
        if key.kind == EntityKind::Function && looks_like_class_method(entity, new_module) {
            self.push_rule(
                module,
                key,
                "pvp/class-method-added",
                self.options.method_addition_class(),
                Confidence::Medium,
                None,
                entity_text(entity),
                "signature looks like a new class method; a default body cannot be ruled out",
            );
            return;
        }
        let rule = if key.kind == EntityKind::Pattern {
            "pvp/pattern-added"
        } else {
            "pvp/entity-added"
        };
        self.push_rule(
            module,
            key,
            rule,
            Classification::Additive,
            Confidence::High,
            None,
            entity_text(entity),
            "exported declaration added",
        );
    }

    #[allow(clippy::too_many_arguments)]
    // The six report fields are independent values; a builder would only move
    // the argument list to the call site.
    fn push_comparison(
        &mut self,
        module: &str,
        key: &EntityKey,
        changed_rule: &str,
        unknown_rule: &str,
        comparison: Comparison,
        old: &str,
        new: &str,
    ) {
        match comparison {
            Comparison::Same => {}
            Comparison::Changed => {
                self.push_rule(
                    module,
                    key,
                    changed_rule,
                    Classification::Breaking,
                    Confidence::High,
                    Some(old),
                    Some(new),
                    "declaration changed",
                );
            }
            Comparison::Unknown(reason) => {
                self.push_rule(
                    module,
                    key,
                    unknown_rule,
                    Classification::Unknown,
                    Confidence::Medium,
                    Some(old),
                    Some(new),
                    reason,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    // The report fields are independent values; grouping them would obscure
    // the rule call sites.
    fn push_rule(
        &mut self,
        module: &str,
        key: &EntityKey,
        rule: &str,
        classification: Classification,
        confidence: Confidence,
        before: Option<&str>,
        after: Option<&str>,
        message: &str,
    ) {
        let change = Change {
            id: String::new(),
            rule: rule.to_string(),
            classification,
            confidence,
            kind: key.kind.as_str().to_string(),
            modules: Vec::new(),
            name: Some(key.name.clone()),
            parent: key.parent.clone(),
            before: before.map(ToString::to_string),
            after: after.map(ToString::to_string),
            counted: true,
            message: message.to_string(),
        };
        self.push(module, change, true, true);
    }

    fn push(&mut self, module: &str, mut change: Change, group: bool, policy: bool) {
        change.modules.push(module.to_string());
        self.changes.push(RawChange {
            change,
            group,
            apply_internal_policy: policy,
        });
    }

    fn diff_fixities(&mut self, module: &str, old_module: &ModuleApi, new_module: &ModuleApi) {
        let old_names: BTreeSet<&str> = old_module
            .entities
            .keys()
            .map(|key| key.name.as_str())
            .collect();
        let new_names: BTreeSet<&str> = new_module
            .entities
            .keys()
            .map(|key| key.name.as_str())
            .collect();

        for (operator, old_fixity) in &old_module.fixities {
            if old_names.contains(operator.as_str()) != new_names.contains(operator.as_str()) {
                continue;
            }
            match new_module.fixities.get(operator) {
                Some(new_fixity) if new_fixity == old_fixity => {}
                Some(new_fixity) => {
                    self.push_fixity(
                        module,
                        operator,
                        self.options.fixity_class(),
                        Some(&format_fixity(old_fixity)),
                        Some(&format_fixity(new_fixity)),
                    );
                }
                None => {
                    self.push_fixity(
                        module,
                        operator,
                        self.options.fixity_class(),
                        Some(&format_fixity(old_fixity)),
                        None,
                    );
                }
            }
        }
        for (operator, new_fixity) in &new_module.fixities {
            if old_module.fixities.contains_key(operator) {
                continue;
            }
            if old_names.contains(operator.as_str()) != new_names.contains(operator.as_str()) {
                continue;
            }
            self.push_fixity(
                module,
                operator,
                self.options.fixity_class(),
                None,
                Some(&format_fixity(new_fixity)),
            );
        }
    }

    fn push_fixity(
        &mut self,
        module: &str,
        operator: &str,
        classification: Classification,
        before: Option<&str>,
        after: Option<&str>,
    ) {
        let change = Change {
            id: String::new(),
            rule: "pvp/fixity-changed".to_string(),
            classification,
            confidence: Confidence::Medium,
            kind: "fixity".to_string(),
            modules: Vec::new(),
            name: Some(operator.to_string()),
            parent: None,
            before: before.map(ToString::to_string),
            after: after.map(ToString::to_string),
            counted: true,
            message: "fixity changed; operators may parse differently".to_string(),
        };
        self.push(module, change, false, true);
    }

    fn diff_instances(&mut self, old: &PackageApi, new: &PackageApi) {
        let old_records = collect_instances(&old.surface);
        let new_records = collect_instances(&new.surface);
        let old_by_key: BTreeMap<&str, &InstanceRecord<'_>> = old_records
            .iter()
            .map(|record| (record.key.as_str(), record))
            .collect();
        let new_by_key: BTreeMap<&str, &InstanceRecord<'_>> = new_records
            .iter()
            .map(|record| (record.key.as_str(), record))
            .collect();
        let mut new_by_head: BTreeMap<&str, Vec<&InstanceRecord<'_>>> = BTreeMap::new();
        for record in &new_records {
            new_by_head
                .entry(record.head_key.as_str())
                .or_default()
                .push(record);
        }

        let package_modules: BTreeSet<&str> = old
            .surface
            .module_names()
            .chain(new.surface.module_names())
            .map(String::as_str)
            .collect();
        let mut handled_new: BTreeSet<&str> = BTreeSet::new();

        for record in &old_records {
            if new_by_key.contains_key(record.key.as_str()) {
                continue;
            }
            if let Some(candidates) = new_by_head.get(record.head_key.as_str())
                && let Some(candidate) = candidates
                    .iter()
                    .find(|candidate| !old_by_key.contains_key(candidate.key.as_str()))
            {
                handled_new.insert(candidate.key.as_str());
                let mut change = instance_change(
                    "pvp/instance-changed",
                    self.options.instance_removal_class(),
                    Confidence::Medium,
                    "instance context changed",
                );
                change.modules = vec![record.module.to_string(), candidate.module.to_string()];
                change.name = Some(format!(
                    "{} {}",
                    record.instance.class, record.instance.head
                ));
                change.before = Some(record.instance.raw.clone());
                change.after = Some(candidate.instance.raw.clone());
                self.changes.push(RawChange {
                    change,
                    group: false,
                    apply_internal_policy: false,
                });
                continue;
            }
            let mut change = instance_change(
                "pvp/instance-removed",
                self.options.instance_removal_class(),
                Confidence::Low,
                "instance is no longer listed; doc-build GHC changes can cause false removals",
            );
            change.modules = vec![record.module.to_string()];
            change.name = Some(format!(
                "{} {}",
                record.instance.class, record.instance.head
            ));
            change.before = Some(record.instance.raw.clone());
            self.changes.push(RawChange {
                change,
                group: false,
                apply_internal_policy: false,
            });
        }

        for record in &new_records {
            if old_by_key.contains_key(record.key.as_str())
                || handled_new.contains(record.key.as_str())
            {
                continue;
            }
            let orphan = is_orphan(record.instance, &package_modules);
            let (classification, confidence) = if orphan {
                (Classification::Breaking, Confidence::Medium)
            } else {
                (Classification::Additive, Confidence::High)
            };
            let mut change = instance_change(
                "pvp/instance-added",
                classification,
                confidence,
                if orphan {
                    "orphan instance added; PVP requires a major bump"
                } else {
                    "instance added"
                },
            );
            change.modules = vec![record.module.to_string()];
            change.name = Some(format!(
                "{} {}",
                record.instance.class, record.instance.head
            ));
            change.after = Some(record.instance.raw.clone());
            self.changes.push(RawChange {
                change,
                group: false,
                apply_internal_policy: false,
            });
        }
    }

    fn diff_reexports(&mut self, old: Option<&CabalMeta>, new: Option<&CabalMeta>) {
        let (Some(old), Some(new)) = (old, new) else {
            return;
        };
        if old.conditional || new.conditional {
            self.diagnostics.push(Diagnostic {
                code: "conditional-api".to_string(),
                severity: Severity::Warning,
                message:
                    "Cabal conditionals are present; the version may not uniquely identify the API"
                        .to_string(),
            });
        }
        let old_modules: BTreeSet<&String> = old
            .libraries
            .iter()
            .flat_map(|library| library.reexported_modules.iter())
            .collect();
        let new_modules: BTreeSet<&String> = new
            .libraries
            .iter()
            .flat_map(|library| library.reexported_modules.iter())
            .collect();
        for module in old_modules.difference(&new_modules) {
            self.push_reexport(
                module,
                Classification::Breaking,
                "reexported module removed",
            );
        }
        for module in new_modules.difference(&old_modules) {
            self.push_reexport(module, Classification::Additive, "reexported module added");
        }
    }

    fn push_reexport(&mut self, module: &str, classification: Classification, message: &str) {
        let change = Change {
            id: String::new(),
            rule: "pvp/reexport-changed".to_string(),
            classification,
            confidence: Confidence::High,
            kind: "reexport".to_string(),
            modules: vec![module.to_string()],
            name: Some(module.to_string()),
            parent: None,
            before: None,
            after: None,
            counted: true,
            message: message.to_string(),
        };
        self.changes.push(RawChange {
            change,
            group: false,
            apply_internal_policy: false,
        });
    }
}

struct InstanceRecord<'a> {
    module: &'a str,
    instance: &'a Instance,
    key: String,
    head_key: String,
}

fn collect_instances(surface: &ApiSurface) -> Vec<InstanceRecord<'_>> {
    let mut records = Vec::new();
    for (module, api) in &surface.modules {
        for instance in &api.instances {
            records.push(InstanceRecord {
                module,
                instance,
                key: normalize_instance(&instance.raw),
                head_key: normalize_instance_head(instance),
            });
        }
    }
    records
}

fn instance_change(
    rule: &str,
    classification: Classification,
    confidence: Confidence,
    message: &str,
) -> Change {
    Change {
        id: String::new(),
        rule: rule.to_string(),
        classification,
        confidence,
        kind: "instance".to_string(),
        modules: Vec::new(),
        name: None,
        parent: None,
        before: None,
        after: None,
        counted: true,
        message: message.to_string(),
    }
}

fn is_orphan(instance: &Instance, package_modules: &BTreeSet<&str>) -> bool {
    let class_module = qualified_prefix(&instance.class);
    let type_module = first_qualified_prefix(&instance.head);
    match (class_module, type_module) {
        (Some(class_module), Some(type_module)) => {
            !package_modules.contains(class_module.as_str())
                && !package_modules.contains(type_module.as_str())
        }
        _ => false,
    }
}

fn qualified_prefix(name: &str) -> Option<String> {
    name.rsplit_once('.').map(|(prefix, _)| prefix.to_string())
}

fn first_qualified_prefix(text: &str) -> Option<String> {
    // The first word is the class token; the type head follows it.
    let mut words = text.split_whitespace();
    let _class = words.next()?;
    for word in words {
        let word =
            word.trim_matches(|character: char| matches!(character, '(' | ')' | '[' | ']' | ','));
        if !word.is_empty() && word.contains('.') {
            return qualified_prefix(word);
        }
    }
    None
}

fn top_level(module: &ModuleApi) -> BTreeMap<&EntityKey, &Entity> {
    module
        .entities
        .iter()
        .filter(|(key, _)| key.kind.is_top_level())
        .collect()
}

fn ordered_children<'a>(
    module: &'a ModuleApi,
    parent: &str,
    kind: EntityKind,
) -> Vec<&'a EntityKey> {
    module
        .entity_order
        .iter()
        .filter(|key| key.parent.as_deref() == Some(parent) && key.kind == kind)
        .filter(|key| module.entities.contains_key(*key))
        .collect()
}

fn looks_like_class_method(entity: &Entity, module: &ModuleApi) -> bool {
    let Some(signature) = &entity.signature else {
        return false;
    };
    let Some(context) = signature.split("=>").next() else {
        return false;
    };
    module
        .entities
        .keys()
        .filter(|key| key.kind == EntityKind::Class)
        .any(|key| contains_word(context, &key.name))
}

fn contains_word(text: &str, word: &str) -> bool {
    let mut start = 0usize;
    while let Some(position) = text[start..].find(word) {
        let absolute = start + position;
        let before = text[..absolute].chars().next_back();
        let after = text[absolute + word.len()..].chars().next();
        let boundary_before = before.is_none_or(|character| !is_identifier(character));
        let boundary_after = after.is_none_or(|character| !is_identifier(character));
        if boundary_before && boundary_after {
            return true;
        }
        start = absolute + word.len();
    }
    false
}

fn is_identifier(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '\''
}

fn has_whoops(entity: &Entity) -> bool {
    entity
        .signature
        .as_deref()
        .is_some_and(|signature| signature.contains("Whoops"))
}

fn entity_text(entity: &Entity) -> Option<&str> {
    entity.head.as_deref().or(entity.signature.as_deref())
}

fn entity_text_for(top: &BTreeMap<&EntityKey, &Entity>, key: &EntityKey) -> Option<String> {
    top.get(key)
        .and_then(|entity| entity_text(entity))
        .map(ToString::to_string)
}

fn format_fixity(fixity: &pvpdiff_parse::Fixity) -> String {
    let associativity = match fixity.associativity {
        pvpdiff_parse::Associativity::Left => "infixl",
        pvpdiff_parse::Associativity::Right => "infixr",
        pvpdiff_parse::Associativity::None => "infix",
    };
    format!("{associativity} {} {}", fixity.precedence, fixity.operator)
}

fn entities_exist_elsewhere(old_module: &ModuleApi, new: &ApiSurface) -> bool {
    old_module
        .entities
        .keys()
        .filter(|key| key.kind.is_top_level())
        .all(|key| {
            new.modules
                .values()
                .any(|module| module.entities.contains_key(key))
        })
}

fn is_internal_module(module: &str) -> bool {
    module.split('.').any(|part| {
        part.eq_ignore_ascii_case("internal")
            || part.eq_ignore_ascii_case("internals")
            || part.eq_ignore_ascii_case("private")
    })
}

fn counts_of(changes: &[Change]) -> Counts {
    let mut counts = Counts::default();
    for change in changes {
        if !change.counted {
            counts.internal += 1;
            continue;
        }
        match change.classification {
            Classification::Breaking => counts.breaking += 1,
            Classification::Additive => counts.additive += 1,
            Classification::None => counts.none += 1,
            Classification::Unknown => counts.unknown += 1,
        }
    }
    counts
}

fn era_mismatch(old: &ApiSurface, new: &ApiSurface) -> bool {
    fn styles(surface: &ApiSurface) -> (bool, bool) {
        let mut qualified = false;
        let mut unqualified = false;
        for module in surface.modules.values() {
            for instance in &module.instances {
                if instance.raw.contains('.') {
                    qualified = true;
                } else {
                    unqualified = true;
                }
            }
        }
        (qualified, unqualified)
    }
    let (old_qualified, old_unqualified) = styles(old);
    let (new_qualified, new_unqualified) = styles(new);
    (old_qualified && !old_unqualified && new_unqualified && !new_qualified)
        || (old_unqualified && !old_qualified && new_qualified && !new_unqualified)
}

/// Look up a rule summary, used by the CLI rule listing.
pub fn catalog() -> &'static [rules::Rule] {
    rules::all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvpdiff_parse::parse_hoogle;

    /// Build a surface from a body string with one module named Demo.
    fn surface(version: &str, body: &str) -> PackageApi {
        let database = format!(
            "-- Hoogle documentation, generated by Haddock\n-- See Hoogle, http://www.haskell.org/hoogle/\n\n@package demo\n@version {version}\n\nmodule Demo\n\n{body}\n"
        );
        PackageApi {
            surface: parse_hoogle(&database).expect("parse"),
            cabal: None,
        }
    }

    /// Check that a removed top-level function is a breaking candidate and a
    /// new signature is additive.
    #[test]
    fn classifies_removal_and_addition() {
        let old = surface("1.0.0", "foo :: Int\nbar :: Int");
        let new = surface("1.1.0", "bar :: Int\nbaz :: Int");
        let result = diff(&old, &new, DiffOptions::default());
        assert_eq!(result.classification(), Classification::Breaking);
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/entity-removed"
                    && change.name.as_deref() == Some("foo"))
        );
        assert!(result.changes.iter().any(
            |change| change.rule == "pvp/entity-added" && change.name.as_deref() == Some("baz")
        ));
        assert_eq!(result.counts().breaking, 1);
        assert_eq!(result.counts().additive, 1);
    }

    /// Check that a changed signature is breaking and that a changed
    /// constructor is grouped under the data type.
    #[test]
    fn classifies_signature_and_constructor_changes() {
        let old = surface("1.0.0", "data T\nT :: Int -> T\nfoo :: Int -> Int");
        let new = surface(
            "1.1.0",
            "data T\nT :: String -> T\nfoo :: Int -> Int -> Int",
        );
        let result = diff(&old, &new, DiffOptions::default());
        assert_eq!(result.classification(), Classification::Breaking);
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/constructor-changed")
        );
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/signature-changed")
        );
    }

    /// Check that an internal-named module is reported but not counted by
    /// default, and counted under strict mode.
    #[test]
    fn applies_internal_module_policy() {
        let old = surface("1.0.0", "module Demo.Internal\n\nfoo :: Int");
        let new = surface("1.1.0", "module Demo.Internal\n\nbar :: Int");
        let default = diff(&old, &new, DiffOptions::default());
        assert_eq!(default.counts().internal, 2);
        assert_eq!(default.classification(), Classification::None);
        let strict = diff(
            &old,
            &new,
            DiffOptions {
                strict: true,
                include_internal: true,
            },
        );
        assert_eq!(strict.classification(), Classification::Breaking);
    }

    /// Check that a type synonym difference in a signature becomes unknown
    /// rather than breaking.
    #[test]
    fn treats_synonym_signature_as_unknown() {
        let old = surface("1.0.0", "type Forest a = [a]\ndfs :: Forest Int -> Int");
        let new = surface("1.1.0", "type Forest a = [a]\ndfs :: [Int] -> Int");
        let result = diff(&old, &new, DiffOptions::default());
        assert_eq!(result.classification(), Classification::Unknown);
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/signature-unverifiable")
        );
    }

    /// Check that an added instance is additive and a removed instance is
    /// unknown by default.
    #[test]
    fn applies_instance_policy() {
        let old = surface("1.0.0", "data T\nT :: T\ninstance Show T");
        let new = surface("1.1.0", "data T\nT :: T\ninstance Eq T");
        let result = diff(&old, &new, DiffOptions::default());
        assert_eq!(result.classification(), Classification::Unknown);
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/instance-removed"
                    && change.classification == Classification::Unknown)
        );
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.rule == "pvp/instance-added"
                    && change.classification == Classification::Additive)
        );
    }
}
