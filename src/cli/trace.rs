use crate::resolve::{Candidate, Resolution};

pub struct TraceResult {
    pub hops: Vec<TraceHop>,
    pub resolution_from: Resolution,
    pub resolution_to: Resolution,
}

pub struct TraceHop {
    pub from: Candidate,
    pub to: Candidate,
    pub edge_type: String,
    pub edge_cost: f64,
    pub forward: bool,
}

pub fn format_trace_result(result: &TraceResult) -> String {
    let mut out = String::new();

    if result.hops.is_empty() {
        out.push_str("No trace found.\n");
        return out;
    }

    // Summary line: a -> b -> c
    let mut names: Vec<&str> = vec![&result.hops[0].from.name];
    for hop in &result.hops {
        names.push(&hop.to.name);
    }
    out.push_str(&names.join(" -> "));
    out.push('\n');

    // Detail: each hop
    let first = &result.hops[0];
    out.push_str(&format!(
        "  {} [{}] {}:{}\n",
        first.from.name, first.from.kind, first.from.file_path, first.from.line
    ));

    for hop in &result.hops {
        let arrow = if hop.forward {
            format!("--{}-->", hop.edge_type)
        } else {
            format!("<--{}--", hop.edge_type)
        };
        out.push_str(&format!(
            "    {} {} [{}] {}:{}\n",
            arrow, hop.to.name, hop.to.kind, hop.to.file_path, hop.to.line
        ));
    }

    out
}
