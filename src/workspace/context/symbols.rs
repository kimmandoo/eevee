//! Local, source-preserving Rust symbol and reference excerpt proposals.
//!
//! Line numbers are one-based and inclusive. Unsupported files and Rust parse
//! failures deliberately produce labelled text ranges, not inferred symbols.

use std::collections::BTreeSet;

use syn::{spanned::Spanned, visit::Visit};

const MAX_LINES: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolRange {
    pub start_line: usize,
    pub end_line: usize,
    pub label: String,
    pub identifiers: Vec<String>,
}

/// Propose bounded definitions/signatures and direct references to search terms.
///
/// Call this independently for each source file with the same terms to discover
/// both a definition and its callers. Terms are split into identifier tokens;
/// matching is exact and case-sensitive. Empty terms select all parser symbols.
pub fn ranges(path: &str, text: &str, terms: &[String]) -> Vec<SymbolRange> {
    let terms = tokens(terms.iter().map(String::as_str));
    if path.rsplit('.').next().is_none_or(|ext| ext != "rs") {
        return fallback(text, &terms, "unsupported syntax");
    }
    let file = match syn::parse_file(text) {
        Ok(file) => file,
        Err(_) => return fallback(text, &terms, "Rust parse failure"),
    };
    let mut collector = Collector {
        terms,
        line_count: text.lines().count().max(1),
        ranges: Vec::new(),
    };
    collector.visit_file(&file);
    collector.ranges.sort_by(|a, b| {
        (a.start_line, a.end_line, &a.label, &a.identifiers).cmp(&(
            b.start_line,
            b.end_line,
            &b.label,
            &b.identifiers,
        ))
    });
    collector.ranges.dedup();
    collector.ranges
}

