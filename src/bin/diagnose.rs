use canopy::chunker::chunk_file;
use std::fs;

fn main() {
    let path = std::env::args().nth(1).expect("pass file path");
    let source = fs::read_to_string(&path).unwrap();
    // Use huge cap to see all references
    let chunks = chunk_file(&source, &path, "rust", 20, 200, None);
    for chunk in &chunks {
        let names: Vec<&str> = chunk.defines.iter().map(|d| d.name.as_str()).collect();
        if names.contains(&"sync_agent_visuals") {
            println!("=== ALL identifiers (before 50 cap) ===");
            println!("Total references: {}", chunk.other_refs.len());

            // Check if the names would have been in the full set
            // The cap happens inside extract_references after sort
            // So we need to check if they'd be after position 50 alphabetically
            for r in &chunk.other_refs {
                println!("  {}", r);
            }
        }
    }
}
