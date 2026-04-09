use std::collections::HashSet;
use tree_sitter::{Language, Parser};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A semantic code chunk extracted by tree-sitter
#[derive(Debug, Clone)]
pub struct Chunk {
    pub content: String,
    pub file_path: String,
    pub language: String,
    pub node_kinds: Vec<String>,
    pub line_start: usize, // 1-based
    pub line_end: usize,   // 1-based
    pub parent_scope: String,
    pub defines: Vec<SymbolDef>,
    pub free_calls: Vec<String>,
    pub method_calls: Vec<String>,
    pub other_refs: Vec<String>,
    pub imports: Vec<RawImport>,
    pub type_refs: Vec<RawTypeRef>,
    pub field_defs: Vec<RawFieldDef>,
}

/// A symbol defined within a chunk
#[derive(Debug, Clone)]
pub struct SymbolDef {
    pub name: String,
    pub kind: String, // "function", "struct", "class", etc.
    pub line: usize,  // 1-based
    pub signature: String,
}

/// An import/use statement
#[derive(Debug, Clone)]
pub struct RawImport {
    pub name: String,
    pub path: String,
}

/// A type reference from a function signature (parameter or return)
#[derive(Debug, Clone)]
pub struct RawTypeRef {
    pub name: String,
    pub position: TypeRefPosition,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeRefPosition {
    Parameter,
    ReturnType,
}

/// A field declaration on a struct/class
#[derive(Debug, Clone)]
pub struct RawFieldDef {
    pub field_name: String,
    pub type_name: String,
}

// ---------------------------------------------------------------------------
// Language registry
// ---------------------------------------------------------------------------

struct LanguageConfig {
    ts_language: Language,
    primary_nodes: &'static [&'static str],
    container_nodes: &'static [&'static str],
    call_nodes: &'static [&'static str],
    member_access_nodes: &'static [&'static str],
    binding_nodes: &'static [&'static str],
}

