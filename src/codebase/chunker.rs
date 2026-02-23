use std::path::Path;

use crate::types::{ChunkType, CodeChunk, Language};

use super::parser::languages::get_language_support;
use super::scanner::detect_language;

const MAX_CHUNK_CHARS: usize = 4000;
const MIN_CHUNK_CHARS: usize = 10;
const MAX_CHUNK_LINES: usize = 150;

pub fn chunk_file(path: &Path, content: &str, project_id: &str) -> Vec<CodeChunk> {
    let language = detect_language(path);
    let file_path = path.to_string_lossy().to_string();

    if content.trim().is_empty() {
        return vec![];
    }

    if let Some(support) = get_language_support(language.clone()) {
        chunk_by_ast(
            content,
            &file_path,
            project_id,
            language,
            support.get_language(),
        )
    } else {
        chunk_by_structure(content, &file_path, project_id, language)
    }
}

fn chunk_by_ast(
    content: &str,
    file_path: &str,
    project_id: &str,
    language: Language,
    ts_language: tree_sitter::Language,
) -> Vec<CodeChunk> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return chunk_by_structure(content, file_path, project_id, language);
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return chunk_by_structure(content, file_path, project_id, language),
    };

    let mut chunks = Vec::new();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        let start_byte = child.start_byte();
        let end_byte = child.end_byte();

        let node_text = content.get(start_byte..end_byte).unwrap_or("");

        if node_text.len() < MIN_CHUNK_CHARS {
            continue;
        }

        if node_text.len() <= MAX_CHUNK_CHARS {
            chunks.push(create_chunk(
                node_text,
                file_path,
                project_id,
                language.clone(),
                child.start_position().row as u32 + 1,
                child.end_position().row as u32 + 1,
                detect_chunk_type(&child),
            ));
        } else {
            let sub_chunks = split_large_node(
                node_text,
                file_path,
                project_id,
                language.clone(),
                child.start_position().row as u32 + 1,
            );
            chunks.extend(sub_chunks);
        }
    }

    if chunks.is_empty() {
        return chunk_by_structure(content, file_path, project_id, language);
    }

    chunks
}

fn chunk_by_structure(
    content: &str,
    file_path: &str,
    project_id: &str,
    language: Language,
) -> Vec<CodeChunk> {
    let paragraphs: Vec<&str> = content.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut current_start_line: u32 = 1;
    let mut line_counter: u32 = 1;

    for para in paragraphs {
        let para_lines = para.lines().count() as u32;

        if current_chunk.len() + para.len() > MAX_CHUNK_CHARS && !current_chunk.is_empty() {
            let end_line = line_counter.saturating_sub(1);
            chunks.push(create_chunk(
                &current_chunk,
                file_path,
                project_id,
                language.clone(),
                current_start_line,
                end_line,
                ChunkType::Other,
            ));
            current_chunk.clear();
            current_start_line = line_counter;
        }

        if !current_chunk.is_empty() {
            current_chunk.push_str("\n\n");
        }
        current_chunk.push_str(para);
        line_counter += para_lines + 1;
    }

    if current_chunk.len() >= MIN_CHUNK_CHARS {
        chunks.push(create_chunk(
            &current_chunk,
            file_path,
            project_id,
            language,
            current_start_line,
            line_counter,
            ChunkType::Other,
        ));
    }

    chunks
}

fn split_large_node(
    text: &str,
    file_path: &str,
    project_id: &str,
    language: Language,
    base_line: u32,
) -> Vec<CodeChunk> {
    let lines: Vec<&str> = text.lines().collect();
    let mut chunks = Vec::new();
    let mut current_start = 0;

    while current_start < lines.len() {
        let end = (current_start + MAX_CHUNK_LINES).min(lines.len());
        let chunk_lines = &lines[current_start..end];
        let chunk_content = chunk_lines.join("\n");

        if chunk_content.len() >= MIN_CHUNK_CHARS {
            chunks.push(create_chunk(
                &chunk_content,
                file_path,
                project_id,
                language.clone(),
                base_line + current_start as u32,
                base_line + end as u32,
                ChunkType::Other,
            ));
        }

        current_start = end;
    }

    chunks
}

fn create_chunk(
    content: &str,
    file_path: &str,
    project_id: &str,
    language: Language,
    start_line: u32,
    end_line: u32,
    chunk_type: ChunkType,
) -> CodeChunk {
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

    CodeChunk {
        id: None,
        file_path: file_path.to_string(),
        content: content.to_string(),
        language,
        start_line,
        end_line,
        chunk_type,
        name: None,
        embedding: None,
        content_hash,
        project_id: Some(project_id.to_string()),
        indexed_at: crate::types::Datetime::default(),
    }
}

fn detect_chunk_type(node: &tree_sitter::Node) -> ChunkType {
    match node.kind() {
        "function_item" | "function_definition" | "function_declaration" | "method_definition" => {
            ChunkType::Function
        }
        "struct_item" | "class_definition" | "class_declaration" => ChunkType::Class,
        "impl_item" | "trait_item" | "interface_declaration" => ChunkType::Class,
        "mod_item" | "module" => ChunkType::Module,
        _ => ChunkType::Other,
    }
}
