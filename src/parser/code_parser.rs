use super::{CodeChunk, SymbolKind};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::{Language, Parser, TreeCursor};

/// Code parser using tree-sitter for AST-based code chunking
pub struct CodeParser {
    languages: HashMap<String, Language>,
}

impl CodeParser {
    pub fn new() -> Self {
        let mut languages = HashMap::new();

        // Register languages (tree-sitter 0.20 uses language() function)
        languages.insert("rs".to_string(), tree_sitter_rust::language());
        languages.insert("ts".to_string(), tree_sitter_typescript::language_tsx());
        languages.insert("tsx".to_string(), tree_sitter_typescript::language_tsx());
        languages.insert("js".to_string(), tree_sitter_javascript::language());
        languages.insert("py".to_string(), tree_sitter_python::language());
        languages.insert("go".to_string(), tree_sitter_go::language());
        languages.insert("cpp".to_string(), tree_sitter_cpp::language());
        languages.insert("cc".to_string(), tree_sitter_cpp::language());
        languages.insert("java".to_string(), tree_sitter_java::language());
        languages.insert("cs".to_string(), tree_sitter_c_sharp::language());

        Self { languages }
    }

    /// Get file hash for change detection
    pub fn hash_file(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Parse code and extract chunks
    pub fn parse(&self, file_path: &Path, content: &str) -> Result<Vec<CodeChunk>> {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        // Check if we have a parser for this file type
        let language = match self.languages.get(&extension) {
            Some(lang) => lang,
            None => {
                // Fallback: treat entire file as one chunk
                return Ok(vec![CodeChunk {
                    file_path: file_path.to_string_lossy().to_string(),
                    content: content.to_string(),
                    start_line: 0,
                    end_line: content.lines().count(),
                    symbol_name: None,
                    symbol_kind: SymbolKind::Other,
                }]);
            }
        };

        let mut parser = Parser::new();
        parser
            .set_language(*language)
            .context("Failed to set language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse code")?;

        let mut chunks = Vec::new();
        let root = tree.root_node();

        self.extract_chunks(&mut chunks, &mut root.walk(), content, file_path);

        if chunks.is_empty() {
            // Fallback: entire file as one chunk
            chunks.push(CodeChunk {
                file_path: file_path.to_string_lossy().to_string(),
                content: content.to_string(),
                start_line: 0,
                end_line: content.lines().count(),
                symbol_name: None,
                symbol_kind: SymbolKind::Other,
            });
        }

        Ok(chunks)
    }

    fn extract_chunks(
        &self,
        chunks: &mut Vec<CodeChunk>,
        cursor: &mut TreeCursor,
        source: &str,
        file_path: &Path,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            // Extract based on node type
            if let Some(symbol_kind) = self.identify_symbol(kind) {
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();
                let content = &source[start_byte..end_byte];

                // Get symbol name
                let symbol_name = self.extract_symbol_name(cursor, source);

                chunks.push(CodeChunk {
                    file_path: file_path.to_string_lossy().to_string(),
                    content: content.to_string(),
                    start_line: node.start_position().row,
                    end_line: node.end_position().row,
                    symbol_name,
                    symbol_kind: symbol_kind.clone(),
                });

                // Don't recurse into this node, we've captured it
                if !cursor.goto_next_sibling() {
                    break;
                }
                continue;
            }

            // Recurse into children
            if cursor.goto_first_child() {
                self.extract_chunks(chunks, cursor, source, file_path);
                cursor.goto_parent();
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn identify_symbol(&self, node_kind: &str) -> Option<SymbolKind> {
        match node_kind {
            "function_definition"
            | "function_item"
            | "function_declaration"
            | "method_definition" => Some(SymbolKind::Function),
            "class_definition" | "class_declaration" | "impl_item" => Some(SymbolKind::Class),
            "method_declaration" | "method_item" => Some(SymbolKind::Method),
            "interface_declaration" => Some(SymbolKind::Interface),
            "struct_item" | "struct_declaration" => Some(SymbolKind::Struct),
            "module" => Some(SymbolKind::Module),
            _ => None,
        }
    }

    fn extract_symbol_name(&self, cursor: &TreeCursor, source: &str) -> Option<String> {
        let mut name_cursor = cursor.clone();

        // Try to find identifier child
        if name_cursor.goto_first_child() {
            loop {
                let node = name_cursor.node();
                if node.kind().contains("identifier") || node.kind().contains("name") {
                    let start = node.start_byte();
                    let end = node.end_byte();
                    return Some(source[start..end].to_string());
                }

                if !name_cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        None
    }
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_function() {
        let parser = CodeParser::new();
        let code = r#"
            fn hello_world() -> String {
                "Hello, world!".to_string()
            }
        "#;

        let chunks = parser.parse(Path::new("test.rs"), code).unwrap();
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].symbol_kind, SymbolKind::Function);
    }
}
