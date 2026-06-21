//! Syntax-aware code understanding via tree-sitter.
//!
//! This is the backbone of the OSS-model strategy: instead of dumping whole
//! files at a model with a narrow context window, Damascus slices source into
//! function/type-sized snippets, and instead of *building* every candidate it
//! first rejects the syntactically-broken ones with a parser — no LLM, no
//! sandbox, microseconds per check.

use std::path::Path;

use tree_sitter::{Node, Parser};

/// Languages Damascus can slice and syntax-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
}

impl Lang {
    /// Infer the language from a file extension. Unknown extensions return `None`
    /// (callers fall back to whole-file context, never crash).
    pub fn from_path(path: &Path) -> Option<Lang> {
        match path.extension().and_then(|e| e.to_str())? {
            "rs" => Some(Lang::Rust),
            "py" | "pyi" => Some(Lang::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
            "ts" | "tsx" => Some(Lang::TypeScript),
            "go" => Some(Lang::Go),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Go => "go",
        }
    }

    fn ts_language(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Node kinds that represent a named definition worth slicing.
    fn def_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &[
                "function_item",
                "struct_item",
                "enum_item",
                "union_item",
                "trait_item",
                "type_item",
                "const_item",
                "static_item",
                "macro_definition",
                "mod_item",
            ],
            Lang::Python => &["function_definition", "class_definition"],
            Lang::JavaScript => &[
                "function_declaration",
                "generator_function_declaration",
                "class_declaration",
                "method_definition",
            ],
            Lang::TypeScript => &[
                "function_declaration",
                "generator_function_declaration",
                "class_declaration",
                "abstract_class_declaration",
                "method_definition",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
            ],
            Lang::Go => &["function_declaration", "method_declaration", "type_spec"],
        }
    }
}

fn parser_for(lang: Lang) -> Option<Parser> {
    let mut p = Parser::new();
    p.set_language(&lang.ts_language()).ok()?;
    Some(p)
}

/// True if the source fails to parse cleanly (syntax error or missing node).
/// This is Stage 1 of the deterministic filter.
pub fn has_syntax_errors(lang: Lang, source: &str) -> bool {
    match parser_for(lang).and_then(|mut p| p.parse(source, None)) {
        Some(tree) => tree.root_node().has_error(),
        None => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Type,
    Other,
}

/// A named definition located in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 1-based inclusive line span.
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Declaration header (everything up to the body), trimmed.
    pub signature: String,
}

impl Symbol {
    pub fn line_count(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

/// Extract every named definition (functions, types, methods, …) from `source`.
pub fn symbols(lang: Lang, source: &str) -> Vec<Symbol> {
    let Some(tree) = parser_for(lang).and_then(|mut p| p.parse(source, None)) else {
        return Vec::new();
    };
    let kinds = lang.def_kinds();
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if kinds.contains(&node.kind()) {
            if let Some(sym) = to_symbol(node, source) {
                out.push(sym);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    out.sort_by_key(|s| s.start_byte);
    out
}

/// Find a symbol by name (first match in source order).
pub fn find_symbol<'a>(symbols: &'a [Symbol], name: &str) -> Option<&'a Symbol> {
    symbols.iter().find(|s| s.name == name)
}

fn to_symbol(node: Node, source: &str) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = source.get(name_node.byte_range())?.to_string();
    if name.trim().is_empty() {
        return None;
    }
    let body = node.child_by_field_name("body");
    let sig_end = body
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let signature = source
        .get(node.start_byte()..sig_end)
        .unwrap_or("")
        .trim()
        .to_string();
    let kind = match node.kind() {
        k if k.contains("function") || k == "method_definition" || k == "method_declaration" => {
            SymbolKind::Function
        }
        k if k.contains("struct")
            || k.contains("enum")
            || k.contains("type")
            || k.contains("class")
            || k.contains("interface")
            || k.contains("trait") =>
        {
            SymbolKind::Type
        }
        _ => SymbolKind::Other,
    };
    Some(Symbol {
        name,
        kind,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        signature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_languages() {
        assert_eq!(Lang::from_path(Path::new("a/b.rs")), Some(Lang::Rust));
        assert_eq!(Lang::from_path(Path::new("x.py")), Some(Lang::Python));
        assert_eq!(Lang::from_path(Path::new("x.tsx")), Some(Lang::TypeScript));
        assert_eq!(Lang::from_path(Path::new("x.go")), Some(Lang::Go));
        assert_eq!(Lang::from_path(Path::new("x.txt")), None);
    }

    #[test]
    fn flags_syntax_errors() {
        assert!(!has_syntax_errors(Lang::Rust, "fn a() -> i32 { 1 }"));
        assert!(has_syntax_errors(Lang::Rust, "fn a( -> i32 { "));
        assert!(!has_syntax_errors(Lang::Python, "def a():\n    return 1\n"));
        assert!(has_syntax_errors(Lang::Python, "def a(:\n    return"));
    }

    #[test]
    fn extracts_rust_symbols() {
        let src = "struct Point { x: i32 }\n\nfn dist(p: Point) -> i32 {\n    p.x\n}\n";
        let syms = symbols(Lang::Rust, src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"dist"));
        let dist = find_symbol(&syms, "dist").unwrap();
        assert_eq!(dist.kind, SymbolKind::Function);
        assert!(dist.signature.contains("fn dist(p: Point) -> i32"));
        assert!(!dist.signature.contains("p.x"));
    }

    #[test]
    fn extracts_python_methods() {
        let src = "class C:\n    def method(self, x):\n        return x\n";
        let syms = symbols(Lang::Python, src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"C"));
        assert!(names.contains(&"method"));
    }

    #[test]
    fn extracts_go_types_and_funcs() {
        let src = "package m\ntype Point struct { X int }\nfunc Dist(p Point) int { return p.X }\n";
        let syms = symbols(Lang::Go, src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"Dist"));
    }
}
