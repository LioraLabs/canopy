use std::collections::HashMap;
use std::collections::HashSet;

/// Run Louvain modularity optimization to find communities.
///
/// Each entity starts in its own community. The algorithm iterates two phases:
/// Phase 1 (local moves): For each node, compute modularity gain of moving to
/// each neighbor's community. Move to highest positive gain. Repeat until stable.
/// Phase 2 (aggregation): Collapse communities into super-nodes, build new
/// weighted graph, recurse.
/// Stops when Phase 1 produces no moves.
///
/// The `resolution` parameter (gamma) controls cluster granularity.
/// Higher values produce more, smaller clusters.
pub fn louvain(
    entities: &[String],
    affinities: &HashMap<(String, String), f64>,
    resolution: f64,
) -> HashMap<String, u32> {
    if entities.is_empty() {
        return HashMap::new();
    }

    // Map entity names to indices for internal computation
    let name_to_idx: HashMap<&str, usize> = entities.iter().enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    let n = entities.len();

    // Build adjacency: vec of (neighbor_idx, weight) per node
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for ((a, b), &weight) in affinities {
        if let (Some(&ia), Some(&ib)) = (name_to_idx.get(a.as_str()), name_to_idx.get(b.as_str())) {
            adj[ia].push((ib, weight));
            adj[ib].push((ia, weight));
        }
    }

    // Weighted degree of each node
    let k: Vec<f64> = (0..n).map(|i| adj[i].iter().map(|(_, w)| w).sum()).collect();

    // Total weight of all edges (sum of all edge weights, each counted once)
    let m: f64 = affinities.values().sum();

    // Community assignment: each node starts in its own community
    let mut community: Vec<usize> = (0..n).collect();

    if m == 0.0 {
        // No edges: each node is its own cluster
        let mut result = HashMap::new();
        for (i, name) in entities.iter().enumerate() {
            result.insert(name.clone(), i as u32);
        }
        return result;
    }

    // Phase 1: Local moves
    loop {
        let mut improved = true;
        while improved {
            improved = false;
            for i in 0..n {
                let current_comm = community[i];

                // Compute sum of weights from i to each neighboring community
                let mut comm_weights: HashMap<usize, f64> = HashMap::new();
                for &(j, w) in &adj[i] {
                    *comm_weights.entry(community[j]).or_insert(0.0) += w;
                }

                // k_i_in for current community (weight from i to its own community)
                let k_i_in_current = comm_weights.get(&current_comm).copied().unwrap_or(0.0);

                // Sigma_tot for current community (sum of degrees of nodes in current community, excluding i)
                let sigma_tot_current: f64 = (0..n)
                    .filter(|&node| node != i && community[node] == current_comm)
                    .map(|node| k[node])
                    .sum();

                // Modularity loss from removing i from current community
                let loss = k_i_in_current / m - resolution * sigma_tot_current * k[i] / (2.0 * m * m);

                let mut best_comm = current_comm;
                let mut best_gain = 0.0_f64;

                for (&target_comm, &k_i_in_target) in &comm_weights {
                    if target_comm == current_comm {
                        continue;
                    }

                    // Sigma_tot for target community
                    let sigma_tot_target: f64 = (0..n)
                        .filter(|&node| community[node] == target_comm)
                        .map(|node| k[node])
                        .sum();

                    // Modularity gain from adding i to target community
                    let gain_add = k_i_in_target / m - resolution * sigma_tot_target * k[i] / (2.0 * m * m);

                    // Net gain = gain from adding to target - loss from removing from current
                    let delta_q = gain_add - loss;

                    if delta_q > best_gain {
                        best_gain = delta_q;
                        best_comm = target_comm;
                    }
                }

                if best_comm != current_comm {
                    community[i] = best_comm;
                    improved = true;
                }
            }
        }

        // Phase 2: Check if aggregation would change anything
        // Collect unique communities
        let unique_comms: HashSet<usize> = community.iter().copied().collect();
        if unique_comms.len() == n {
            // No merges happened, we're done
            break;
        }

        // Map old community IDs to new sequential super-node IDs
        let comm_list: Vec<usize> = unique_comms.into_iter().collect();
        let comm_to_super: HashMap<usize, usize> = comm_list.iter().enumerate()
            .map(|(new_id, &old_id)| (old_id, new_id))
            .collect();
        let n_super = comm_list.len();

        // Build super-graph
        let mut super_adj_map: HashMap<(usize, usize), f64> = HashMap::new();
        let mut super_k: Vec<f64> = vec![0.0; n_super];

        for i in 0..n {
            let si = comm_to_super[&community[i]];
            super_k[si] += k[i];
            for &(j, w) in &adj[i] {
                if i < j {
                    let sj = comm_to_super[&community[j]];
                    if si != sj {
                        let key = if si < sj { (si, sj) } else { (sj, si) };
                        *super_adj_map.entry(key).or_insert(0.0) += w;
                    }
                }
            }
        }

        // Build super adjacency list
        let mut super_adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n_super];
        for (&(a, b), &w) in &super_adj_map {
            super_adj[a].push((b, w));
            super_adj[b].push((a, w));
        }

        // Run another local move phase on the super-graph
        let mut super_community: Vec<usize> = (0..n_super).collect();
        let mut any_super_move = false;

        let mut super_improved = true;
        while super_improved {
            super_improved = false;
            for si in 0..n_super {
                let current_comm = super_community[si];

                let mut comm_weights: HashMap<usize, f64> = HashMap::new();
                for &(sj, w) in &super_adj[si] {
                    *comm_weights.entry(super_community[sj]).or_insert(0.0) += w;
                }

                let k_i_in_current = comm_weights.get(&current_comm).copied().unwrap_or(0.0);
                let sigma_tot_current: f64 = (0..n_super)
                    .filter(|&node| node != si && super_community[node] == current_comm)
                    .map(|node| super_k[node])
                    .sum();

                let loss = k_i_in_current / m - resolution * sigma_tot_current * super_k[si] / (2.0 * m * m);

                let mut best_comm = current_comm;
                let mut best_gain = 0.0_f64;

                for (&target_comm, &k_i_in_target) in &comm_weights {
                    if target_comm == current_comm {
                        continue;
                    }

                    let sigma_tot_target: f64 = (0..n_super)
                        .filter(|&node| super_community[node] == target_comm)
                        .map(|node| super_k[node])
                        .sum();

                    let gain_add = k_i_in_target / m - resolution * sigma_tot_target * super_k[si] / (2.0 * m * m);
                    let delta_q = gain_add - loss;

                    if delta_q > best_gain {
                        best_gain = delta_q;
                        best_comm = target_comm;
                    }
                }

                if best_comm != current_comm {
                    super_community[si] = best_comm;
                    super_improved = true;
                    any_super_move = true;
                }
            }
        }

        if !any_super_move {
            break;
        }

        // Propagate super-community assignments back to original nodes
        for i in 0..n {
            let si = comm_to_super[&community[i]];
            community[i] = super_community[si];
        }
    }

    // Normalize community IDs to sequential u32
    let mut comm_id_map: HashMap<usize, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut labels: HashMap<String, u32> = HashMap::new();
    for (i, name) in entities.iter().enumerate() {
        let comm = community[i];
        let id = *comm_id_map.entry(comm).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        labels.insert(name.clone(), id);
    }

    labels
}