fn language_config(name: &str) -> Option<LanguageConfig> {
    Some(match name {
        "rust" => LanguageConfig {
            ts_language: Language::new(tree_sitter_rust::LANGUAGE),
            primary_nodes: &[
                "function_item",
                "struct_item",
                "enum_item",
                "impl_item",
                "trait_item",
                "type_item",
                "const_item",
                "static_item",
                "mod_item",
                "macro_definition",
            ],
            container_nodes: &["impl_item"],
            call_nodes: &["call_expression"],
            member_access_nodes: &["field_expression"],
            binding_nodes: &["let_declaration", "parameter", "for_expression"],
        },
        "typescript" => LanguageConfig {
            ts_language: Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
            primary_nodes: &[
                "function_declaration",
                "class_declaration",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
                "export_statement",
            ],
            container_nodes: &["class_declaration"],
            call_nodes: &["call_expression"],
            member_access_nodes: &["member_expression"],
            binding_nodes: &["variable_declarator", "formal_parameters"],
        },
        "tsx" => LanguageConfig {
            ts_language: Language::new(tree_sitter_typescript::LANGUAGE_TSX),
            primary_nodes: &[
                "function_declaration",
                "class_declaration",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
                "export_statement",
            ],
            container_nodes: &["class_declaration"],
            call_nodes: &["call_expression"],
            member_access_nodes: &["member_expression"],
            binding_nodes: &["variable_declarator", "formal_parameters"],
        },
        "javascript" => LanguageConfig {
            ts_language: Language::new(tree_sitter_javascript::LANGUAGE),
            primary_nodes: &[
                "function_declaration",
                "class_declaration",
                "export_statement",
            ],
            container_nodes: &["class_declaration"],
            call_nodes: &["call_expression"],
            member_access_nodes: &["member_expression"],
            binding_nodes: &["variable_declarator", "formal_parameters"],
        },
        "python" => LanguageConfig {
            ts_language: Language::new(tree_sitter_python::LANGUAGE),
            primary_nodes: &[
                "function_definition",
                "class_definition",
                "decorated_definition",
            ],
            container_nodes: &["class_definition"],
            call_nodes: &["call"],
            member_access_nodes: &["attribute"],
            binding_nodes: &["assignment", "parameters"],
        },
        "go" => LanguageConfig {
            ts_language: Language::new(tree_sitter_go::LANGUAGE),
            primary_nodes: &[
                "function_declaration",
                "method_declaration",
                "type_declaration",
            ],
            container_nodes: &[],
            call_nodes: &["call_expression"],
            member_access_nodes: &["selector_expression"],
            binding_nodes: &["short_var_declaration", "parameter_declaration"],
        },
        "c" => LanguageConfig {
            ts_language: Language::new(tree_sitter_c::LANGUAGE),
            primary_nodes: &[
                "function_definition",
                "struct_specifier",
                "enum_specifier",
                "union_specifier",
                "type_definition",
                "declaration",
            ],
            container_nodes: &[],
            call_nodes: &["call_expression"],
            member_access_nodes: &["field_expression"],
            binding_nodes: &["declaration", "parameter_declaration"],
        },
        "cpp" => LanguageConfig {
            ts_language: Language::new(tree_sitter_cpp::LANGUAGE),
            primary_nodes: &[
                "function_definition",
                "class_specifier",
                "struct_specifier",
                "enum_specifier",
                "namespace_definition",
                "template_declaration",
            ],
            container_nodes: &["class_specifier"],
            call_nodes: &["call_expression"],
            member_access_nodes: &["field_expression", "member_expression"],
            binding_nodes: &["declaration", "parameter_declaration"],
        },
        "java" => LanguageConfig {
            ts_language: Language::new(tree_sitter_java::LANGUAGE),
            primary_nodes: &[
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
            ],
            container_nodes: &["class_declaration", "enum_declaration"],
            call_nodes: &["method_invocation", "object_creation_expression"],
            member_access_nodes: &["field_access"],
            binding_nodes: &["local_variable_declaration", "formal_parameter"],
        },
        "csharp" => LanguageConfig {
            ts_language: Language::new(tree_sitter_c_sharp::LANGUAGE),
            primary_nodes: &[
                "class_declaration",
                "struct_declaration",
                "interface_declaration",
                "enum_declaration",
                "method_declaration",
                "namespace_declaration",
                "record_declaration",
            ],
            container_nodes: &["class_declaration", "struct_declaration", "record_declaration"],
            call_nodes: &["invocation_expression"],
            member_access_nodes: &["member_access_expression"],
            binding_nodes: &["variable_declaration", "parameter"],
        },
        "markdown" => LanguageConfig {
            ts_language: Language::new(tree_sitter_md::LANGUAGE),
            primary_nodes: &[
                "section",
                "fenced_code_block",
                "list",
                "pipe_table",
                "html_block",
            ],
            container_nodes: &[],
            call_nodes: &[],
            member_access_nodes: &[],
            binding_nodes: &[],
        },
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

/// Detect language from file extension
pub fn detect_language(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some("rust"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some("cpp"),
        "java" => Some("java"),
        "cs" => Some("csharp"),
        // Markdown excluded — prose matches NL queries too well, crowding out code
        // "md" | "mdx" => Some("markdown"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Prepend a file-path header to chunk content so embedding models
/// can associate code with its location in the project.
fn with_file_context(content: String, file_path: &str, line_start: usize, line_end: usize) -> String {
    format!("// {file_path}:{line_start}-{line_end}\n{content}")
}

/// Parse a source file and extract semantic chunks
pub fn chunk_file(
    source: &str,
    file_path: &str,
    language: &str,
    merge_threshold: usize,
    split_threshold: usize,
    custom_blocklist: Option<&[String]>,
) -> Vec<Chunk> {
    let config = match language_config(language) {
        Some(c) => c,
        None => return vec![fallback_chunk(source, file_path, language)],
    };

    let blocklist: HashSet<&str> = match custom_blocklist {
        Some(custom) => custom.iter().map(|s| s.as_str()).collect(),
        None => default_blocklist(),
    };

    let mut parser = Parser::new();
    if parser.set_language(&config.ts_language).is_err() {
        return vec![fallback_chunk(source, file_path, language)];
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![fallback_chunk(source, file_path, language)],
    };

    let source_bytes = source.as_bytes();
    let root = tree.root_node();
    let mut raw_chunks: Vec<Chunk> = Vec::new();

    // Walk top-level children and extract primary chunks
    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            let node = cursor.node();
            if config.primary_nodes.contains(&node.kind()) {
                // Include preceding doc comments / decorators / attributes
                let doc_start = find_doc_comment_start(&node, source_bytes, language);
                let start = doc_start.unwrap_or_else(|| node.start_position().row + 1);
                let end = node.end_position().row + 1;

                // Build text including doc comments
                let text = if doc_start.is_some() {
                    let lines: Vec<&str> = source.lines().collect();
                    lines[(start - 1)..end].join("\n")
                } else {
                    node.utf8_text(source_bytes).unwrap_or("").to_string()
                };

                let line_count = end - start + 1;

                if line_count > split_threshold {
                    let children =
                        split_large_node(&node, source_bytes, file_path, language, &config, &blocklist);
                    raw_chunks.extend(children);
                } else {
                    let is_container = config.container_nodes.contains(&node.kind());
                    let mut defines = extract_defines(&node, source_bytes, language);
                    let mut parent_scope = String::new();

                    if is_container {
                        if let Some(container_def) = defines.first() {
                            parent_scope = container_def.name.clone();
                        }
                        collect_child_defines(&node, source_bytes, language, config.primary_nodes, &mut defines);
                    }

                    let defined_names: Vec<String> = defines.iter().map(|d| d.name.clone()).collect();
                    let defined_set: HashSet<&str> = defined_names.iter().map(|s| s.as_str()).collect();
                    let classified = classify_identifiers(&node, source_bytes, &defined_set, &config, &blocklist);
                    let type_refs = extract_type_refs(&node, source_bytes);
                    let field_defs = extract_field_defs(&node, source_bytes);

                    raw_chunks.push(Chunk {
                        content: with_file_context(text, file_path, start, end),
                        file_path: file_path.to_string(),
                        language: language.to_string(),
                        node_kinds: vec![node.kind().to_string()],
                        line_start: start,
                        line_end: end,
                        parent_scope,
                        defines,
                        free_calls: classified.free_calls,
                        method_calls: classified.method_calls,
                        other_refs: classified.other_refs,
                        imports: vec![],
                        type_refs,
                        field_defs,
                    });
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // Attach file-level imports to the first chunk
    if !raw_chunks.is_empty() {
        let file_imports = extract_file_imports(source, file_path, language);
        raw_chunks[0].imports.extend(file_imports);
    }

    merge_small_chunks(raw_chunks, merge_threshold)
}

// ---------------------------------------------------------------------------
// Doc comment / decorator detection
// ---------------------------------------------------------------------------

/// Walk backward from a node to find preceding doc comments, decorators, or attributes
fn find_doc_comment_start(
    node: &tree_sitter::Node,
    source: &[u8],
    language: &str,
) -> Option<usize> {
    let mut prev = node.prev_sibling();
    let mut earliest_comment_row = None;

    while let Some(sibling) = prev {
        match language {
            "rust" => {
                if sibling.kind() == "line_comment" {
                    let text = sibling.utf8_text(source).unwrap_or("");
                    if text.starts_with("///") || text.starts_with("//!") {
                        earliest_comment_row = Some(sibling.start_position().row + 1);
                        prev = sibling.prev_sibling();
                        continue;
                    }
                }
                if sibling.kind() == "attribute_item" {
                    earliest_comment_row = Some(sibling.start_position().row + 1);
                    prev = sibling.prev_sibling();
                    continue;
                }
            }
            "python" => {
                // Python decorators are handled as decorated_definition primary nodes
                break;
            }
            "java" | "csharp" => {
                // Javadoc / XML doc comments
                if sibling.kind() == "block_comment" || sibling.kind() == "comment" {
                    let text = sibling.utf8_text(source).unwrap_or("");
                    if text.starts_with("/**") || text.starts_with("///") {
                        earliest_comment_row = Some(sibling.start_position().row + 1);
                        prev = sibling.prev_sibling();
                        continue;
                    }
                }
                // Annotations / attributes
                if sibling.kind() == "marker_annotation"
                    || sibling.kind() == "annotation"
                    || sibling.kind() == "attribute_list"
                {
                    earliest_comment_row = Some(sibling.start_position().row + 1);
                    prev = sibling.prev_sibling();
                    continue;
                }
            }
            "typescript" | "tsx" | "javascript" => {
                // JSDoc comments
                if sibling.kind() == "comment" {
                    let text = sibling.utf8_text(source).unwrap_or("");
                    if text.starts_with("/**") {
                        earliest_comment_row = Some(sibling.start_position().row + 1);
                        prev = sibling.prev_sibling();
                        continue;
                    }
                }
                // Decorators
                if sibling.kind() == "decorator" {
                    earliest_comment_row = Some(sibling.start_position().row + 1);
                    prev = sibling.prev_sibling();
                    continue;
                }
            }
            _ => {
                // Generic: walk back through comment siblings
                if sibling.kind() == "comment" || sibling.kind() == "line_comment" {
                    earliest_comment_row = Some(sibling.start_position().row + 1);
                    prev = sibling.prev_sibling();
                    continue;
                }
            }
        }
        break;
    }

    earliest_comment_row
}

// ---------------------------------------------------------------------------
// Node splitting
// ---------------------------------------------------------------------------

fn split_large_node(
    node: &tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    language: &str,
    config: &LanguageConfig,
    blocklist: &HashSet<&str>,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let parent_kind = node.kind().to_string();

    let container_name = if config.container_nodes.contains(&node.kind()) {
        extract_symbol_name(node, source, language)
    } else {
        None
    };

    let parent_scope = container_name.clone().unwrap_or_default();

    let parent_defines = extract_defines(node, source, language);

    collect_primary_children(
        node,
        source,
        file_path,
        language,
        config,
        &parent_scope,
        blocklist,
        &mut chunks,
    );

    // Always include the parent node's defines (e.g., the function name when splitting
    // a large function that contains inner structs/enums)
    if !chunks.is_empty() && !parent_defines.is_empty() {
        chunks[0].defines.splice(0..0, parent_defines);
    } else if chunks.is_empty() {
        let start = node.start_position().row + 1;
        let end = node.end_position().row + 1;
        let text = node.utf8_text(source).unwrap_or("").to_string();

        let defines = extract_defines(node, source, language);
        let defined_names: Vec<String> = defines.iter().map(|d| d.name.clone()).collect();
        let defined_set: HashSet<&str> = defined_names.iter().map(|s| s.as_str()).collect();
        let classified = classify_identifiers(node, source, &defined_set, config, blocklist);

        chunks.push(Chunk {
            content: with_file_context(text, file_path, start, end),
            file_path: file_path.to_string(),
            language: language.to_string(),
            node_kinds: vec![parent_kind],
            line_start: start,
            line_end: end,
            parent_scope: String::new(),
            defines,
            free_calls: classified.free_calls,
            method_calls: classified.method_calls,
            other_refs: classified.other_refs,
            imports: vec![],
            type_refs: vec![],
            field_defs: vec![],
        });
    }

    chunks
}

fn collect_primary_children(
    node: &tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    language: &str,
    config: &LanguageConfig,
    parent_scope: &str,
    blocklist: &HashSet<&str>,
    chunks: &mut Vec<Chunk>,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if config.primary_nodes.contains(&child.kind()) {
                let start = child.start_position().row + 1;
                let end = child.end_position().row + 1;
                let text = child.utf8_text(source).unwrap_or("").to_string();
                let defines = extract_defines(&child, source, language);
                let defined_names: Vec<String> = defines.iter().map(|d| d.name.clone()).collect();
                let defined_set: HashSet<&str> = defined_names.iter().map(|s| s.as_str()).collect();
                let classified = classify_identifiers(&child, source, &defined_set, config, blocklist);
                let type_refs = extract_type_refs(&child, source);
                let field_defs = extract_field_defs(&child, source);
                chunks.push(Chunk {
                    content: with_file_context(text, file_path, start, end),
                    file_path: file_path.to_string(),
                    language: language.to_string(),
                    node_kinds: vec![child.kind().to_string()],
                    line_start: start,
                    line_end: end,
                    parent_scope: parent_scope.to_string(),
                    defines,
                    free_calls: classified.free_calls,
                    method_calls: classified.method_calls,
                    other_refs: classified.other_refs,
                    imports: vec![],
                    type_refs,
                    field_defs,
                });
            } else {
                collect_primary_children(
                    &child,
                    source,
                    file_path,
                    language,
                    config,
                    parent_scope,
                    blocklist,
                    chunks,
                );
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Chunk merging
// ---------------------------------------------------------------------------

fn merge_small_chunks(chunks: Vec<Chunk>, merge_threshold: usize) -> Vec<Chunk> {
    if chunks.is_empty() {
        return chunks;
    }

    let mut merged: Vec<Chunk> = Vec::new();
    let mut pending: Option<Chunk> = None;

    for chunk in chunks {
        let line_count = chunk.line_end - chunk.line_start + 1;

        match pending.take() {
            None => {
                if line_count < merge_threshold {
                    pending = Some(chunk);
                } else {
                    merged.push(chunk);
                }
            }
            Some(mut p) => {
                let p_lines = p.line_end - p.line_start + 1;
                if p_lines < merge_threshold && line_count < merge_threshold {
                    p.content.push_str("\n\n");
                    p.content.push_str(&chunk.content);
                    p.line_end = chunk.line_end;
                    p.node_kinds.extend(chunk.node_kinds);
                    p.defines.extend(chunk.defines);
                    p.imports.extend(chunk.imports);
                    p.type_refs.extend(chunk.type_refs);
                    p.field_defs.extend(chunk.field_defs);
                    // Merge other_refs, dedup
                    let mut ref_set: HashSet<String> = p.other_refs.drain(..).collect();
                    ref_set.extend(chunk.other_refs);
                    // Remove defined names from other_refs
                    for d in &p.defines {
                        ref_set.remove(&d.name);
                    }
                    p.other_refs = ref_set.into_iter().collect();
                    p.other_refs.sort();
                    // Merge free_calls and method_calls
                    p.free_calls.extend(chunk.free_calls);
                    p.method_calls.extend(chunk.method_calls);
                    pending = Some(p);
                } else {
                    merged.push(p);
                    if line_count < merge_threshold {
                        pending = Some(chunk);
                    } else {
                        merged.push(chunk);
                    }
                }
            }
        }
    }

    if let Some(p) = pending {
        merged.push(p);
    }

    merged
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------

/// Recursively walk descendants of a node, extracting defines from any primary nodes found.
/// Needed because in many grammars methods are not direct children of the container
/// (e.g., Rust: impl_item -> declaration_list -> function_item).
fn collect_child_defines(
    node: &tree_sitter::Node,
    source: &[u8],
    language: &str,
    primary_nodes: &[&str],
    defines: &mut Vec<SymbolDef>,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if primary_nodes.contains(&child.kind()) {
                let child_defines = extract_defines(&child, source, language);
                defines.extend(child_defines);
            } else {
                collect_child_defines(&child, source, language, primary_nodes, defines);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract symbol definitions from a primary AST node
fn extract_defines(
    node: &tree_sitter::Node,
    source: &[u8],
    language: &str,
) -> Vec<SymbolDef> {
    let mut defines = Vec::new();

    let name = extract_symbol_name(node, source, language);
    if let Some(name) = name {
        defines.push(SymbolDef {
            name,
            kind: normalize_kind(node.kind()),
            line: node.start_position().row + 1,
            signature: extract_signature(node, source),
        });
    }

    defines
}

/// Extract the symbol name from an AST node
fn extract_symbol_name(
    node: &tree_sitter::Node,
    source: &[u8],
    language: &str,
) -> Option<String> {
    // C/C++ function definitions: name is buried in the declarator chain
    if matches!(language, "c" | "cpp") && node.kind() == "function_definition" {
        return extract_name_from_declarator(node, source);
    }

    // Rust impl_item: use the "type" field as the name (e.g., "Store" in "impl Display for Store")
    if language == "rust" && node.kind() == "impl_item" {
        return node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
    }

    // Python decorated_definition: extract name from the inner definition
    if language == "python" && node.kind() == "decorated_definition" {
        // The actual definition is the last child (function_definition or class_definition)
        let child_count = node.child_count();
        if child_count > 0 {
            let inner = node.child(child_count as u32 - 1)?;
            return inner
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
        }
        return None;
    }

    // Default: use the "name" field
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.to_string())
}

/// Walk the declarator chain in C/C++ to find the function name
fn extract_name_from_declarator(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.child_by_field_name("declarator")?;
    // Walk down through pointer_declarator, function_declarator, etc.
    loop {
        if current.kind() == "identifier" || current.kind() == "field_identifier" {
            return current.utf8_text(source).ok().map(|s| s.to_string());
        }
        // Try "declarator" field first (function_declarator, pointer_declarator)
        if let Some(child) = current.child_by_field_name("declarator") {
            current = child;
            continue;
        }
        // Try "name" field (qualified_identifier in C++)
        if let Some(child) = current.child_by_field_name("name") {
            current = child;
            continue;
        }
        // Try first named child as fallback
        if let Some(child) = current.named_child(0) {
            current = child;
            continue;
        }
        return None;
    }
}

/// Extract the first line of a node as its signature, truncated before '{'
fn extract_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    let first_line = text.lines().next().unwrap_or("").trim();
    let sig = if let Some(brace_pos) = first_line.find('{') {
        first_line[..brace_pos].trim()
    } else {
        first_line
    };
    if sig.len() > 120 {
        format!("{}...", &sig[..117])
    } else {
        sig.to_string()
    }
}

/// Normalize tree-sitter node kind to a human-readable symbol kind
fn normalize_kind(kind: &str) -> String {
    match kind {
        // Rust
        "function_item" => "function",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "impl_item" => "impl",
        "trait_item" => "trait",
        "type_item" => "type",
        "const_item" => "const",
        "static_item" => "static",
        "mod_item" => "module",
        "macro_definition" => "macro",
        // Python
        "function_definition" => "function",
        "class_definition" => "class",
        "decorated_definition" => "decorated",
        // JS/TS
        "function_declaration" => "function",
        "class_declaration" => "class",
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type",
        "enum_declaration" => "enum",
        "export_statement" => "export",
        // Go
        "method_declaration" => "method",
        "type_declaration" => "type",
        // C
        "struct_specifier" | "struct_declaration" => "struct",
        "enum_specifier" => "enum",
        "union_specifier" => "union",
        "type_definition" => "typedef",
        "declaration" => "declaration",
        // C++
        "class_specifier" => "class",
        "namespace_definition" | "namespace_declaration" => "namespace",
        "template_declaration" => "template",
        // C#
        "record_declaration" => "record",
        // Fallback
        other => other,
    }
    .to_string()
}

/// Result of classifying identifiers by syntactic context
struct ClassifiedRefs {
    free_calls: Vec<String>,
    method_calls: Vec<String>,
    other_refs: Vec<String>,
}

/// Classify identifiers within an AST node into free calls, method calls, and other refs.
///
/// Uses tree-sitter parent/grandparent context from the LanguageConfig to sort
/// identifiers into the three buckets. Bindings (let, for, parameter) are detected
/// and excluded from all buckets.
fn classify_identifiers(
    node: &tree_sitter::Node,
    source: &[u8],
    defined_names: &HashSet<&str>,
    config: &LanguageConfig,
    blocklist: &HashSet<&str>,
) -> ClassifiedRefs {
    // First pass: collect bindings (local variable names to exclude)
    let mut bindings: HashSet<String> = HashSet::new();
    collect_bindings(node, source, config, &mut bindings);

    // Merge bindings into the exclusion set
    let mut excluded: HashSet<&str> = defined_names.clone();
    // We need the bindings to live long enough — collect refs into strings
    let binding_refs: Vec<&str> = bindings.iter().map(|s| s.as_str()).collect();
    for b in &binding_refs {
        excluded.insert(b);
    }

    // Second pass: classify identifiers
    let mut free_calls: HashSet<String> = HashSet::new();
    let mut method_calls: HashSet<String> = HashSet::new();
    let mut other_refs: HashSet<String> = HashSet::new();

    walk_and_classify(
        node,
        source,
        &excluded,
        config,
        blocklist,
        &mut free_calls,
        &mut method_calls,
        &mut other_refs,
    );

    // Sort and cap each bucket independently at 50
    let mut free_calls: Vec<String> = free_calls.into_iter().collect();
    free_calls.sort();
    free_calls.truncate(50);

    let mut method_calls: Vec<String> = method_calls.into_iter().collect();
    method_calls.sort();
    method_calls.truncate(50);

    let mut other_refs: Vec<String> = other_refs.into_iter().collect();
    other_refs.sort();
    other_refs.truncate(50);

    ClassifiedRefs {
        free_calls,
        method_calls,
        other_refs,
    }
}

/// Walk the AST collecting binding names (let declarations, parameters, for variables)
fn collect_bindings(
    node: &tree_sitter::Node,
    source: &[u8],
    config: &LanguageConfig,
    bindings: &mut HashSet<String>,
) {
    if config.binding_nodes.contains(&node.kind()) {
        // Find the first identifier child — that's the binding name
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "identifier" {
                    if let Ok(text) = child.utf8_text(source) {
                        let text = text.trim();
                        if !text.is_empty() {
                            bindings.insert(text.to_string());
                        }
                    }
                    break; // Only take the first identifier child
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_bindings(&cursor.node(), source, config, bindings);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk the AST and classify each identifier/type_identifier by its parent context
#[allow(clippy::too_many_arguments)]
fn walk_and_classify(
    node: &tree_sitter::Node,
    source: &[u8],
    excluded: &HashSet<&str>,
    config: &LanguageConfig,
    blocklist: &HashSet<&str>,
    free_calls: &mut HashSet<String>,
    method_calls: &mut HashSet<String>,
    other_refs: &mut HashSet<String>,
) {
    let kind = node.kind();

    // Handle field_identifier: only promote to method_calls when it's a method call,
    // otherwise drop (struct field names, field accesses)
    if kind == "field_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let text = text.trim();
            if text.len() < 3 || excluded.contains(text) || is_common_keyword(text) || blocklist.contains(text) {
                return;
            }
            // field_identifier is always inside a member_access node.
            // Check if the grandparent is a call node → method call
            if let Some(parent) = node.parent() {
                if config.member_access_nodes.contains(&parent.kind()) {
                    if let Some(grandparent) = parent.parent() {
                        if config.call_nodes.contains(&grandparent.kind()) {
                            method_calls.insert(text.to_string());
                        }
                    }
                }
            }
        }
        return;
    }

    if kind == "identifier" || kind == "type_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let text = text.trim();

            // Skip short identifiers, excluded names, keywords, and blocklisted names
            if text.len() < 3 || excluded.contains(text) || is_common_keyword(text) || blocklist.contains(text) {
                return;
            }

            // Classify based on parent node context
            if let Some(parent) = node.parent() {
                let parent_kind = parent.kind();

                if config.member_access_nodes.contains(&parent_kind) {
                    // This identifier is inside a member access (e.g., obj.foo)
                    // Check if this is the last named child (the member name)
                    let is_member_name = is_last_named_child(&parent, node);

                    if is_member_name {
                        // This is the member being accessed (e.g., "foo" in obj.foo)
                        if let Some(grandparent) = parent.parent() {
                            if config.call_nodes.contains(&grandparent.kind()) {
                                // obj.foo() — method call
                                method_calls.insert(text.to_string());
                            }
                            // else: obj.foo (field access) — drop entirely
                        }
                    } else {
                        // This is the receiver (e.g., "obj" in obj.foo) — other_refs
                        other_refs.insert(text.to_string());
                    }
                } else if config.call_nodes.contains(&parent_kind) {
                    // Direct child of a call node
                    // Check if this is the function being called (first named child)
                    let is_function_name = is_first_named_child(&parent, node);
                    if is_function_name {
                        free_calls.insert(text.to_string());
                    } else {
                        other_refs.insert(text.to_string());
                    }
                } else {
                    // Everything else
                    other_refs.insert(text.to_string());
                }
            } else {
                other_refs.insert(text.to_string());
            }
        }
        return; // Leaf nodes
    }

    // Recurse into children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_and_classify(
                &cursor.node(),
                source,
                excluded,
                config,
                blocklist,
                free_calls,
                method_calls,
                other_refs,
            );
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Check if `child` is the last named child of `parent`
fn is_last_named_child(parent: &tree_sitter::Node, child: &tree_sitter::Node) -> bool {
    let count = parent.named_child_count();
    if count == 0 {
        return false;
    }
    if let Some(last) = parent.named_child(count as u32 - 1) {
        last.id() == child.id()
    } else {
        false
    }
}

/// Check if `child` is the first named child of `parent`
fn is_first_named_child(parent: &tree_sitter::Node, child: &tree_sitter::Node) -> bool {
    if let Some(first) = parent.named_child(0) {
        first.id() == child.id()
    } else {
        false
    }
}

/// Common method/function names to filter out of classification buckets
fn default_blocklist() -> HashSet<&'static str> {
    [
        "clone", "collect", "iter", "into_iter", "map", "filter", "flat_map",
        "unwrap", "expect", "unwrap_or", "unwrap_or_else", "unwrap_or_default",
        "get", "set", "push", "pop", "insert", "remove", "contains", "len", "is_empty",
        "into", "from", "new", "default", "to_string", "to_owned", "as_ref", "as_mut",
        "fmt", "write", "read", "flush", "close", "lock", "try_lock",
        "some", "none", "and_then", "or_else", "map_err",
        "next", "peek", "take", "skip", "zip", "enumerate", "fold", "reduce",
        "append", "extend", "clear", "drain", "retain", "truncate", "reserve",
        "sort", "sort_by", "sort_by_key", "reverse", "dedup",
        "cmp", "partial_cmp", "hash",
        "display", "debug", "serialize", "deserialize",
        "begin", "end", "first", "last", "keys", "values", "entries",
        "toString", "valueOf", "hasOwnProperty", "indexOf", "forEach", "splice",
        "charAt", "substring", "replace", "split", "trim", "join", "concat",
        "print", "println", "printf", "sprintf", "format",
    ].iter().copied().collect()
}

/// Filter out common language keywords that tree-sitter may expose as identifiers
fn is_common_keyword(text: &str) -> bool {
    matches!(
        text,
        "self" | "Self" | "super" | "crate"
            | "true" | "false" | "None" | "null" | "undefined"
            | "if" | "else" | "for" | "while" | "return" | "break" | "continue"
            | "fn" | "let" | "mut" | "pub" | "use" | "mod" | "impl" | "struct" | "enum"
            | "const" | "static" | "type" | "trait" | "where" | "async" | "await"
            | "def" | "class" | "import" | "from" | "as" | "in" | "is" | "not" | "and" | "or"
            | "var" | "function" | "new" | "this" | "typeof" | "instanceof" | "export" | "default"
            | "void" | "int" | "bool" | "string" | "float" | "double" | "char" | "byte"
            | "package" | "func" | "interface" | "map" | "range" | "defer" | "go" | "chan"
            | "try" | "catch" | "throw" | "throws" | "finally"
            | "private" | "public" | "protected" | "abstract" | "final" | "override" | "virtual"
    )
}

// ---------------------------------------------------------------------------
// Fallback
// ---------------------------------------------------------------------------

fn fallback_chunk(source: &str, file_path: &str, language: &str) -> Chunk {
    let line_count = source.lines().count();
    Chunk {
        content: with_file_context(source.to_string(), file_path, 1, line_count),
        file_path: file_path.to_string(),
        language: language.to_string(),
        node_kinds: vec!["file".to_string()],
        line_start: 1,
        line_end: line_count,
        parent_scope: String::new(),
        defines: Vec::new(),
        free_calls: vec![],
        method_calls: vec![],
        other_refs: vec![],
        imports: vec![],
        type_refs: vec![],
        field_defs: vec![],
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

/// Parse imports/use statements from a file. Only returns crate-local imports for Rust.
pub fn extract_file_imports(source: &str, _file_path: &str, language: &str) -> Vec<RawImport> {
    match language {
        "rust" => extract_rust_imports(source),
        _ => vec![],
    }
}

fn extract_rust_imports(source: &str) -> Vec<RawImport> {
    let ts_language = Language::new(tree_sitter_rust::LANGUAGE);
    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return vec![];
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let source_bytes = source.as_bytes();
    let root = tree.root_node();
    let mut imports = Vec::new();

    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            let node = cursor.node();
            if node.kind() == "use_declaration" {
                collect_rust_use_imports(&node, source_bytes, "", &mut imports);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    imports
}

/// Recursively walk a use_declaration / use_list / scoped_use_list node
fn collect_rust_use_imports(
    node: &tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    imports: &mut Vec<RawImport>,
) {
    // use_declaration has an "argument" child
    let argument = if node.kind() == "use_declaration" {
        node.child_by_field_name("argument")
    } else {
        None
    };
    let target = argument.as_ref().unwrap_or(node);

    match target.kind() {
        "scoped_identifier" => {
            // e.g., crate::store::Store
            let path_text = target.utf8_text(source).unwrap_or("").to_string();
            // Only keep crate:: or super:: imports
            if path_text.starts_with("crate::") || path_text.starts_with("super::") {
                let name = target
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .unwrap_or("")
                    .to_string();
                imports.push(RawImport {
                    name,
                    path: path_text,
                });
            }
        }
        "scoped_use_list" => {
            // e.g., crate::{A, B}
            let path_node = target.child_by_field_name("path");
            let new_prefix = path_node
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or(prefix)
                .to_string();
            let list = target.child_by_field_name("list");
            if let Some(list_node) = list {
                collect_rust_use_list(&list_node, source, &new_prefix, imports);
            }
        }
        "use_list" => {
            collect_rust_use_list(target, source, prefix, imports);
        }
        "use_wildcard" => {
            // e.g., crate::foo::* — skip wildcards
        }
        "identifier" => {
            // bare identifier after some prefix
            let name = target.utf8_text(source).unwrap_or("").to_string();
            if !prefix.is_empty()
                && (prefix.starts_with("crate") || prefix.starts_with("super"))
                && !name.is_empty()
            {
                imports.push(RawImport {
                    name,
                    path: prefix.to_string(),
                });
            }
        }
        _ => {}
    }
}

fn collect_rust_use_list(
    list_node: &tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    imports: &mut Vec<RawImport>,
) {
    let mut cursor = list_node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                "identifier" => {
                    let name = child.utf8_text(source).unwrap_or("").to_string();
                    if !name.is_empty()
                        && name != "{"
                        && name != "}"
                        && name != ","
                        && (prefix.starts_with("crate") || prefix.starts_with("super"))
                    {
                        imports.push(RawImport {
                            name,
                            path: prefix.to_string(),
                        });
                    }
                }
                "scoped_identifier" => {
                    let full = child.utf8_text(source).unwrap_or("").to_string();
                    let full_path = if prefix.is_empty() {
                        full.clone()
                    } else {
                        format!("{prefix}::{full}")
                    };
                    if full_path.starts_with("crate") || full_path.starts_with("super") {
                        let name = child
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source).ok())
                            .unwrap_or("")
                            .to_string();
                        imports.push(RawImport {
                            name,
                            path: full_path,
                        });
                    }
                }
                "scoped_use_list" => {
                    let inner_path = child
                        .child_by_field_name("path")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("")
                        .to_string();
                    let new_prefix = if prefix.is_empty() {
                        inner_path
                    } else {
                        format!("{prefix}::{inner_path}")
                    };
                    if let Some(inner_list) = child.child_by_field_name("list") {
                        collect_rust_use_list(&inner_list, source, &new_prefix, imports);
                    }
                }
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type ref extraction (function signatures)
// ---------------------------------------------------------------------------

/// Primitive types to filter out from type references
fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "f32" | "f64"
            | "char" | "str"
            | "String" | "Vec" | "Option" | "Result"
            | "HashMap" | "HashSet" | "BTreeMap" | "BTreeSet"
            | "Box" | "Rc" | "Arc" | "Cell" | "RefCell"
            | "Self" | "self"
    )
}

/// Extract type identifiers from function parameter list and return type
pub fn extract_type_refs(node: &tree_sitter::Node, source: &[u8]) -> Vec<RawTypeRef> {
    let mut refs = Vec::new();

    if node.kind() != "function_item" {
        return refs;
    }

    // Extract parameter types
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_type_identifiers(&params, source, TypeRefPosition::Parameter, &mut refs);
    }

    // Extract return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        collect_type_identifiers(&ret, source, TypeRefPosition::ReturnType, &mut refs);
    }

    refs
}

fn collect_type_identifiers(
    node: &tree_sitter::Node,
    source: &[u8],
    position: TypeRefPosition,
    refs: &mut Vec<RawTypeRef>,
) {
    if node.kind() == "type_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let name = text.trim().to_string();
            if !name.is_empty() && !is_primitive_type(&name) {
                refs.push(RawTypeRef {
                    name,
                    position: position.clone(),
                });
            }
        }
        return;
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_type_identifiers(&cursor.node(), source, position.clone(), refs);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Field def extraction (structs)
// ---------------------------------------------------------------------------

/// Extract field name + type from struct field declarations
pub fn extract_field_defs(node: &tree_sitter::Node, source: &[u8]) -> Vec<RawFieldDef> {
    let mut fields = Vec::new();

    if node.kind() != "struct_item" {
        return fields;
    }

    // Find the field_declaration_list
    let body = node.child_by_field_name("body");
    if let Some(body_node) = body {
        let mut cursor = body_node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "field_declaration" {
                    let field_name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string());
                    // Find type_identifier in the type child
                    let type_name = child
                        .child_by_field_name("type")
                        .and_then(|t| find_type_identifier(&t, source));

                    if let (Some(fname), Some(tname)) = (field_name, type_name) {
                        if !is_primitive_type(&tname) {
                            fields.push(RawFieldDef {
                                field_name: fname,
                                type_name: tname,
                            });
                        }
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fields
}

/// Find the first type_identifier in a subtree
fn find_type_identifier(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "type_identifier" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(result) = find_type_identifier(&cursor.node(), source) {
                return Some(result);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}
