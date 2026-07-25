//! Optimal group→cluster labeling: the assignment problem (2026-07-25).
//!
//! The partitioner's output is LABEL-FREE: groups of entities identified by
//! arbitrary indices. Which REAL cluster hosts which group is a separate,
//! post-hoc decision — and it must minimize migrations: relabeling a group
//! that already sits wholly on cluster A onto cluster B would teleport an
//! entire community for zero objective gain. (Live-observed design concern:
//! under coordinated wave adoption, a greedy labeler can do exactly that.)
//!
//! Formally: given agreement[i][j] = how many members of group i currently
//! live on cluster j, choose an injective labeling group→cluster maximizing
//! total agreement — equivalently minimizing movers, since
//! movers = total − agreement. Greedy (largest group picks its plurality
//! first) is suboptimal: with X = {A:60, B:40} and Y = {A:45, C:5}, greedy
//! gives X→A, Y→C (agreement 65) while X→B, Y→A scores 85 — a whole
//! community's worth of needless migrations. The Hungarian algorithm solves
//! it exactly in O(k³); k = cluster count (4–16), so cost is negligible.

/// Solve max-agreement assignment. `agreement[i][j]` = members of group `i`
/// already on cluster `j`. Returns `label[i] = j`, injective over groups.
/// Requires `agreement` to be square (pad with zeros; see caller).
/// Deterministic: ties resolve toward lower indices.
pub fn max_agreement_labels(agreement: &[Vec<u64>]) -> Vec<usize> {
    let n = agreement.len();
    if n == 0 {
        return Vec::new();
    }
    debug_assert!(agreement.iter().all(|row| row.len() == n), "square matrix");

    // Hungarian (Jonker-style potentials, O(n³)) on COST = max − agreement.
    let max_val = agreement
        .iter()
        .flat_map(|r| r.iter())
        .copied()
        .max()
        .unwrap_or(0) as i64;
    let cost = |i: usize, j: usize| max_val - agreement[i][j] as i64;

    // 1-indexed potentials arrays per the classic formulation.
    let inf = i64::MAX / 2;
    let mut u = vec![0i64; n + 1];
    let mut v = vec![0i64; n + 1];
    let mut p = vec![0usize; n + 1]; // p[j] = row assigned to column j (1-indexed)
    let mut way = vec![0usize; n + 1];

    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![inf; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = inf;
            let mut j1 = 0usize;
            for j in 1..=n {
                if !used[j] {
                    let cur = cost(i0 - 1, j - 1) - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut labels = vec![0usize; n];
    for j in 1..=n {
        if p[j] > 0 {
            labels[p[j] - 1] = j - 1;
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agreement_total(m: &[Vec<u64>], labels: &[usize]) -> u64 {
        labels.iter().enumerate().map(|(i, &j)| m[i][j]).sum()
    }

    #[test]
    fn empty_is_fine() {
        assert!(max_agreement_labels(&[]).is_empty());
    }

    #[test]
    fn identity_when_diagonal_dominates() {
        // Each group already lives on "its" cluster: labeling must be identity
        // (zero migrations), not any permutation.
        let m = vec![
            vec![90, 1, 0, 2],
            vec![0, 75, 3, 1],
            vec![2, 0, 80, 0],
            vec![1, 2, 0, 66],
        ];
        assert_eq!(max_agreement_labels(&m), vec![0, 1, 2, 3]);
    }

    #[test]
    fn beats_greedy_on_the_conflict_case() {
        // THE case greedy loses (see module docs): X={A:60,B:40}, Y={A:45,C:5}.
        // Greedy: X→A, Y→C = 65 agreement. Optimal: X→B, Y→A = 85.
        // (Padded to 3x3 with a zero row for the unused third group slot.)
        let m = vec![
            vec![60, 40, 0], // X over clusters A,B,C
            vec![45, 0, 5],  // Y
            vec![0, 0, 0],   // padding
        ];
        let labels = max_agreement_labels(&m);
        assert_eq!(agreement_total(&m, &labels), 85, "optimal, not greedy");
        assert_eq!(labels[0], 1, "X takes B");
        assert_eq!(labels[1], 0, "Y takes A");
    }

    #[test]
    fn injective_always() {
        let m = vec![vec![10, 10, 10], vec![10, 10, 10], vec![10, 10, 10]];
        let labels = max_agreement_labels(&m);
        let mut seen = std::collections::HashSet::new();
        for &l in &labels {
            assert!(seen.insert(l), "labels must be injective");
        }
    }

    #[test]
    fn deterministic() {
        let m = vec![vec![5, 5, 0], vec![0, 5, 5], vec![5, 0, 5]];
        let a = max_agreement_labels(&m);
        let b = max_agreement_labels(&m);
        assert_eq!(a, b);
    }

    #[test]
    fn brute_force_agreement_on_random_matrices() {
        // Exhaustive check against permutations for k=4: the Hungarian result
        // must achieve the true maximum agreement on every instance.
        fn permutations(k: usize) -> Vec<Vec<usize>> {
            if k == 1 {
                return vec![vec![0]];
            }
            let mut out = Vec::new();
            for p in permutations(k - 1) {
                for pos in 0..=p.len() {
                    let mut q: Vec<usize> = p.iter().map(|&x| x + 1).collect();
                    q.insert(pos, 0);
                    out.push(q);
                }
            }
            out
        }
        // Deterministic pseudo-random matrices.
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % 100
        };
        for _case in 0..50 {
            let m: Vec<Vec<u64>> = (0..4).map(|_| (0..4).map(|_| next()).collect()).collect();
            let labels = max_agreement_labels(&m);
            let got = agreement_total(&m, &labels);
            let best = permutations(4)
                .into_iter()
                .map(|p| agreement_total(&m, &p))
                .max()
                .unwrap();
            assert_eq!(got, best, "must match brute-force optimum on {m:?}");
        }
    }
}
