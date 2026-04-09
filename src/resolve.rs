use crate::store::Store;

#[derive(Debug)]
pub enum Resolution {
    Exact(String),
    Suggestions {
        input: String,
        candidates: Vec<Candidate>,
        total: usize,
    },
    NoMatch(String),
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: usize,
    pub ambiguous: bool,
}

pub struct FuzzyResolver<'a> {
    store: &'a Store,
    max_suggestions: usize,
}

/// Known kind qualifiers for `kind:Name` syntax.
const KNOWN_KINDS: &[&str] = &[
    "struct", "function", "trait", "module", "enum", "const",
    "class", "interface", "method", "namespace",
];

/// Parsed qualifier information extracted from user input.
struct ParsedInput {
    name: String,
    kind_filter: Option<String>,
    file_filter: Option<String>,
}

/// Parse user input for qualifier syntax.
///
/// Supported formats:
/// - `Analysis [struct]` — bracket format, kind extracted from `[...]`
/// - `struct:Analysis` — kind prefix (first `:` where left is a known kind)
/// - `crates/ide/src/lib.rs:Analysis` — file qualifier (last `:` where left contains `/` or `.`)
/// - `struct:crates/ide/src/lib.rs:Analysis` — combined: kind first, then file from remainder
fn parse_input(raw: &str) -> ParsedInput {
    // Bracket format: `Analysis [struct]`
    if let Some(bracket_start) = raw.rfind('[') {
        if let Some(bracket_end) = raw.rfind(']') {
            if bracket_end > bracket_start {
                let kind = raw[bracket_start + 1..bracket_end].trim().to_lowercase();
                if KNOWN_KINDS.contains(&kind.as_str()) {
                    let name = raw[..bracket_start].trim().to_string();
                    return ParsedInput { name, kind_filter: Some(kind), file_filter: None };
                }
            }
        }
    }

    let mut remainder = raw;
    let mut kind_filter = None;

    // Check for kind prefix: `struct:...`
    if let Some(colon_pos) = remainder.find(':') {
        let left = &remainder[..colon_pos];
        if KNOWN_KINDS.contains(&left.to_lowercase().as_str()) {
            kind_filter = Some(left.to_lowercase());
            remainder = &remainder[colon_pos + 1..];
        }
    }

    // Check for file qualifier in remainder: split on last `:` where left contains `/` or `.`
    let mut file_filter = None;
    if let Some(colon_pos) = remainder.rfind(':') {
        let left = &remainder[..colon_pos];
        if left.contains('/') || left.contains('.') {
            file_filter = Some(left.to_string());
            remainder = &remainder[colon_pos + 1..];
        }
    }

    ParsedInput {
        name: remainder.to_string(),
        kind_filter,
        file_filter,
    }
}

impl<'a> FuzzyResolver<'a> {
    pub fn new(store: &'a Store, max_suggestions: usize) -> Self {
        Self { store, max_suggestions }
    }

