// Copyright (C) 2026 xhdlphzr
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Syntax analysis via tree-sitter. Safe lifetime management — no `unsafe` transmute.
//! Nodes borrow from the [`SyntaxTree`] that owns the source.
//! Syntax highlighting is based on tree-sitter queries.

use crate::edit::Range;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use tree_sitter::StreamingIterator;
use tree_sitter::{Language as TsLanguage, Node, Parser, Point, Query, QueryCursor, Tree};

// Language enum and language mappings

/// Supported programming languages for parsing and syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    C,
    Cpp,
    Go,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Python,
}

impl Language {
    /// Returns the corresponding tree‑sitter language object.
    ///
    /// # Returns
    /// A `tree_sitter::Language` that can be used in a parser.
    pub fn ts_language(&self) -> TsLanguage {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// Returns the human‑readable language name.
    ///
    /// # Returns
    /// A static string slice, e.g. `"rust"`, `"python"`.
    pub fn name(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Go => "go",
            Language::Html => "html",
            Language::Css => "css",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Python => "python",
        }
    }

    /// Returns the tree‑sitter highlight query source for the language.
    fn highlight_query(&self) -> &'static str {
        // For simplicity we provide a common subset that works across languages.
        // In a production editor you would want per‑language optimised queries.
        // The following query covers most keywords and common syntactic elements.
        match self {
            Language::Rust => include_str!("queries/rust.scm"),
            Language::C | Language::Cpp => include_str!("queries/c.scm"),
            Language::Go => include_str!("queries/go.scm"),
            Language::Html => include_str!("queries/html.scm"),
            Language::Css => include_str!("queries/css.scm"),
            Language::JavaScript | Language::TypeScript => include_str!("queries/javascript.scm"),
            Language::Python => include_str!("queries/python.scm"),
        }
    }
}

// AST node wrapper

/// A reference to an AST node that borrows from a [`SyntaxTree`].
///
/// The underlying [`SyntaxTree`] must outlive this reference.
#[derive(Debug, Clone, Copy)]
pub struct SyntaxNode<'a> {
    pub inner: Node<'a>,
}

impl<'a> SyntaxNode<'a> {
    /// Returns the node's kind (e.g. `"function_item"`).
    pub fn kind(&self) -> &str {
        self.inner.kind()
    }

    /// Returns the byte range of this node.
    pub fn byte_range(&self) -> std::ops::Range<usize> {
        self.inner.byte_range()
    }

    /// Returns the start byte offset of the node.
    pub fn start_byte(&self) -> usize {
        self.inner.start_byte()
    }

    /// Returns the end byte offset of the node.
    pub fn end_byte(&self) -> usize {
        self.inner.end_byte()
    }

    /// Returns the start position (line, column) of the node.
    pub fn start_position(&self) -> Point {
        self.inner.start_position()
    }

    /// Returns the end position (line, column) of the node.
    pub fn end_position(&self) -> Point {
        self.inner.end_position()
    }

    /// Returns `true` if this node is an error node.
    pub fn is_error(&self) -> bool {
        self.inner.is_error()
    }

    /// Returns `true` if this node is a named node (as opposed to anonymous).
    pub fn is_named(&self) -> bool {
        self.inner.is_named()
    }

    /// Returns the number of direct children.
    pub fn child_count(&self) -> usize {
        self.inner.child_count()
    }