/// Build the affinity graph from stored relationships.
#[allow(clippy::type_complexity)]
pub fn build_affinity_graph(
    store: &crate::store::Store,
    hub_entities: &[String],
) -> anyhow::Result<(Vec<String>, HashMap<(String, String), f64>)> {
    use std::collections::HashSet;

    let hub_set: HashSet<&str> = hub_entities.iter().map(|s| s.as_str()).collect();
    let mut entity_set: HashSet<String> = HashSet::new();
    let mut entities: Vec<String> = Vec::new();
    let mut affinities: HashMap<(String, String), f64> = HashMap::new();

    // Collect all non-hub, non-module entities
    let all_names = store.all_entity_names()?;
    for name in &all_names {
        if hub_set.contains(name.as_str()) { continue; }
        let is_module = store.get_entity_type(name)
            .ok().flatten()
            .map(|t| t == "MODULE")
            .unwrap_or(false);
        if is_module { continue; }
        if entity_set.insert(name.clone()) {
            entities.push(name.clone());
        }
    }

    // First-order affinities from direct edges
    for name in &entities {
        if let Ok(rels) = store.get_relationships_for_entity(name) {
            for rel in &rels {
                let other = if rel.src_name == *name { &rel.tgt_name } else { &rel.src_name };
                if hub_set.contains(other.as_str()) || !entity_set.contains(other) { continue; }

                let weight = match rel.relationship_type.as_str() {
                    "CALLS" => {
                        let reverse_exists = rels.iter().any(|r|
                            r.src_name == *other && r.tgt_name == *name && r.relationship_type == "CALLS");
                        if reverse_exists { 1.0 } else { 0.7 }
                    }
                    "IMPORTS" => 0.4,
                    "DEFINES" | "CONTAINS" => 0.3,
                    _ => 0.2,
                };

                let key = if name < other { (name.clone(), other.clone()) } else { (other.clone(), name.clone()) };
                *affinities.entry(key).or_insert(0.0) += weight;
            }
        }
    }

    // Second-order: entities sharing a connection to the same hub get affinity.
    // This restores connectivity lost by hub removal.
    let mut hub_users: HashMap<String, Vec<String>> = HashMap::new();
    for name in &entities {
        if let Ok(rels) = store.get_relationships_for_entity(name) {
            for rel in &rels {
                let other = if rel.src_name == *name { &rel.tgt_name } else { &rel.src_name };
                if hub_set.contains(other.as_str()) {
                    hub_users.entry(other.clone()).or_default().push(name.clone());
                }
            }
        }
    }
    for users in hub_users.values() {
        if users.len() > 50 { continue; } // Skip extremely popular hubs to avoid O(n²) blowup
        for i in 0..users.len() {
            for j in (i + 1)..users.len() {
                let key = if users[i] < users[j] { (users[i].clone(), users[j].clone()) } else { (users[j].clone(), users[i].clone()) };
                *affinities.entry(key).or_insert(0.0) += 0.3;
            }
        }
    }

    Ok((entities, affinities))
}

