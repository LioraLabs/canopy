use crate::resolve::Candidate;

pub struct ClusterResult {
    pub label: String,
    pub members: Vec<Candidate>,
    pub relationships: Vec<ClusterEdge>,
    pub truncated_members: Option<usize>,
    pub truncated_relationships: Option<usize>,
}

pub struct ClusterEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
}

pub fn format_cluster_result(result: &ClusterResult) -> String {
    let mut out = String::new();

    out.push_str(&format!("cluster: {} ({} members)\n", result.label, result.members.len()));

    for m in &result.members {
        out.push_str(&format!("  {} [{}] {}:{}\n", m.name, m.kind, m.file_path, m.line));
    }

    if let Some(remaining) = result.truncated_members {
        out.push_str(&format!("  ... and {} more members\n", remaining));
    }

    if !result.relationships.is_empty() {
        out.push_str("\n  key relationships:\n");
        for r in &result.relationships {
            out.push_str(&format!("    {} --{}--> {}\n", r.source, r.edge_type, r.target));
        }
    }

    if let Some(remaining) = result.truncated_relationships {
        out.push_str(&format!("    ... and {} more relationships\n", remaining));
    }

    out
}