fn tokens<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    values
        .into_iter()
        .flat_map(|value| value.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

struct Collector {
    terms: BTreeSet<String>,
    line_count: usize,
    ranges: Vec<SymbolRange>,
}

impl Collector {
    fn add(&mut self, span: proc_macro2::Span, label: String, identifiers: Vec<String>) {
        let identifiers: Vec<_> = identifiers
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !self.terms.is_empty() && !identifiers.iter().any(|id| self.terms.contains(id)) {
            return;
        }
        let reference = label.contains(" reference");
        let span_start = span.start().line.max(1).min(self.line_count);
        let span_end = span.end().line.max(span_start).min(self.line_count);
        let start_line = if reference {
            span_start.saturating_sub(3).max(1)
        } else {
            span_start
        };
        let actual_end = if reference {
            span_end.saturating_add(3).min(self.line_count)
        } else {
            span_end
        };
        let end_line = actual_end.min(start_line + MAX_LINES - 1);
        self.ranges.push(SymbolRange {
            start_line,
            end_line,
            label: if actual_end > end_line {
                format!("{label} (first {MAX_LINES} lines)")
            } else {
                label
            },
            identifiers,
        });
    }

    fn definition(&mut self, kind: &str, ident: &syn::Ident, span: proc_macro2::Span) {
        self.add(
            span,
            format!("Rust {kind} definition: {ident}"),
            vec![ident.to_string()],
        );
    }

    fn signature(&mut self, signature: &syn::Signature) {
        let mut names = SignatureNames::default();
        names.visit_signature(signature);
        self.add(
            signature.span(),
            format!("Rust function signature: {}", signature.ident),
            names.0,
        );
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        match item {
            syn::Item::Fn(item) => {
                self.definition("function", &item.sig.ident, item.span());
                self.signature(&item.sig);
            }
            syn::Item::Struct(item) => self.definition("struct", &item.ident, item.span()),
            syn::Item::Enum(item) => self.definition("enum", &item.ident, item.span()),
            syn::Item::Trait(item) => self.definition("trait", &item.ident, item.ident.span()),
            syn::Item::Type(item) => self.definition("type", &item.ident, item.span()),
            syn::Item::Const(item) => self.definition("const", &item.ident, item.span()),
            syn::Item::Static(item) => self.definition("static", &item.ident, item.span()),
            syn::Item::Union(item) => self.definition("union", &item.ident, item.span()),
            syn::Item::Mod(item) => self.definition("module", &item.ident, item.ident.span()),
            syn::Item::Macro(item) => {
                if let Some(ident) = &item.ident {
                    self.definition("macro", ident, item.span());
                }
            }
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.definition("function", &item.sig.ident, item.span());
        self.signature(&item.sig);
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if item.default.is_some() {
            self.definition("function", &item.sig.ident, item.span());
        }
        self.signature(&item.sig);
        syn::visit::visit_trait_item_fn(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let identifiers = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        self.add(path.span(), "Rust path reference".into(), identifiers);
        syn::visit::visit_path(self, path);
    }
    fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
        match tree {
            syn::UseTree::Path(path) => self.add(
                path.ident.span(),
                "Rust path reference".into(),
                vec![path.ident.to_string()],
            ),
            syn::UseTree::Name(name) => self.add(
                name.ident.span(),
                "Rust path reference".into(),
                vec![name.ident.to_string()],
            ),
            syn::UseTree::Rename(rename) => self.add(
                rename.span(),
                "Rust import reference".into(),
                vec![rename.ident.to_string(), rename.rename.to_string()],
            ),
            syn::UseTree::Glob(_) | syn::UseTree::Group(_) => {}
        }
        syn::visit::visit_use_tree(self, tree);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        // A syntactic reference only: no claim of semantic name resolution.
        self.add(
            call.method.span(),
            format!("Rust method reference: {}", call.method),
            vec![call.method.to_string()],
        );
        syn::visit::visit_expr_method_call(self, call);
    }
}

#[derive(Default)]
struct SignatureNames(Vec<String>);

impl<'ast> Visit<'ast> for SignatureNames {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.0.push(ident.to_string());
    }
}

fn fallback(text: &str, terms: &BTreeSet<String>, reason: &str) -> Vec<SymbolRange> {
    let lines: Vec<_> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut windows = BTreeSet::new();
    if terms.is_empty() {
        windows.insert((1, lines.len().min(MAX_LINES)));
    } else {
        for (index, line) in lines.iter().enumerate() {
            if tokens([*line]).iter().any(|token| terms.contains(token)) {
                let start = index.saturating_sub(MAX_LINES / 2);
                windows.insert((start + 1, (start + MAX_LINES).min(lines.len())));
            }
        }
    }
    windows
        .into_iter()
        .map(|(start_line, end_line)| SymbolRange {
            start_line,
            end_line,
            label: format!("Text-range fallback ({reason})"),
            identifiers: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definition_and_cross_file_callers_without_claiming_resolution() {
        let terms = vec!["render".into()];
        let definition = ranges(
            "src/render.rs",
            "pub fn render(value: &str) -> usize {\n    value.len()\n}\n",
            &terms,
        );
        assert!(
            definition
                .iter()
                .any(|range| range.start_line == 1
                    && range.label == "Rust function signature: render")
        );
        assert!(definition.iter().any(|range| range.start_line == 1
            && range.end_line == 3
            && range.label == "Rust function definition: render"));
        let callers = ranges(
            "src/main.rs",
            "use crate::render::render;\nfn main() {\n    render(\"hello\");\n    engine.render();\n}\n",
            &terms,
        );
        assert!(
            callers
                .iter()
                .any(|range| range.start_line == 1 && range.label == "Rust path reference")
        );
        assert!(callers.iter().any(|range| range.start_line <= 3
            && range.end_line >= 3
            && range.identifiers == ["render"]));
        assert!(callers.iter().any(|range| range.start_line <= 4
            && range.end_line >= 4
            && range.label == "Rust method reference: render"));
        assert_eq!(
            callers,
            ranges(
                "src/main.rs",
                "use crate::render::render;\nfn main() {\n    render(\"hello\");\n    engine.render();\n}\n",
                &terms
            )
        );
    }

    #[test]
    fn includes_impl_and_trait_default_bodies() {
        let source = "struct Engine;\nimpl Engine {\nfn render(&self) {\nlet body_evidence = 42;\n}\n}\ntrait Render {\nfn render(&self) {\nlet default_evidence = 7;\n}\n}\n";
        let proposed = ranges("engine.rs", source, &["render".into()]);
        assert!(
            proposed
                .iter()
                .any(|range| range.label == "Rust function definition: render"
                    && range.start_line == 3
                    && range.end_line == 5)
        );
        assert!(
            proposed
                .iter()
                .any(|range| range.label == "Rust function definition: render"
                    && range.start_line == 8
                    && range.end_line == 10)
        );
    }

    #[test]
    fn unsupported_and_unparseable_sources_are_only_text_fallbacks() {
        for (path, text, reason) in [
            (
                "script.py",
                "def render(value):\n    return value\n",
                "unsupported syntax",
            ),
            ("broken.rs", "fn render( {\n", "Rust parse failure"),
        ] {
            let proposed = ranges(path, text, &["render".into()]);
            assert!(!proposed.is_empty());
            assert!(proposed.iter().all(|range| range.label
                == format!("Text-range fallback ({reason})")
                && range.identifiers.is_empty()));
        }
    }

    #[test]
    fn bounds_large_definitions_and_fallback_windows() {
        let source = format!(
            "struct Large {{\n{}\n}}",
            (0..100)
                .map(|i| format!("field_{i}: usize,"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let proposed = ranges("large.rs", &source, &["Large".into()]);
        assert!(
            proposed
                .iter()
                .any(|range| range.label.contains("first 48 lines"))
        );
        assert!(
            proposed
                .iter()
                .all(|range| range.end_line - range.start_line < MAX_LINES)
        );
        let fallback = ranges("large.txt", &source, &["field_70".into()]);
        assert!(
            fallback
                .iter()
                .any(|range| range.start_line <= 72 && range.end_line >= 72)
        );
        assert!(
            fallback
                .iter()
                .all(|range| range.end_line - range.start_line < MAX_LINES)
        );
    }

    #[test]
    fn matches_identifiers_not_comments_or_substrings() {
        let source = "// render\nfn prerender() {}\nfn caller() { prerender(); }\n";
        assert!(ranges("source.rs", source, &["render".into()]).is_empty());
        assert!(ranges("empty.txt", "", &[]).is_empty());
    }
}
