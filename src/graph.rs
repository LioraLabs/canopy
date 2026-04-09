use crate::chunker::{Chunk, TypeRefPosition};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A graph entity extracted deterministically from code structure
#[derive(Debug, Clone)]
pub struct EntityDef {
    pub entity_name: String,
    pub entity_type: String,
    pub description: String,
    pub source_id: String,
    pub metadata: serde_json::Value,
    pub parent: Option<String>,
    pub visibility: Option<String>,
}

/// A graph relationship between two entities
#[derive(Debug, Clone)]
pub struct RelationshipDef {
    pub src_id: String,
    pub tgt_id: String,
    pub relationship_type: String,
    pub keywords: String,
    pub weight: f64,
    pub description: String,
    pub source_id: String,
    pub ambiguous: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Maps a SymbolDef kind string to a graph entity type.
/// Returns None for kinds that should be skipped (e.g. "impl").
pub fn entity_type_for_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "function" | "method" => Some("FUNCTION"),
        "struct" | "class" | "enum" | "record" => Some("TYPE"),
        "trait" | "interface" => Some("TRAIT"),
        "const" | "static" => Some("CONSTANT"),
        "module" | "namespace" => Some("MODULE"),
        _ => None,
    }
}

/// Builds a qualified entity name: `{NAME}::{FILE_PATH}` all uppercase.
pub fn qualified_name(symbol_name: &str, file_path: &str) -> String {
    format!("{}::{}", symbol_name, file_path).to_uppercase()
}

// ---------------------------------------------------------------------------
// Entity generation
// ---------------------------------------------------------------------------

/// Extracts the visibility modifier from a signature string.
pub fn extract_visibility(signature: &str) -> Option<String> {
    let sig = signature.trim();
    if sig.starts_with("pub(crate)") { Some("pub(crate)".to_string()) }
    else if sig.starts_with("pub(super)") { Some("pub(super)".to_string()) }
    else if sig.starts_with("pub ") || sig.starts_with("pub(") { Some("pub".to_string()) }
    else if sig.starts_with("export ") { Some("export".to_string()) }
    else { Some("private".to_string()) }
}

/// Generates graph entities from a chunk's defined symbols.
pub fn generate_entities(chunk: &Chunk, source_doc_id: &str) -> Vec<EntityDef> {
    let parent = if chunk.parent_scope.is_empty() {
        None
    } else {
        Some(chunk.parent_scope.clone())
    };

    chunk
        .defines
        .iter()
        .filter_map(|sym| {
            let entity_type = entity_type_for_kind(&sym.kind)?;
            let visibility = extract_visibility(&sym.signature);
            Some(EntityDef {
                entity_name: qualified_name(&sym.name, &chunk.file_path),
                entity_type: entity_type.to_string(),
                description: sym.signature.clone(),
                source_id: source_doc_id.to_string(),
                metadata: serde_json::json!({
                    "name": sym.name,
                    "file_path": chunk.file_path,
                    "line": sym.line,
                    "kind": sym.kind,
                    "signature": sym.signature,
                }),
                parent: parent.clone(),
                visibility,
            })
        })
        .collect()
}

