use crate::resolve::Candidate;

pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub related: Vec<SearchHit>,
    pub truncated: Option<usize>,
}

pub struct SearchHit {
    pub file_path: String,
    pub line_range: (usize, usize),
    pub symbols: Vec<Candidate>,
}

pub fn format_search_result(result: &SearchResult) -> String {
    let mut out = String::new();

    for hit in &result.hits {
        out.push_str(&format!("{}:{}-{}\n", hit.file_path, hit.line_range.0, hit.line_range.1));
        if !hit.symbols.is_empty() {
            let syms: Vec<String> = hit.symbols.iter()
                .map(|s| format!("{} [{}]", s.name, s.kind))
                .collect();
            out.push_str(&format!("  {}\n", syms.join(", ")));
        }
    }

    if !result.related.is_empty() {
        out.push_str("\nrelated (graph-discovered):\n");
        for hit in &result.related {
            out.push_str(&format!("  {}:{}-{}\n", hit.file_path, hit.line_range.0, hit.line_range.1));
            if !hit.symbols.is_empty() {
                let syms: Vec<String> = hit.symbols.iter()
                    .map(|s| format!("{} [{}]", s.name, s.kind))
                    .collect();
                out.push_str(&format!("    {}\n", syms.join(", ")));
            }
        }
    }

    if let Some(remaining) = result.truncated {
        out.push_str(&format!("\n... and {} more\n", remaining));
    }

    out
}
