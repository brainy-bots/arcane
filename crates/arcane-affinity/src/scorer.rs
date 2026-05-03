use crate::{config::AffinityConfig, interaction_graph::InteractionGraph};
use arcane_core::types::Vec2;
use std::collections::HashMap;
use uuid::Uuid;

/// Result of scoring an entity against all clusters.
pub struct ScoringResult {
    pub best_cluster: Uuid,
    pub best_score: f64,
    pub current_score: f64,
}

/// Compute the affinity score of `entity` for each cluster and return the best assignment.
///
/// score(E, C) = interaction_score(E, C) + spatial_score(E, C)
///
/// interaction_score = sum of interaction_weight(E, F) for all F in C
/// spatial_score     = spatial_weight / (1.0 + distance(E_pos, C_centroid))
///
/// A soft capacity penalty is applied when a cluster exceeds capacity_soft_limit_fraction.
#[allow(clippy::too_many_arguments)]
pub fn score_entity(
    entity: Uuid,
    entity_pos: Vec2,
    current_cluster: Uuid,
    cluster_members: &HashMap<Uuid, Vec<Uuid>>,
    cluster_centroids: &HashMap<Uuid, Vec2>,
    cluster_sizes: &HashMap<Uuid, usize>,
    interaction_graph: &InteractionGraph,
    config: &AffinityConfig,
) -> ScoringResult {
    // Build a quick lookup: entity → cluster for interaction scoring
    let entity_cluster: HashMap<Uuid, Uuid> = cluster_members
        .iter()
        .flat_map(|(cid, members)| members.iter().map(move |&eid| (eid, *cid)))
        .collect();

    let mut best_cluster = current_cluster;
    let mut best_score = f64::NEG_INFINITY;
    let mut current_score = f64::NEG_INFINITY;

    for (cluster_id, centroid) in cluster_centroids {
        let interaction_score: f64 = interaction_graph
            .neighbors(entity)
            .filter_map(|(neighbor, weight)| {
                if entity_cluster.get(&neighbor) == Some(cluster_id) {
                    Some(weight)
                } else {
                    None
                }
            })
            .sum();

        let dx = entity_pos.x - centroid.x;
        let dy = entity_pos.y - centroid.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let spatial_score = config.spatial_weight / (1.0 + dist);

        let mut score = interaction_score + spatial_score;

        // Soft capacity penalty
        if config.max_entities_per_cluster > 0 {
            let size = cluster_sizes.get(cluster_id).copied().unwrap_or(0);
            let soft_limit = (config.max_entities_per_cluster as f64
                * config.capacity_soft_limit_fraction) as usize;
            if size > soft_limit {
                let overflow = (size - soft_limit) as f64;
                let max_overflow = (config.max_entities_per_cluster - soft_limit).max(1) as f64;
                let penalty = 1.0 - (overflow / max_overflow);
                score *= penalty.max(0.1);
            }
        }

        if score > best_score {
            best_score = score;
            best_cluster = *cluster_id;
        }
        if *cluster_id == current_cluster {
            current_score = score;
        }
    }

    // If current_cluster wasn't in centroids (shouldn't happen, but be safe)
    if current_score == f64::NEG_INFINITY {
        current_score = 0.0;
    }

    ScoringResult {
        best_cluster,
        best_score,
        current_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn vec2(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    type ClusterMaps = (
        HashMap<Uuid, Vec<Uuid>>,
        HashMap<Uuid, Vec2>,
        HashMap<Uuid, usize>,
    );

    fn build_test_clusters(assignments: &[(Uuid, Uuid, Vec2)]) -> ClusterMaps {
        let mut members: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut centroids: HashMap<Uuid, Vec2> = HashMap::new();
        let mut sizes: HashMap<Uuid, usize> = HashMap::new();
        let mut cluster_pos_sum: HashMap<Uuid, (f64, f64, usize)> = HashMap::new();

        for &(entity, cluster, pos) in assignments {
            members.entry(cluster).or_default().push(entity);
            *sizes.entry(cluster).or_insert(0) += 1;
            let e = cluster_pos_sum.entry(cluster).or_insert((0.0, 0.0, 0));
            e.0 += pos.x;
            e.1 += pos.y;
            e.2 += 1;
        }
        for (cid, (sx, sy, n)) in &cluster_pos_sum {
            centroids.insert(*cid, vec2(sx / *n as f64, sy / *n as f64));
        }
        (members, centroids, sizes)
    }

    #[test]
    fn interaction_dominated_scoring() {
        let e = uuid(1);
        let c1 = uuid(10);
        let c2 = uuid(11);
        let f1 = uuid(2);
        let f2 = uuid(3);

        let assignments = [
            (e, c1, vec2(0.0, 0.0)),
            (f1, c1, vec2(1.0, 0.0)),
            (f2, c2, vec2(100.0, 0.0)),
        ];
        let (members, centroids, sizes) = build_test_clusters(&assignments);

        let mut graph = InteractionGraph::new();
        graph.record_interaction(e, f2, 10.0); // heavy interaction with c2 entity

        let config = AffinityConfig {
            spatial_weight: 0.0, // pure interaction
            ..AffinityConfig::default()
        };
        let result = score_entity(
            e,
            vec2(0.0, 0.0),
            c1,
            &members,
            &centroids,
            &sizes,
            &graph,
            &config,
        );

        assert_eq!(result.best_cluster, c2);
    }

    #[test]
    fn spatial_fallback_no_interactions() {
        let e = uuid(1);
        let c1 = uuid(10);
        let c2 = uuid(11);
        let f1 = uuid(2);
        let f2 = uuid(3);

        let assignments = [(f1, c1, vec2(1000.0, 0.0)), (f2, c2, vec2(5.0, 0.0))];
        let (members, centroids, sizes) = build_test_clusters(&assignments);

        let graph = InteractionGraph::new(); // no interactions
        let config = AffinityConfig::default();

        // Entity at (0,0) — closer to c2 centroid at (5,0) than c1 at (1000,0)
        let result = score_entity(
            e,
            vec2(0.0, 0.0),
            c1,
            &members,
            &centroids,
            &sizes,
            &graph,
            &config,
        );
        assert_eq!(result.best_cluster, c2);
    }

    #[test]
    fn capacity_penalty_reduces_score() {
        let e = uuid(1);
        let c1 = uuid(10);
        let c2 = uuid(11);

        let members_c1: Vec<Uuid> = (2..12).map(uuid).collect();
        let mut members: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        members.insert(c1, members_c1);
        members.insert(c2, vec![uuid(20)]);
        let centroids: HashMap<Uuid, Vec2> = [(c1, vec2(0.0, 0.0)), (c2, vec2(0.0, 0.0))]
            .into_iter()
            .collect();
        let sizes: HashMap<Uuid, usize> = [(c1, 10), (c2, 1)].into_iter().collect();

        let graph = InteractionGraph::new();
        let config = AffinityConfig {
            max_entities_per_cluster: 10,
            capacity_soft_limit_fraction: 0.8,
            spatial_weight: 1.0, // pure spatial (same centroid, so equal)
            ..AffinityConfig::default()
        };

        let result = score_entity(
            e,
            vec2(0.0, 0.0),
            c1,
            &members,
            &centroids,
            &sizes,
            &graph,
            &config,
        );
        // c1 at capacity, c2 has room — c2 should score better despite same spatial distance
        assert_eq!(result.best_cluster, c2);
    }

    #[test]
    fn capacity_penalty_never_fully_zeros_score() {
        let e = uuid(1);
        let c1 = uuid(10);

        let members: HashMap<Uuid, Vec<Uuid>> = [(c1, vec![uuid(2)])].into_iter().collect();
        let centroids: HashMap<Uuid, Vec2> = [(c1, vec2(0.0, 0.0))].into_iter().collect();
        let sizes: HashMap<Uuid, usize> = [(c1, 100)].into_iter().collect(); // way over limit

        let graph = InteractionGraph::new();
        let config = AffinityConfig {
            max_entities_per_cluster: 10,
            capacity_soft_limit_fraction: 0.8,
            spatial_weight: 1.0,
            ..AffinityConfig::default()
        };

        let result = score_entity(
            e,
            vec2(0.0, 0.0),
            c1,
            &members,
            &centroids,
            &sizes,
            &graph,
            &config,
        );
        assert!(result.best_score > 0.0);
    }
}
