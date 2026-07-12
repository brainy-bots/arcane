use std::collections::HashMap;
use uuid::Uuid;

/// Resolve per-entity desired assignments into a globally-consistent set that
/// COLLAPSES mutual cross-boundary swaps into co-location.
///
/// A mutual swap is a pair (a, b) where a and b are on different clusters and each
/// wants the other's current cluster:
///   current[a] != current[b]  &&  desired[a] == current[b]  &&  desired[b] == current[a]
/// Left alone this makes a and b cross past each other and never co-locate. We resolve
/// it by co-locating both on the LOWER-Uuid entity's current cluster (deterministic):
/// cancel the lower-id entity's move (it stays put); the higher-id entity's desired
/// already equals the lower's current cluster, so it migrates onto it. Net: one migration,
/// both entities end co-located.
pub fn resolve_convergence(
    current: &HashMap<Uuid, Uuid>,
    desired: &HashMap<Uuid, Uuid>,
) -> HashMap<Uuid, Uuid> {
    let mut resolved = desired.clone();

    // Collect movers: entities where desired differs from current
    let movers: Vec<Uuid> = desired
        .iter()
        .filter_map(|(entity_id, desired_cluster)| {
            if let Some(&current_cluster) = current.get(entity_id) {
                if desired_cluster != &current_cluster {
                    return Some(*entity_id);
                }
            }
            None
        })
        .collect();

    // Check each pair of movers for mutual swaps
    for i in 0..movers.len() {
        for j in (i + 1)..movers.len() {
            let a = movers[i];
            let b = movers[j];

            // Ensure consistent ordering: a < b by Uuid Ord
            let (lower_id, higher_id) = if a < b { (a, b) } else { (b, a) };

            let ca = current.get(&lower_id).unwrap_or(&lower_id);
            let cb = current.get(&higher_id).unwrap_or(&higher_id);
            let da = desired.get(&lower_id).unwrap_or(&lower_id);
            let db = desired.get(&higher_id).unwrap_or(&higher_id);

            // Check for mutual swap: ca != cb && da == cb && db == ca
            if ca != cb && da == cb && db == ca {
                // Cancel the lower-id entity's move; it stays on its current cluster
                resolved.insert(lower_id, *ca);
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutual_swap_collapses() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);

        let mut current = HashMap::new();
        current.insert(a, c1);
        current.insert(b, c2);

        let mut desired = HashMap::new();
        desired.insert(a, c2);
        desired.insert(b, c1);

        let resolved = resolve_convergence(&current, &desired);

        // Both should end up on c1 (lower-id entity's current cluster)
        assert_eq!(resolved.get(&a), Some(&c1), "A should stay on C1");
        assert_eq!(resolved.get(&b), Some(&c1), "B should move to C1");
    }

    #[test]
    fn lower_id_determinism() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);

        let mut current = HashMap::new();
        current.insert(a, c1);
        current.insert(b, c2);

        let mut desired = HashMap::new();
        desired.insert(a, c2);
        desired.insert(b, c1);

        let resolved1 = resolve_convergence(&current, &desired);

        // Try inserting in reverse order to verify determinism
        let mut desired_rev = HashMap::new();
        desired_rev.insert(b, c1);
        desired_rev.insert(a, c2);

        let resolved2 = resolve_convergence(&current, &desired_rev);

        // Should be identical regardless of insertion order
        assert_eq!(resolved1.get(&a), resolved2.get(&a));
        assert_eq!(resolved1.get(&b), resolved2.get(&b));
        assert_eq!(resolved1.get(&a), Some(&c1));
        assert_eq!(resolved2.get(&a), Some(&c1));
    }

    #[test]
    fn non_swap_unaffected() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        let c3 = Uuid::from_u128(300);

        let mut current = HashMap::new();
        current.insert(a, c1);
        current.insert(b, c2);

        let mut desired = HashMap::new();
        desired.insert(a, c2);
        desired.insert(b, c3); // B wants C3, not C1 (not a swap)

        let resolved = resolve_convergence(&current, &desired);

        // A should still want C2, B should still want C3 (no swap detected)
        assert_eq!(resolved.get(&a), Some(&c2), "A should still want C2");
        assert_eq!(resolved.get(&b), Some(&c3), "B should still want C3");
    }

    #[test]
    fn third_cluster_move_unaffected() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        let c3 = Uuid::from_u128(300);

        let mut current = HashMap::new();
        current.insert(a, c1);
        current.insert(b, c2);

        let mut desired = HashMap::new();
        desired.insert(a, c3); // A wants C3, not C2 (not a swap with B)
        desired.insert(b, c1);

        let resolved = resolve_convergence(&current, &desired);

        // A should still want C3, B should still want C1 (no swap detected)
        assert_eq!(resolved.get(&a), Some(&c3), "A should still want C3");
        assert_eq!(resolved.get(&b), Some(&c1), "B should still want C1");
    }

    #[test]
    fn no_movers() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);

        let mut current = HashMap::new();
        current.insert(a, c1);
        current.insert(b, c2);

        let mut desired = HashMap::new();
        desired.insert(a, c1); // A stays on C1
        desired.insert(b, c2); // B stays on C2

        let resolved = resolve_convergence(&current, &desired);

        // Everything stays put
        assert_eq!(
            resolved, desired,
            "resolved should equal desired when no movers"
        );
    }
}