    /// Returns the source text covered by this node.
    ///
    /// # Arguments
    /// * `source` - The original source string (must be the same one used by the `SyntaxTree`).
    ///
    /// # Returns
    /// A string slice containing the node's text.
    pub fn text<'s>(&self, source: &'s str) -> &'s str {
        &source[self.inner.byte_range()]
    }

    /// Returns the parent node, if any.
    pub fn parent(&self) -> Option<SyntaxNode<'a>> {
        self.inner.parent().map(|n| SyntaxNode { inner: n })
    }

    /// Returns the child node with the given field name, if it exists.
    ///
    /// # Arguments
    /// * `name` - The field name (e.g. `"name"`, `"value"`).
    pub fn child_by_field_name(&self, name: &str) -> Option<SyntaxNode<'a>> {
        self.inner
            .child_by_field_name(name)
            .map(|n| SyntaxNode { inner: n })
    }

    /// Returns a vector of all direct children.
    pub fn children(&self) -> Vec<SyntaxNode<'a>> {
        let mut cursor = self.inner.walk();
        let mut out = Vec::new();
        for child in self.inner.children(&mut cursor) {
            out.push(SyntaxNode { inner: child });
        }
        out
    }

    /// Finds the deepest descendant node whose byte range fully overlaps or is contained in `[start, end)`.
    ///
    /// # Arguments
    /// * `start` - Start byte offset (inclusive).
    /// * `end` - End byte offset (exclusive).
    pub fn descendant_for_byte_range(&self, start: usize, end: usize) -> Option<SyntaxNode<'a>> {
        self.inner
            .descendant_for_byte_range(start, end)
            .map(|n| SyntaxNode { inner: n })
    }
}

// Symbols

/// A symbol (e.g. function, struct, variable) extracted from the AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub byte_range: std::ops::Range<usize>,
    pub line: usize,
    pub column: usize,
}

/// Categorisation of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    TypeAlias,
    Const,
    Static,
    Variable,
    Macro,
    Unknown,
}

impl SymbolKind {
    /// Infers the symbol kind from a tree‑sitter node kind string.
    ///
    /// # Arguments
    /// * `kind` - The node kind.
    ///
    /// # Returns
    /// A `SymbolKind` variant.
    pub fn from_node_kind(kind: &str) -> Self {
        match kind {
            "function_item" | "function_signature_item" => SymbolKind::Function,
            "struct_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "impl_item" => SymbolKind::Impl,
            "mod_item" => SymbolKind::Module,
            "type_item" => SymbolKind::TypeAlias,
            "const_item" => SymbolKind::Const,
            "static_item" => SymbolKind::Static,
            "let_declaration" => SymbolKind::Variable,
            "macro_definition" => SymbolKind::Macro,
            _ => SymbolKind::Unknown,
        }
    }
}

// Highlighting styles

/// Style information for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightStyle {
    pub fg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
}

impl HighlightStyle {
    /// Creates an empty (default) style.
    pub fn new() -> Self {
        Self {
            fg: None,
            bold: false,
            italic: false,
        }
    }

    /// Sets the foreground colour (RGB packed 0xRRGGBB).
    ///
    /// # Arguments
    /// * `c` - The colour value.
    ///
    /// # Returns
    /// The style with the colour set.
    pub fn fg(mut self, c: u32) -> Self {
        self.fg = Some(c);
        self
    }

    /// Sets the bold attribute.
    ///
    /// # Arguments
    /// * `b` - `true` for bold.
    ///
    /// # Returns
    /// The style with the bold flag set.
    pub fn bold(mut self, b: bool) -> Self {
        self.bold = b;
        self
    }

    /// Sets the italic attribute.
    ///
    /// # Arguments
    /// * `i` - `true` for italic.
    ///
    /// # Returns
    /// The style with the italic flag set.
    pub fn italic(mut self, i: bool) -> Self {
        self.italic = i;
        self
    }
}

impl Default for HighlightStyle {
    fn default() -> Self {
        Self::new()
    }
}

// Highlighter (multi‑language)

/// Highlighter using tree-sitter queries, specialised per language.
///
/// **Note**: This type does **not** implement `Serialize`/`Deserialize` because it contains
/// a tree-sitter query object and internal maps that are not meant to be serialized.
#[derive(Debug, Clone)]
pub struct Highlighter {
    query: Rc<Query>,
    capture_to_style: HashMap<String, HighlightStyle>,
}

