pub mod code_parser;

/// Code chunk representing a semantic unit (function, class, etc.)
#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub file_path: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol_name: Option<String>,
    pub symbol_kind: SymbolKind,
}

/// Type of code symbol
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Interface,
    Struct,
    Module,
    #[allow(dead_code)] // Reserved for future language support
    Variable,
    Other,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::Interface => "interface",
            SymbolKind::Struct => "struct",
            SymbolKind::Module => "module",
            SymbolKind::Variable => "variable",
            SymbolKind::Other => "other",
        }
    }
}