/// Creates a MODULE entity representing an entire file.
pub fn generate_module_entity(file_path: &str) -> EntityDef {
    EntityDef {
        entity_name: file_path.to_uppercase(),
        entity_type: "MODULE".to_string(),
        description: format!("File module: {}", file_path),
        source_id: "canopy".to_string(),
        metadata: serde_json::json!({
            "name": file_path,
            "file_path": file_path,
        }),
        parent: None,
        visibility: Some("pub".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Symbol map
// ---------------------------------------------------------------------------

/// Builds a map from lowercase symbol name to all qualified entity names
/// and their entity kinds across all files.
pub fn build_symbol_map(
    file_chunks: &[(String, Vec<Chunk>)],
    source_doc_ids: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<(String, String)>> {
    let mut map: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for (file_path, chunks) in file_chunks {
        let source_id = source_doc_ids
            .get(file_path)
            .and_then(|ids| ids.first())
            .map(|s| s.as_str())
            .unwrap_or("canopy");

        for chunk in chunks {
            let entities = generate_entities(chunk, source_id);
            for entity in &entities {
                let key = entity.entity_name.split("::").next().unwrap_or("").to_lowercase();
                map.entry(key)
                    .or_default()
                    .push((entity.entity_name.clone(), entity.entity_type.clone()));
            }
        }
    }

    map
}

// ---------------------------------------------------------------------------
// Relationship generation
// ---------------------------------------------------------------------------

/// Generates MODULE → symbol "defines" relationships for a file.
pub fn generate_defines_relationships(
    file_path: &str,
    entities: &[EntityDef],
) -> Vec<RelationshipDef> {
    let module_name = file_path.to_uppercase();

    entities
        .iter()
        .filter(|e| e.entity_type != "MODULE")
        .map(|e| RelationshipDef {
            src_id: module_name.clone(),
            tgt_id: e.entity_name.clone(),
            relationship_type: "DEFINES".to_string(),
            keywords: "defines, contains".to_string(),
            weight: 1.0,
            description: format!("{} defines {}", module_name, e.entity_name),
            source_id: "canopy".to_string(),
            ambiguous: false,
        })
        .collect()
}

/// Generates parent_scope → child symbol "contains" relationships.
///
/// Looks up the chunk's `parent_scope` (lowercased) in the symbol map to find
/// the parent entity in the same file, then creates edges to each defined
/// symbol in the chunk.
pub fn generate_contains_relationships(
    chunk: &Chunk,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    if chunk.parent_scope.is_empty() {
        return vec![];
    }

    let parent_key = chunk.parent_scope.to_lowercase();
    let file_upper = chunk.file_path.to_uppercase();

    // Find the parent entity in the same file
    let parent_entity = symbol_map
        .get(&parent_key)
        .and_then(|candidates| {
            candidates
                .iter()
                .find(|(name, _kind)| name.ends_with(&format!("::{}", file_upper)))
        });

    let parent_name = match parent_entity {
        Some((name, _kind)) => name.clone(),
        None => return vec![],
    };

    chunk
        .defines
        .iter()
        .filter_map(|sym| {
            entity_type_for_kind(&sym.kind)?;
            let child_name = qualified_name(&sym.name, &chunk.file_path);
            Some(RelationshipDef {
                src_id: parent_name.clone(),
                tgt_id: child_name.clone(),
                relationship_type: "CONTAINS".to_string(),
                keywords: "contains, parent".to_string(),
                weight: 1.0,
                description: format!("{} contains {}", parent_name, child_name),
                source_id: "canopy".to_string(),
                ambiguous: false,
            })
        })
        .collect()
}

/// Bucket used to determine resolution strategy for a reference.
enum RefBucket {
    FreeCall,
    MethodCall,
    OtherRef,
}

/// Generates classified relationships from the three reference buckets.
///
/// Each bucket uses a different resolution strategy:
/// - **Free calls**: highest confidence — resolve up to 4 candidates, tag
///   multi-file matches as ambiguous.
/// - **Method calls**: only resolve when exactly 1 candidate exists.
/// - **Other refs**: same rules as free calls, but non-FUNCTION targets
///   become REFERENCES edges instead of CALLS.
pub fn generate_classified_relationships(
    chunk: &Chunk,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    let src_sym = chunk
        .defines
        .iter()
        .find(|sym| entity_type_for_kind(&sym.kind).is_some());

    let src_sym = match src_sym {
        Some(s) => s,
        None => return vec![],
    };

    let src_name = qualified_name(&src_sym.name, &chunk.file_path);
    let mut rels = Vec::new();

    for name in &chunk.free_calls {
        resolve_ref(&src_name, name, symbol_map, RefBucket::FreeCall, &mut rels);
    }
    for name in &chunk.method_calls {
        resolve_ref(&src_name, name, symbol_map, RefBucket::MethodCall, &mut rels);
    }
    for name in &chunk.other_refs {
        resolve_ref(&src_name, name, symbol_map, RefBucket::OtherRef, &mut rels);
    }

    rels
}

/// Resolves a single reference against the symbol map using bucket-specific rules.
fn resolve_ref(
    src_name: &str,
    ref_name: &str,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
    bucket: RefBucket,
    rels: &mut Vec<RelationshipDef>,
) {
    let ref_key = ref_name.to_lowercase();
    let targets = match symbol_map.get(&ref_key) {
        Some(t) if !t.is_empty() && t.len() <= 4 => t,
        _ => return,
    };

    // Method calls: only resolve if exactly 1 candidate
    if matches!(bucket, RefBucket::MethodCall) && targets.len() > 1 {
        return;
    }

    // Same-file overload check
    let all_same_file = targets.len() > 1 && {
        let first_file = extract_file_from_qualified(&targets[0].0);
        targets
            .iter()
            .all(|t| extract_file_from_qualified(&t.0) == first_file)
    };

    let ambiguous = targets.len() > 1 && !all_same_file;
    let weight = if all_same_file {
        1.0
    } else {
        1.0 / targets.len() as f64
    };

    for (tgt_name, tgt_kind) in targets {
        if *tgt_name == src_name {
            continue;
        }

        let rel_type = match bucket {
            RefBucket::OtherRef if tgt_kind != "FUNCTION" => "REFERENCES",
            _ => "CALLS",
        };
        let keywords = if rel_type == "CALLS" {
            "calls, invokes"
        } else {
            "references, uses"
        };

        rels.push(RelationshipDef {
            src_id: src_name.to_string(),
            tgt_id: tgt_name.clone(),
            relationship_type: rel_type.to_string(),
            keywords: keywords.to_string(),
            weight,
            description: format!("{} {} {}", src_name, rel_type.to_lowercase(), tgt_name),
            source_id: "canopy".to_string(),
            ambiguous,
        });
    }
}

/// Extracts the file path portion from a qualified name like "FOO::SRC/A.RS".
fn extract_file_from_qualified(qname: &str) -> &str {
    qname.split("::").nth(1).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Task 7: New typed relationship generators
// ---------------------------------------------------------------------------

/// Parse an impl signature to extract (trait_name, type_name) if it is a
/// "impl Trait for Type" form. Returns None for plain impl blocks.
pub fn parse_impl_signature(sig: &str) -> Option<(String, String)> {
    let sig = sig.trim();
    if !sig.starts_with("impl") {
        return None;
    }
    let rest = sig.strip_prefix("impl")?.trim();
    // Skip generic params if present (e.g. impl<T> ...)
    let rest = if rest.starts_with('<') {
        let mut depth = 0usize;
        let mut end = 0usize;
        for (i, c) in rest.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest[end..].trim()
    } else {
        rest
    };
    let parts: Vec<&str> = rest.splitn(2, " for ").collect();
    if parts.len() != 2 {
        return None;
    }
    let trait_name = parts[0].trim().split('<').next()?.trim().to_string();
    let type_name = parts[1]
        .trim()
        .split('<')
        .next()?
        .split('{')
        .next()?
        .trim()
        .to_string();
    if trait_name.is_empty() || type_name.is_empty() {
        return None;
    }
    Some((trait_name, type_name))
}

/// Generates ACCEPTS/RETURNS relationships from `chunk.type_refs`.
pub fn generate_type_ref_relationships(
    chunk: &Chunk,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    let src_sym = chunk
        .defines
        .iter()
        .find(|sym| entity_type_for_kind(&sym.kind).is_some());
    let src_sym = match src_sym {
        Some(s) => s,
        None => return vec![],
    };
    let src_name = qualified_name(&src_sym.name, &chunk.file_path);

    let mut rels = Vec::new();

    for type_ref in &chunk.type_refs {
        let key = type_ref.name.to_lowercase();
        let targets = match symbol_map.get(&key) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        let (rel_type, keywords) = match type_ref.position {
            TypeRefPosition::Parameter => ("ACCEPTS", "accepts, parameter"),
            TypeRefPosition::ReturnType => ("RETURNS", "returns, result"),
        };

        for (tgt_name, _kind) in targets {
            if *tgt_name == src_name {
                continue;
            }
            rels.push(RelationshipDef {
                src_id: src_name.clone(),
                tgt_id: tgt_name.clone(),
                relationship_type: rel_type.to_string(),
                keywords: keywords.to_string(),
                weight: 0.9,
                description: format!("{} {} {}", src_name, rel_type.to_lowercase(), tgt_name),
                source_id: "canopy".to_string(),
                ambiguous: false,
            });
        }
    }

    rels
}

/// Generates FIELD_OF relationships from `chunk.field_defs`.
///
/// For each field, looks up the field's type in the symbol map. The source is
/// the struct/enum entity in this chunk; the target is the resolved type entity.
pub fn generate_field_of_relationships(
    chunk: &Chunk,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    let src_sym = chunk
        .defines
        .iter()
        .find(|sym| entity_type_for_kind(&sym.kind).is_some());
    let src_sym = match src_sym {
        Some(s) => s,
        None => return vec![],
    };
    let src_name = qualified_name(&src_sym.name, &chunk.file_path);

    let mut rels = Vec::new();

    for field in &chunk.field_defs {
        let key = field.type_name.to_lowercase();
        let targets = match symbol_map.get(&key) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };
        for (tgt_name, _kind) in targets {
            if *tgt_name == src_name {
                continue;
            }
            rels.push(RelationshipDef {
                src_id: src_name.clone(),
                tgt_id: tgt_name.clone(),
                relationship_type: "FIELD_OF".to_string(),
                keywords: "field, member".to_string(),
                weight: 0.8,
                description: format!(
                    "{} has field '{}' of type {}",
                    src_name, field.field_name, tgt_name
                ),
                source_id: "canopy".to_string(),
                ambiguous: false,
            });
        }
    }

    rels
}

/// Generates IMPLEMENTS relationships by parsing impl signatures.
///
/// For each `impl` symbol in the chunk with a "impl Trait for Type" signature,
/// resolves both Trait and Type in the symbol_map and emits Type IMPLEMENTS Trait.
pub fn generate_implements_relationships(
    chunk: &Chunk,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    let mut rels = Vec::new();

    for sym in &chunk.defines {
        if sym.kind != "impl" {
            continue;
        }
        let (trait_name, type_name) = match parse_impl_signature(&sym.signature) {
            Some(pair) => pair,
            None => continue,
        };

        let trait_key = trait_name.to_lowercase();
        let type_key = type_name.to_lowercase();

        let trait_targets = match symbol_map.get(&trait_key) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };
        let type_targets = match symbol_map.get(&type_key) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        // Use the first candidate for each (most specific match)
        let (tgt_name, _) = &trait_targets[0];
        let (src_name, _) = &type_targets[0];

        if src_name == tgt_name {
            continue;
        }

        rels.push(RelationshipDef {
            src_id: src_name.clone(),
            tgt_id: tgt_name.clone(),
            relationship_type: "IMPLEMENTS".to_string(),
            keywords: "implements, trait".to_string(),
            weight: 1.0,
            description: format!("{} implements {}", src_name, tgt_name),
            source_id: "canopy".to_string(),
            ambiguous: false,
        });
    }

    rels
}

/// Deduplicates relationships by (src, tgt) pair, keeping the highest-priority edge type.
///
/// Priority: CALLS > ACCEPTS/RETURNS > everything else (REFERENCES, DEFINES, etc.)
pub fn dedup_relationships(rels: Vec<RelationshipDef>) -> Vec<RelationshipDef> {
    let mut best: HashMap<(String, String), RelationshipDef> = HashMap::new();

    for rel in rels {
        let key = (rel.src_id.clone(), rel.tgt_id.clone());
        let dominated = best.get(&key).is_some_and(|existing| {
            edge_priority(&existing.relationship_type) >= edge_priority(&rel.relationship_type)
        });
        if !dominated {
            best.insert(key, rel);
        }
    }

    best.into_values().collect()
}

fn edge_priority(rel_type: &str) -> u8 {
    match rel_type {
        "CALLS" => 3,
        "ACCEPTS" | "RETURNS" => 2,
        _ => 1,
    }
}

/// Generates IMPORTS relationships from a file's imports list.
///
/// For each import, looks up the imported name (lowercased) in the symbol_map
/// and emits MODULE IMPORTS symbol edges.
pub fn generate_imports_relationships(
    imports: &[crate::chunker::RawImport],
    file_path: &str,
    symbol_map: &HashMap<String, Vec<(String, String)>>,
) -> Vec<RelationshipDef> {
    let module_name = file_path.to_uppercase();
    let mut rels = Vec::new();

    for import in imports {
        // The imported name is the last segment of the path
        let name = import.name.split("::").last().unwrap_or(&import.name);
        let key = name.to_lowercase();
        let targets = match symbol_map.get(&key) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };
        for (tgt_name, _kind) in targets {
            if *tgt_name == module_name {
                continue;
            }
            rels.push(RelationshipDef {
                src_id: module_name.clone(),
                tgt_id: tgt_name.clone(),
                relationship_type: "IMPORTS".to_string(),
                keywords: "imports, uses".to_string(),
                weight: 0.7,
                description: format!("{} imports {}", module_name, tgt_name),
                source_id: "canopy".to_string(),
                ambiguous: false,
            });
        }
    }

    rels
}