impl Highlighter {
    /// Creates a highlighter for a specific language using default colours (VS Code Dark theme).
    ///
    /// # Arguments
    /// * `language` - The programming language.
    ///
    /// # Returns
    /// A `Highlighter` instance ready to highlight source code of that language.
    pub fn for_language(language: Language) -> Self {
        let query_source = language.highlight_query();
        let lang = language.ts_language();
        let query =
            Rc::new(Query::new(&lang, query_source).expect("Failed to compile highlight query"));

        let mut capture_to_style = HashMap::new();

        // Common styles (VS Code Dark theme colours)
        let keyword_style = HighlightStyle::new().fg(0x569CD6);
        let type_style = HighlightStyle::new().fg(0x4EC9B0);
        let function_style = HighlightStyle::new().fg(0xDCDCAA);
        let string_style = HighlightStyle::new().fg(0xCE9178);
        let number_style = HighlightStyle::new().fg(0xB5CEA8);
        let comment_style = HighlightStyle::new().fg(0x6A9955).italic(true);
        let variable_style = HighlightStyle::new().fg(0x9CDCFE);
        let macro_style = HighlightStyle::new().fg(0xC8C8C8);
        let attribute_style = HighlightStyle::new().fg(0xC8C8C8);

        // Map capture names to styles – these names come from the query files.
        capture_to_style.insert("function".to_string(), function_style);
        capture_to_style.insert("type".to_string(), type_style);
        capture_to_style.insert("variable".to_string(), variable_style);
        capture_to_style.insert("macro".to_string(), macro_style);
        capture_to_style.insert("string".to_string(), string_style);
        capture_to_style.insert("number".to_string(), number_style);
        capture_to_style.insert("comment".to_string(), comment_style);
        capture_to_style.insert("attribute".to_string(), attribute_style);
        // Keywords – many, all use same style
        for cap in &[
            "keyword", "fn", "let", "mut", "self", "super", "crate", "if", "else", "while", "for",
            "loop", "match", "return", "use", "mod", "pub", "struct", "enum", "trait", "impl",
            "where", "as", "in",
        ] {
            capture_to_style.insert(cap.to_string(), keyword_style);
        }

        Self {
            query,
            capture_to_style,
        }
    }

    /// Returns all highlight ranges for the given tree and source code.
    ///
    /// # Arguments
    /// * `tree` - The parsed tree.
    /// * `source` - The original source text.
    ///
    /// # Returns
    /// A vector of `(character_range, style)` pairs.
    pub fn get_highlights(&self, tree: &Tree, source: &str) -> Vec<(Range, HighlightStyle)> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        let mut out = Vec::new();
        while let Some(mat) = matches.next() {
            for capture in mat.captures {
                let node = capture.node;
                let cap_name = self.query.capture_names()[capture.index as usize].to_string();
                if let Some(style) = self.capture_to_style.get(&cap_name) {
                    let byte_range = node.byte_range();
                    let char_start = Self::byte_to_char(source, byte_range.start);
                    let char_end = Self::byte_to_char(source, byte_range.end);
                    out.push((Range::new(char_start, char_end), *style));
                }
            }
        }
        out
    }

    /// Convert byte offset to character offset (simple, used only in `get_highlights`).
    fn byte_to_char(text: &str, byte_offset: usize) -> usize {
        text[..byte_offset.min(text.len())].chars().count()
    }
}

// SyntaxTree – main structure

/// A tree‑sitter syntax tree that owns the source text and the parsed tree.
///
/// All `SyntaxNode` references produced from this object are tied to its lifetime.
///
/// **Note**: This type does **not** implement `Serialize`/`Deserialize` because it contains
/// a live parser and tree that cannot be meaningfully serialized.
pub struct SyntaxTree {
    #[allow(dead_code)]
    language: Language,
    parser: Parser,
    tree: Option<Tree>,
    highlighter: Highlighter,
    source: String,
    line_starts: Vec<usize>,
}