    pub fn resolve(&self, input: &str) -> Resolution {
        let all_entities = match self.store.all_entity_names() {
            Ok(e) => e,
            Err(_) => return Resolution::NoMatch(input.to_string()),
        };

        let parsed = parse_input(input);
        let has_qualifiers = parsed.kind_filter.is_some() || parsed.file_filter.is_some();

        // Apply qualifier filters to narrow candidate set
        let candidates: Vec<&String> = if has_qualifiers {
            all_entities.iter()
                .filter(|key| self.matches_qualifiers(key, &parsed))
                .collect()
        } else {
            all_entities.iter().collect()
        };

        if has_qualifiers && candidates.is_empty() {
            return Resolution::NoMatch(input.to_string());
        }

        let search_name = &parsed.name;
        let search_upper = search_name.to_uppercase();

        // Stage 0: Case-sensitive exact match on display name
        let cs_exact: Vec<&String> = candidates.iter()
            .filter(|e| {
                self.display_name_for_key(e)
                    .map(|n| n == *search_name)
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if cs_exact.len() == 1 { return Resolution::Exact(cs_exact[0].clone()); }
        if cs_exact.len() > 1 { return self.make_suggestions(input, &cs_exact); }

        // Stage 1: Case-insensitive exact match on display name
        let ci_exact: Vec<&String> = candidates.iter()
            .filter(|e| {
                self.display_name_for_key(e)
                    .map(|n| n.to_uppercase() == search_upper)
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if ci_exact.len() == 1 { return Resolution::Exact(ci_exact[0].clone()); }
        if ci_exact.len() > 1 { return self.make_suggestions(input, &ci_exact); }

        // Stage 2: Prefix match (case-insensitive)
        let prefix: Vec<&String> = candidates.iter()
            .filter(|e| {
                self.display_name_for_key(e)
                    .map(|n| n.to_uppercase().starts_with(&search_upper))
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if prefix.len() == 1 { return Resolution::Exact(prefix[0].clone()); }
        if prefix.len() > 1 { return self.make_suggestions(input, &prefix); }

        // Stage 3: Substring match (case-insensitive)
        let substring: Vec<&String> = candidates.iter()
            .filter(|e| {
                self.display_name_for_key(e)
                    .map(|n| n.to_uppercase().contains(&search_upper))
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if substring.len() == 1 { return Resolution::Exact(substring[0].clone()); }
        if substring.len() > 1 { return self.make_suggestions(input, &substring); }

        // Stage 4: Fuzzy (edit distance)
        let mut scored: Vec<(&String, usize)> = candidates.iter()
            .filter_map(|e| {
                let name = self.display_name_for_key(e)?;
                let dist = edit_distance(&search_upper, &name.to_uppercase());
                let threshold = (search_name.len() / 3 + 1).min(5);
                if dist <= threshold { Some((*e, dist)) } else { None }
            })
            .collect();

        scored.sort_by_key(|(_, dist)| *dist);

        if scored.is_empty() { return Resolution::NoMatch(input.to_string()); }

        let keys: Vec<&String> = scored.iter().map(|(k, _)| *k).collect();
        self.make_suggestions(input, &keys)
    }

    /// Check if an entity matches the parsed kind and/or file qualifiers.
    fn matches_qualifiers(&self, key: &str, parsed: &ParsedInput) -> bool {
        let rec = match self.store.get_entity(key).ok().flatten() {
            Some(r) => r,
            None => return false,
        };
        let meta = rec.metadata_value();

        if let Some(ref kind_filter) = parsed.kind_filter {
            let entity_kind = meta.get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or(&rec.entity_type);
            if entity_kind.to_lowercase() != *kind_filter {
                return false;
            }
        }

        if let Some(ref file_filter) = parsed.file_filter {
            if !rec.file_path.ends_with(file_filter.as_str()) && rec.file_path != *file_filter {
                return false;
            }
        }

        true
    }

    fn make_suggestions(&self, input: &str, keys: &[&String]) -> Resolution {
        let total = keys.len();
        let candidates: Vec<Candidate> = keys.iter()
            .take(self.max_suggestions)
            .filter_map(|key| self.entity_to_candidate(key))
            .collect();

        Resolution::Suggestions { input: input.to_string(), candidates, total }
    }

    fn display_name_for_key(&self, key: &str) -> Option<String> {
        let rec = self.store.get_entity(key).ok()??;
        let meta = rec.metadata_value();
        meta.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
    }

    fn entity_to_candidate(&self, key: &str) -> Option<Candidate> {
        let rec = self.store.get_entity(key).ok()??;
        let meta = rec.metadata_value();
        Some(Candidate {
            name: meta.get("name")?.as_str()?.to_string(),
            kind: meta.get("kind").and_then(|k| k.as_str()).unwrap_or(&rec.entity_type).to_string(),
            file_path: rec.file_path.clone(),
            line: meta.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize,
            ambiguous: false,
        })
    }
}

/// Format a resolution result for CLI output.
pub fn format_resolution(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Exact(_) => String::new(),
        Resolution::Suggestions { input, candidates, total } => {
            let mut out = if candidates.iter().any(|c| c.name.to_lowercase() == input.to_lowercase()) {
                format!("Multiple matches for \"{}\" ({} total):\n", input, total)
            } else {
                format!("No exact match for \"{}\". Similar ({} total):\n", input, total)
            };
            for c in candidates {
                out.push_str(&format!("  {}:{} [{}]\n", c.file_path, c.name, c.kind));
            }
            out
        }
        Resolution::NoMatch(input) => {
            format!("No match for \"{}\".\n", input)
        }
    }
}

#[allow(clippy::needless_range_loop)]
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1).min(dp[i][j - 1] + 1).min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n]
}
