use crate::resolve::{Candidate, Resolution};

pub struct MapResult {
    pub entity: EntityDetail,
    pub resolution: Resolution,
}

pub struct EntityDetail {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: usize,
    pub signature: String,
    pub cluster_label: Option<String>,
    pub calls: Vec<Candidate>,
    pub called_by: Vec<Candidate>,
    pub references: Vec<Candidate>,
    pub referenced_by: Vec<Candidate>,
    pub accepts: Vec<Candidate>,
    pub accepted_by: Vec<Candidate>,
    pub returns: Vec<Candidate>,
    pub returned_by: Vec<Candidate>,
    pub field_of: Vec<Candidate>,
    pub has_fields: Vec<Candidate>,
    pub implements: Vec<Candidate>,
    pub implemented_by: Vec<Candidate>,
}

pub fn format_map_result(result: &MapResult) -> String {
    let e = &result.entity;
    let mut out = String::new();

    out.push_str(&format!("{} [{}]\n", e.name, e.kind));
    out.push_str(&format!("  file: {}:{}\n", e.file_path, e.line));
    out.push_str(&format!("  signature: {}\n", e.signature));

    if let Some(label) = &e.cluster_label {
        out.push_str(&format!("  cluster: {}\n", label));
    }

    format_rel_section(&mut out, "calls", &e.calls);
    format_rel_section(&mut out, "called_by", &e.called_by);
    format_rel_section(&mut out, "references", &e.references);
    format_rel_section(&mut out, "referenced_by", &e.referenced_by);
    format_rel_section(&mut out, "accepts", &e.accepts);
    format_rel_section(&mut out, "accepted_by", &e.accepted_by);
    format_rel_section(&mut out, "returns", &e.returns);
    format_rel_section(&mut out, "returned_by", &e.returned_by);
    format_rel_section(&mut out, "field_of", &e.field_of);
    format_rel_section(&mut out, "has_fields", &e.has_fields);
    format_rel_section(&mut out, "implements", &e.implements);
    format_rel_section(&mut out, "implemented_by", &e.implemented_by);

    out
}

fn format_rel_section(out: &mut String, label: &str, candidates: &[Candidate]) {
    if candidates.is_empty() {
        return;
    }
    out.push_str(&format!("\n  {}:\n", label));
    for c in candidates {
        if c.ambiguous {
            out.push_str(&format!(
                "    {} [{}] {}:{} (ambiguous)\n",
                c.name, c.kind, c.file_path, c.line
            ));
        } else {
            out.push_str(&format!(
                "    {} [{}] {}:{}\n",
                c.name, c.kind, c.file_path, c.line
            ));
        }
    }
}
