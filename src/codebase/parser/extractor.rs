use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::types::symbol::{CodeReference, CodeSymbol};
use crate::types::Language;

use super::languages::{get_language_support, LanguageSupport};

pub struct Extractor {
    parser: Parser,
    language: Language,
    support: Box<dyn LanguageSupport>,
}

impl Extractor {
    pub fn new(language: Language) -> Option<Self> {
        let support = get_language_support(language.clone())?;
        let mut parser = Parser::new();
        if parser.set_language(&support.get_language()).is_err() {
            tracing::error!("Failed to load grammar for {:?}", language);
            return None;
        }

        Some(Self {
            parser,
            language,
            support,
        })
    }

    pub fn parse(
        &mut self,
        content: &str,
        file_path: &str,
        project_id: &str,
    ) -> (Vec<CodeSymbol>, Vec<CodeReference>) {
        let tree = match self.parser.parse(content, None) {
            Some(t) => t,
            None => return (vec![], vec![]),
        };

        let symbols = self.extract_symbols(&tree, content, file_path, project_id);
        let references = self.extract_references(&tree, content, file_path, &symbols);

        (symbols, references)
    }

    fn extract_symbols(
        &self,
        tree: &tree_sitter::Tree,
        content: &str,
        file_path: &str,
        project_id: &str,
    ) -> Vec<CodeSymbol> {
        let query_source = self.support.get_definition_query();
        let query = match Query::new(&self.support.get_language(), query_source) {
            Ok(q) => q,
            Err(e) => {
                tracing::error!("Invalid definition query for {:?}: {}", self.language, e);
                return vec![];
            }
        };

        let mut query_cursor = QueryCursor::new();
        let mut matches = query_cursor.matches(&query, tree.root_node(), content.as_bytes());

        let mut symbols = Vec::new();

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                if let Ok(name) = node.utf8_text(content.as_bytes()) {
                    let symbol_type = self.support.map_symbol_type(capture_name);
                    let name = self.support.extract_symbol_name(capture_name, name);

                    // node is the identifier, parent is the full definition (function_item, struct_item, etc.)
                    let definition_node = node.parent().unwrap_or(node);
                    let start_line = definition_node.start_position().row as u32 + 1;
                    let end_line = definition_node.end_position().row as u32 + 1;

                    let signature = self
                        .support
                        .extract_signature(&definition_node, content.as_bytes());

                    let mut symbol = CodeSymbol::new(
                        name,
                        symbol_type,
                        file_path.to_string(),
                        start_line,
                        end_line,
                        project_id.to_string(),
                    );

                    if let Some(sig) = signature {
                        symbol = symbol.with_signature(sig);
                    }

                    symbols.push(symbol);
                }
            }
        }

        symbols
    }

    fn extract_references(
        &self,
        tree: &tree_sitter::Tree,
        content: &str,
        file_path: &str,
        symbols: &[CodeSymbol],
    ) -> Vec<CodeReference> {
        let query_source = self.support.get_reference_query();
        let query = match Query::new(&self.support.get_language(), query_source) {
            Ok(q) => q,
            Err(e) => {
                tracing::error!("Invalid reference query for {:?}: {}", self.language, e);
                return vec![];
            }
        };

        let mut query_cursor = QueryCursor::new();
        let mut matches = query_cursor.matches(&query, tree.root_node(), content.as_bytes());

        let mut references = Vec::new();

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                if let Ok(raw_name) = node.utf8_text(content.as_bytes()) {
                    let start_line = node.start_position().row as u32 + 1;
                    let column = node.start_position().column as u32;

                    // Find which symbol contains this reference
                    let (from_symbol, from_symbol_line) = symbols
                        .iter()
                        .find(|s| start_line >= s.start_line && start_line <= s.end_line)
                        .map(|s| (s.name.clone(), s.start_line))
                        .unwrap_or_else(|| ("global".to_string(), 0));

                    let relation_type = self.support.map_relation_type(capture_name);
                    let name = self.support.extract_reference_name(capture_name, raw_name);

                    references.push(
                        CodeReference::builder()
                            .name(name.clone())
                            .from_symbol(from_symbol)
                            .from_symbol_line(from_symbol_line)
                            .to_symbol(name) // to_symbol is the same as name for now
                            .relation_type(relation_type)
                            .file_path(file_path.to_string())
                            .line(start_line)
                            .column(column)
                            .build(),
                    );
                }
            }
        }

        references
    }
}