impl SyntaxTree {
    /// Creates a new syntax tree by parsing the provided source text.
    ///
    /// # Arguments
    /// * `language` - The programming language of the source.
    /// * `source_text` - The source code to parse.
    ///
    /// # Returns
    /// A `SyntaxTree` instance or an error if the parser cannot be initialised.
    pub fn new(language: Language, source_text: &str) -> anyhow::Result<Self> {
        let mut parser = Parser::new();
        parser.set_language(&language.ts_language())?;
        let source = source_text.to_string();
        let tree = parser.parse(&source, None);
        let line_starts = Self::build_line_starts(&source);
        let highlighter = Highlighter::for_language(language);
        Ok(Self {
            language,
            parser,
            tree,
            highlighter,
            source,
            line_starts,
        })
    }

    /// Updates the tree after a text change.
    ///
    /// # Arguments
    /// * `new_source` - The new source text.
    pub fn update(&mut self, new_source: &str) {
        self.source = new_source.to_string();
        self.line_starts = Self::build_line_starts(&self.source);
        self.tree = self.parser.parse(&self.source, self.tree.as_ref());
    }

    /// Builds a list of byte offsets for line starts.
    ///
    /// # Arguments
    /// * `s` - The source string.
    ///
    /// # Returns
    /// A `Vec<usize>` where each element is the byte offset of a line start.
    fn build_line_starts(s: &str) -> Vec<usize> {
        let mut v = vec![0];
        for (i, &b) in s.as_bytes().iter().enumerate() {
            if b == b'\n' {
                v.push(i + 1);
            }
        }
        v
    }

