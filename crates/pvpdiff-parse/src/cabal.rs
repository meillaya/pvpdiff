//! Adapter from `cabalist-parser` to the small metadata model pvpdiff needs.
//!
//! The adapter owns four gaps in the upstream AST: `reexported-modules` is an
//! unstructured field, common stanzas are referenced by name but not merged,
//! conditionals are not resolved, and old Cabal files may produce diagnostics
//! while still having a usable name and version. Unparseable files return
//! `meta: None` with diagnostics; the caller reports an unknown result instead
//! of guessing.

use std::collections::BTreeSet;

use cabalist_parser::ast::{self, derive_ast};
use cabalist_parser::parse;

use crate::model::{CabalMeta, ComponentKind, DependencyMeta, LibraryMeta, ParsedCabal};

const REEXPORTED_MODULES: &str = "reexported-modules";
const EXPOSED_MODULES: &str = "exposed-modules";
const X_REVISION: &str = "x-revision";
const MAX_COMMON_DEPTH: usize = 8;

/// Parse one `.cabal` file.
pub fn parse_cabal(source: &str) -> ParsedCabal {
    let result = parse(source);
    let mut diagnostics: Vec<String> = result
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{:?}: {}", diagnostic.severity, diagnostic.message))
        .collect();

    let ast = derive_ast(&result.cst);
    let Some(name) = ast.name.map(ToString::to_string) else {
        diagnostics.push("missing package name".to_string());
        return ParsedCabal {
            meta: None,
            diagnostics,
        };
    };
    let Some(version) = ast.version.as_ref().map(ToString::to_string) else {
        diagnostics.push("missing package version".to_string());
        return ParsedCabal {
            meta: None,
            diagnostics,
        };
    };

    let revision = ast
        .other_fields
        .iter()
        .find(|field| field.name == X_REVISION)
        .and_then(|field| field.value.trim().parse::<u32>().ok());

    let mut libraries = Vec::new();
    if let Some(library) = ast.library.as_ref() {
        libraries.push(convert_library(library, &ast, &mut diagnostics));
    }
    for library in &ast.named_libraries {
        libraries.push(convert_library(library, &ast, &mut diagnostics));
    }

    let mut conditional = libraries.iter().any(|library| library.conditional_surface);
    for component in ast.all_components() {
        if has_conditionals(component.fields()) {
            conditional = true;
        }
    }

    ParsedCabal {
        meta: Some(CabalMeta {
            name,
            version,
            revision,
            libraries,
            conditional,
        }),
        diagnostics,
    }
}

fn convert_library(
    library: &ast::Library<'_>,
    ast: &ast::CabalFile<'_>,
    diagnostics: &mut Vec<String>,
) -> LibraryMeta {
    let mut exposed_modules: Vec<String> = library
        .exposed_modules
        .iter()
        .map(|module| (*module).to_string())
        .collect();

    let mut reexported_modules = Vec::new();
    let mut conditional_surface = false;
    for field in &library.fields.other_fields {
        if field.name == REEXPORTED_MODULES {
            reexported_modules.extend(parse_reexported_modules(&field.value));
        }
    }
    collect_conditional_surface(
        &library.fields.conditionals,
        &mut exposed_modules,
        &mut reexported_modules,
        &mut conditional_surface,
    );

    let mut dependencies = Vec::new();
    let mut seen_common = BTreeSet::new();
    collect_common_stanzas(
        &library.fields.imports,
        ast,
        &mut seen_common,
        0,
        &mut dependencies,
        diagnostics,
    );
    collect_dependencies(&library.fields.build_depends, &mut dependencies);
    collect_conditional_dependencies(
        &library.fields.conditionals,
        &mut dependencies,
        &mut conditional_surface,
    );

    dedupe(&mut exposed_modules);
    dedupe(&mut reexported_modules);
    dedupe(&mut dependencies);

    LibraryMeta {
        name: library.fields.name.map(ToString::to_string),
        exposed_modules,
        reexported_modules,
        dependencies,
        conditional_surface,
    }
}

fn collect_common_stanzas(
    imports: &[&str],
    ast: &ast::CabalFile<'_>,
    seen: &mut BTreeSet<String>,
    depth: usize,
    dependencies: &mut Vec<DependencyMeta>,
    diagnostics: &mut Vec<String>,
) {
    if depth > MAX_COMMON_DEPTH {
        diagnostics.push("common stanza import depth exceeded".to_string());
        return;
    }
    for name in imports {
        if !seen.insert((*name).to_string()) {
            continue;
        }
        let Some(stanza) = ast
            .common_stanzas
            .iter()
            .find(|stanza| stanza.name == *name)
        else {
            diagnostics.push(format!("missing common stanza {name}"));
            continue;
        };
        collect_dependencies(&stanza.fields.build_depends, dependencies);
        collect_common_stanzas(
            &stanza.fields.imports,
            ast,
            seen,
            depth + 1,
            dependencies,
            diagnostics,
        );
    }
}

fn collect_dependencies(source: &[ast::Dependency<'_>], output: &mut Vec<DependencyMeta>) {
    for dependency in source {
        output.push(DependencyMeta {
            package: dependency.package.to_string(),
            range: dependency
                .version_range
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "-any".to_string()),
            component: ComponentKind::Library,
        });
    }
}