/// Compute labels for clusters (highest-degree non-test entity in each).
pub fn compute_cluster_labels(
    clusters: &HashMap<String, u32>,
    store: &crate::store::Store,
) -> anyhow::Result<HashMap<u32, String>> {
    let mut cluster_degrees: HashMap<u32, Vec<(String, usize, bool)>> = HashMap::new();
    for (entity, &cluster_id) in clusters {
        let degree = store.get_relationships_for_entity(entity).map(|r| r.len()).unwrap_or(0);
        let is_test = is_test_entity(entity, store);
        cluster_degrees.entry(cluster_id).or_default().push((entity.clone(), degree, is_test));
    }
    let mut labels = HashMap::new();
    for (cluster_id, mut members) in cluster_degrees {
        // Sort: non-test first (false < true), then by degree descending
        members.sort_by(|a, b| a.2.cmp(&b.2).then(b.1.cmp(&a.1)));
        if let Some((name, _, _)) = members.first() {
            labels.insert(cluster_id, name.clone());
        }
    }
    Ok(labels)
}

fn is_test_entity(entity_key: &str, store: &crate::store::Store) -> bool {
    if let Ok(Some(rec)) = store.get_entity(entity_key) {
        if is_test_path(&rec.file_path) {
            return true;
        }
        let meta = rec.metadata_value();
        if let Some(name) = meta.get("name").and_then(|n| n.as_str()) {
            return is_test_entity_name(name);
        }
    }
    false
}

fn is_test_path(path: &str) -> bool {
    path.split('/').any(|seg|
        seg == "tests" || seg == "test" || seg == "__tests__" ||
        seg == "testdata" || seg == "test_data" || seg == "fixtures"
    )
}

fn is_test_entity_name(name: &str) -> bool {
    name.starts_with("test_") || name.starts_with("Test")
}

/// Assign each hub to the cluster that references it most.
pub fn attach_hubs_to_clusters(
    store: &crate::store::Store,
    hub_names: &[String],
    clusters: &HashMap<String, u32>,
) -> anyhow::Result<HashMap<String, u32>> {
    let mut hub_assignments: HashMap<String, u32> = HashMap::new();
    for hub in hub_names {
        let mut cluster_refs: HashMap<u32, usize> = HashMap::new();
        if let Ok(rels) = store.get_relationships_for_entity(hub) {
            for rel in &rels {
                let other = if rel.src_name == *hub { &rel.tgt_name } else { &rel.src_name };
                if let Some(&cluster_id) = clusters.get(other) {
                    *cluster_refs.entry(cluster_id).or_insert(0) += 1;
                }
            }
        }
        if let Some((&best_cluster, _)) = cluster_refs.iter().max_by_key(|(_, count)| *count) {
            hub_assignments.insert(hub.clone(), best_cluster);
        }
    }
    Ok(hub_assignments)
}

/// Absorb small clusters (≤ `min_size` members) into the cluster most connected
/// to them via the full relationship graph. Runs iteratively until stable.
pub fn absorb_small_clusters(
    store: &crate::store::Store,
    clusters: &mut HashMap<String, u32>,
    min_size: usize,
) -> anyhow::Result<()> {
    loop {
        let mut cluster_sizes: HashMap<u32, usize> = HashMap::new();
        for &label in clusters.values() {
            *cluster_sizes.entry(label).or_insert(0) += 1;
        }

        let small_entities: Vec<String> = clusters.iter()
            .filter(|(_, label)| cluster_sizes.get(label).copied().unwrap_or(0) <= min_size)
            .map(|(name, _)| name.clone())
            .collect();

        if small_entities.is_empty() {
            break;
        }

        let mut changed = false;
        for entity in &small_entities {
            let mut cluster_refs: HashMap<u32, usize> = HashMap::new();
            if let Ok(rels) = store.get_relationships_for_entity(entity) {
                for rel in &rels {
                    let other = if rel.src_name == *entity { &rel.tgt_name } else { &rel.src_name };
                    if let Some(&cluster_id) = clusters.get(other) {
                        if cluster_sizes.get(&cluster_id).copied().unwrap_or(0) > min_size {
                            *cluster_refs.entry(cluster_id).or_insert(0) += 1;
                        }
                    }
                }
            }
            if let Some((&best_cluster, _)) = cluster_refs.iter().max_by_key(|(_, count)| *count) {
                let current = clusters[entity];
                if best_cluster != current {
                    clusters.insert(entity.clone(), best_cluster);
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }
    Ok(())
}