    /// Returns the root AST node, if present.
    ///
    /// # Returns
    /// An optional `SyntaxNode` referencing the root of the tree.
    pub fn get_ast(&self) -> Option<SyntaxNode<'_>> {
        self.tree.as_ref().map(|t| SyntaxNode {
            inner: t.root_node(),
        })
    }

    /// Finds the deepest node that contains the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - The character offset (Unicode scalar count).
    ///
    /// # Returns
    /// An optional `SyntaxNode` at that position.
    pub fn get_node_at_position(&self, char_offset: usize) -> Option<SyntaxNode<'_>> {
        let byte = self.char_to_byte(char_offset);
        let root = self.get_ast()?;
        root.descendant_for_byte_range(byte, byte)
    }

    /// Collects all symbols of a specific kind.
    ///
    /// # Arguments
    /// * `kind` - The symbol kind to filter.
    ///
    /// # Returns
    /// A vector of `Symbol`s.
    pub fn get_symbols(&self, kind: SymbolKind) -> Vec<Symbol> {
        let mut out = Vec::new();
        if let Some(root) = self.get_ast() {
            self.collect_symbols(&root, kind, &mut out);
        }
        out
    }

    /// Collects all symbols in the tree.
    ///
    /// # Returns
    /// A vector of all `Symbol`s found.
    pub fn get_all_symbols(&self) -> Vec<Symbol> {
        let mut out = Vec::new();
        if let Some(root) = self.get_ast() {
            self.collect_all(&root, &mut out);
        }
        out
    }

    /// Returns the nearest enclosing function symbol at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - The character offset.
    ///
    /// # Returns
    /// An optional `Symbol` representing the enclosing function.
    pub fn get_parent_function(&self, char_offset: usize) -> Option<Symbol> {
        let mut cur = self.get_node_at_position(char_offset)?;
        loop {
            if SymbolKind::from_node_kind(cur.kind()) == SymbolKind::Function {
                let name = cur
                    .child_by_field_name("name")
                    .map_or("<unknown>".into(), |n| n.text(&self.source).to_string());
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    byte_range: cur.byte_range(),
                    line: cur.start_position().row,
                    column: cur.start_position().column,
                });
            }
            cur = cur.parent()?;
        }
    }

    /// Computes highlight ranges for a given character range.
    ///
    /// # Arguments
    /// * `range` - The character range to highlight.
    ///
    /// # Returns
    /// A vector of `(character_range, style)` pairs.
    pub fn get_highlight_ranges(&self, range: &Range) -> Vec<(Range, HighlightStyle)> {
        if let Some(tree) = &self.tree {
            let mut all = self.highlighter.get_highlights(tree, &self.source);
            all.retain(|(r, _)| r.end > range.start && r.start < range.end);
            all
        } else {
            Vec::new()
        }
    }

    /// Suggests a rename edit for the symbol at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - The offset of the symbol or its identifier.
    /// * `new_name` - The new name to use.
    ///
    /// # Returns
    /// An optional `(range, new_text)` pair if a renameable symbol is found.
    pub fn rename_symbol(&self, char_offset: usize, new_name: &str) -> Option<(Range, String)> {
        let node = self.get_node_at_position(char_offset)?;
        let name_node = if node.kind() == "identifier" {
            node
        } else {
            node.child_by_field_name("name")?
        };
        let br = name_node.byte_range();
        Some((
            Range::new(self.byte_to_char(br.start), self.byte_to_char(br.end)),
            new_name.to_string(),
        ))
    }

    /// Returns the underlying source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Converts a character offset to a byte offset using the line cache.
    ///
    /// # Arguments
    /// * `char_offset` - The character offset (Unicode scalar count).
    ///
    /// # Returns
    /// The corresponding byte offset.
    pub fn char_to_byte(&self, char_offset: usize) -> usize {
        if char_offset == 0 {
            return 0;
        }
        let mut remaining = char_offset;
        for (line_idx, &line_byte_start) in self.line_starts.iter().enumerate() {
            let line_end = self
                .line_starts
                .get(line_idx + 1)
                .copied()
                .unwrap_or(self.source.len());
            let line_text = &self.source[line_byte_start..line_end];
            let line_chars = line_text.chars().count();
            if remaining < line_chars {
                return line_byte_start
                    + line_text
                        .char_indices()
                        .nth(remaining)
                        .map(|(i, _)| i)
                        .unwrap_or(line_text.len());
            }
            remaining -= line_chars;
        }
        self.source.len()
    }

    /// Converts a byte offset to a character offset using the line cache.
    ///
    /// # Arguments
    /// * `byte_offset` - The byte offset.
    ///
    /// # Returns
    /// The corresponding character offset (Unicode scalar count).
    pub fn byte_to_char(&self, byte_offset: usize) -> usize {
        let bo = byte_offset.min(self.source.len());
        let mut char_count = 0;
        for (line_idx, &line_byte_start) in self.line_starts.iter().enumerate() {
            let line_end = self
                .line_starts
                .get(line_idx + 1)
                .copied()
                .unwrap_or(self.source.len());
            if bo >= line_byte_start && bo <= line_end {
                return char_count + self.source[line_byte_start..bo].chars().count();
            }
            char_count += self.source[line_byte_start..line_end].chars().count();
        }
        char_count
    }

    /// Converts a character offset to an LSP position (line, UTF‑16 column).
    ///
    /// # Arguments
    /// * `char_offset` - The character offset.
    ///
    /// # Returns
    /// A tuple `(line, utf16_column)`.
    pub fn offset_to_position(&self, char_offset: usize) -> (usize, usize) {
        let co = char_offset.min(self.source.chars().count());
        let mut remaining = co;
        for (line_idx, &line_byte_start) in self.line_starts.iter().enumerate() {
            let line_end = self
                .line_starts
                .get(line_idx + 1)
                .copied()
                .unwrap_or(self.source.len());
            let line_text = &self.source[line_byte_start..line_end];
            let line_chars = line_text.chars().count();
            if remaining < line_chars {
                let byte_in_line = line_text
                    .char_indices()
                    .nth(remaining)
                    .map(|(i, _)| i)
                    .unwrap_or(line_text.len());
                let col = line_text[..byte_in_line].encode_utf16().count();
                return (line_idx, col);
            }
            remaining -= line_chars;
        }
        (self.line_starts.len().saturating_sub(1), 0)
    }

    // Symbol collection helpers

    fn collect_symbols(&self, node: &SyntaxNode, kind: SymbolKind, out: &mut Vec<Symbol>) {
        let nk = SymbolKind::from_node_kind(node.kind());
        if nk == kind && node.is_named() {
            if let Some(nn) = node.child_by_field_name("name") {
                out.push(Symbol {
                    name: nn.text(&self.source).into(),
                    kind: nk,
                    byte_range: node.byte_range(),
                    line: node.start_position().row,
                    column: node.start_position().column,
                });
            }
        }
        for ch in node.children() {
            self.collect_symbols(&ch, kind, out);
        }
    }

    fn collect_all(&self, node: &SyntaxNode, out: &mut Vec<Symbol>) {
        let nk = SymbolKind::from_node_kind(node.kind());
        if nk != SymbolKind::Unknown && node.is_named() {
            if let Some(nn) = node.child_by_field_name("name") {
                out.push(Symbol {
                    name: nn.text(&self.source).into(),
                    kind: nk,
                    byte_range: node.byte_range(),
                    line: node.start_position().row,
                    column: node.start_position().column,
                });
            }
        }
        for ch in node.children() {
            self.collect_all(&ch, out);
        }
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(s: &str) -> SyntaxTree {
        SyntaxTree::new(Language::Rust, s).unwrap()
    }

    #[test]
    fn test_ast() {
        assert!(mk("fn main() {}").get_ast().is_some());
    }

    #[test]
    fn test_symbols() {
        let t = mk("fn a() {}\nfn b() {}");
        assert_eq!(t.get_symbols(SymbolKind::Function).len(), 2);
    }

    #[test]
    fn test_offset_to_position() {
        let st = mk("abc\ndef");
        assert_eq!(st.offset_to_position(0), (0, 0));
        assert_eq!(st.offset_to_position(4), (1, 0));
    }

    #[test]
    fn test_char_byte_roundtrip() {
        let st = mk("héllo\nwörld");
        for i in 0..12 {
            let b = st.char_to_byte(i);
            let c = st.byte_to_char(b);
            assert_eq!(c, i, "roundtrip failed at {i}");
        }
    }

    #[test]
    fn test_highlight_ranges() {
        let st = mk("fn main() { let x = 42; }");
        let ranges = st.get_highlight_ranges(&Range::new(0, st.source().len()));
        assert!(!ranges.is_empty());
        assert!(ranges.iter().any(|(_, style)| style.fg.is_some()));
    }

    #[test]
    fn test_parent_function() {
        let st = mk("fn outer() { let x = 1; fn inner() {} }");
        let pos = st.source().find("x").unwrap();
        let parent = st.get_parent_function(pos);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().name, "outer");
        let inner_pos = st.source().find("inner()").unwrap() + 6;
        let inner_parent = st.get_parent_function(inner_pos);
        assert!(inner_parent.is_some());
        assert_eq!(inner_parent.unwrap().name, "inner");
    }

    #[test]
    fn test_rename_symbol() {
        let st = mk("fn old_name() {}");
        let pos = st.source().find("old_name").unwrap();
        let rename = st.rename_symbol(pos, "new_name");
        assert!(rename.is_some());
        let (range, new_text) = rename.unwrap();
        assert_eq!(range.start, pos);
        assert_eq!(range.end, pos + "old_name".len());
        assert_eq!(new_text, "new_name");
    }

    #[test]
    fn test_get_node_at_position() {
        let st = mk("fn main() { 42 }");
        let pos = st.source().find("42").unwrap();
        let node = st.get_node_at_position(pos);
        assert!(node.is_some());
        assert_eq!(node.unwrap().kind(), "integer_literal");
    }
}