fn collect_conditional_dependencies(
    conditionals: &[ast::Conditional<'_>],
    output: &mut Vec<DependencyMeta>,
    conditional_surface: &mut bool,
) {
    for conditional in conditionals {
        collect_dependencies(&conditional.then_deps, output);
        collect_dependencies(&conditional.else_deps, output);
        if conditional
            .then_fields
            .iter()
            .chain(conditional.else_fields.iter())
            .any(|field| field.name == EXPOSED_MODULES || field.name == REEXPORTED_MODULES)
        {
            *conditional_surface = true;
        }
        collect_conditional_dependencies(
            &conditional.then_conditionals,
            output,
            conditional_surface,
        );
        collect_conditional_dependencies(
            &conditional.else_conditionals,
            output,
            conditional_surface,
        );
    }
}

fn collect_conditional_surface(
    conditionals: &[ast::Conditional<'_>],
    exposed_modules: &mut Vec<String>,
    reexported_modules: &mut Vec<String>,
    conditional_surface: &mut bool,
) {
    for conditional in conditionals {
        for field in conditional
            .then_fields
            .iter()
            .chain(conditional.else_fields.iter())
        {
            match field.name.as_str() {
                EXPOSED_MODULES => {
                    *conditional_surface = true;
                    exposed_modules.extend(split_list(&field.value));
                }
                REEXPORTED_MODULES => {
                    *conditional_surface = true;
                    reexported_modules.extend(parse_reexported_modules(&field.value));
                }
                _ => {}
            }
        }
        collect_conditional_surface(
            &conditional.then_conditionals,
            exposed_modules,
            reexported_modules,
            conditional_surface,
        );
        collect_conditional_surface(
            &conditional.else_conditionals,
            exposed_modules,
            reexported_modules,
            conditional_surface,
        );
    }
}

fn has_conditionals(fields: &ast::ComponentFields<'_>) -> bool {
    !fields.conditionals.is_empty()
}

fn parse_reexported_modules(value: &str) -> Vec<String> {
    split_list(value)
        .into_iter()
        .filter_map(|entry| {
            // Cabal syntax: `pkg:Module`, `Module`, and optional ` as Alias`.
            let entry = entry.trim();
            let entry = entry.split_once(" as ").map_or(entry, |(module, _)| module);
            let module = entry.rsplit_once(':').map_or(entry, |(_, module)| module);
            let module = module.trim();
            if module.is_empty() {
                None
            } else {
                Some(module.to_string())
            }
        })
        .collect()
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn dedupe<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC: &str = "\
cabal-version: 2.2
name: demo
version: 1.2.3
x-revision: 2

library
  exposed-modules:
    Demo
    Demo.Internal
  reexported-modules:
    Data.Map as Map
  build-depends:
    base >=4.14 && <5,
    text ^>=2.0
";

    /// Check the basic package fields, exposed modules, re-exports, and
    /// canonical dependency ranges.
    #[test]
    fn parses_basic_library() {
        let parsed = parse_cabal(BASIC);
        let meta = parsed.meta.expect("metadata");
        assert_eq!(meta.name, "demo");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.revision, Some(2));
        assert_eq!(meta.libraries.len(), 1);
        let library = &meta.libraries[0];
        assert_eq!(library.exposed_modules, vec!["Demo", "Demo.Internal"]);
        assert_eq!(library.reexported_modules, vec!["Data.Map"]);
        assert_eq!(library.dependencies.len(), 2);
        assert_eq!(library.dependencies[0].package, "base");
        assert!(library.dependencies[0].range.contains(">=4.14"));
    }

    /// Check that a common stanza's dependencies are merged into the library
    /// that imports it.
    #[test]
    fn merges_common_stanza_dependencies() {
        let source = "\
cabal-version: 2.2
name: demo
version: 0.1.0

common deps
  build-depends:
    bytestring >=0.11

library
  import: deps
  exposed-modules: Demo
  build-depends: base >=4.14
";
        let parsed = parse_cabal(source);
        let library = &parsed.meta.expect("metadata").libraries[0];
        let names: Vec<&str> = library
            .dependencies
            .iter()
            .map(|dependency| dependency.package.as_str())
            .collect();
        assert!(names.contains(&"bytestring"));
        assert!(names.contains(&"base"));
    }

    /// Check that a conditional guarding exposed modules marks the surface as
    /// conditional and unions the branch modules.
    #[test]
    fn marks_conditional_module_surface() {
        let source = "\
cabal-version: 2.2
name: demo
version: 0.1.0

flag extra
  default: False

library
  exposed-modules: Demo
  if flag(extra)
    exposed-modules: Demo.Extra
";
        let parsed = parse_cabal(source);
        let meta = parsed.meta.expect("metadata");
        assert!(meta.conditional);
        let library = &meta.libraries[0];
        assert!(library.conditional_surface);
        assert!(library.exposed_modules.contains(&"Demo.Extra".to_string()));
    }

    /// Check that a file without a name is reported as unusable instead of
    /// producing a partial package identity.
    #[test]
    fn rejects_missing_name() {
        let parsed = parse_cabal("cabal-version: 2.2\nversion: 1.0\n");
        assert!(parsed.meta.is_none());
        assert!(!parsed.diagnostics.is_empty());
    }
}
