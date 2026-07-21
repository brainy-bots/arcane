//! Scale and property tests for the graph partitioner and refinement (epic #245).
//!
//! These guard the design's **load-bearing complexity and quality claims** that the
//! example-based unit tests cannot: that partitioning is near-linear in the number of
//! entities (design `meta-control-layer.md` §6 "Memory: O(N) ... near-linear per decision"),
//! that it produces balanced partitions with a small boundary cut on bounded-degree graphs
//! (the "geometry → small separators" premise the whole thesis rests on), and that both
//! stay deterministic at scale.
//!
//! Sizes are kept CI-friendly (a few thousand entities) so the suite runs in well under a
//! second in release and a few seconds in debug. The runtime-ratio assertions use generous
//! bounds so they catch a return to quadratic blow-up (the O(N^2) regression these tests were
//! written to pin down and fix) without being flaky on a loaded CI box.

use arcane_affinity::interaction_graph::Colocation;
use arcane_affinity::partition::{
    GreedyGrowthPartitioner, IPartitioner, PartitionInput, WeightedEdge,
};
use arcane_affinity::refinement::{refine, RefineConfig};
use std::time::Instant;
use uuid::Uuid;

/// Tiny deterministic PRNG (splitmix64). No external deps; reproducible across runs/platforms.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn uuid_n(n: usize) -> Uuid {
    Uuid::from_u128(n as u128 + 1)
}

/// A bounded-degree geometric graph: N entities on a ~sqrt(N) grid, each linked to its right
/// and down grid neighbor with a random Soft weight. Degree is bounded by construction (<=4),
/// which is the "bounded interaction degree" the thesis assumes for physical MMO interaction.
fn gen_grid_graph(n: usize, num_partitions: usize, seed: u64) -> PartitionInput {
    let mut rng = Rng::new(seed);
    let side = (n as f64).sqrt().ceil() as usize;
    let entities: Vec<Uuid> = (0..n).map(uuid_n).collect();
    let mut edges = Vec::new();
    for i in 0..n {
        if (i % side) + 1 < side && i + 1 < n {
            edges.push(WeightedEdge {
                a: uuid_n(i),
                b: uuid_n(i + 1),
                weight: 1.0 + rng.below(5) as f64,
                colocation: Colocation::Soft,
            });
        }
        if i + side < n {
            edges.push(WeightedEdge {
                a: uuid_n(i),
                b: uuid_n(i + side),
                weight: 1.0 + rng.below(5) as f64,
                colocation: Colocation::Soft,
            });
        }
    }
    PartitionInput {
        entities,
        edges,
        num_partitions,
        capacity: 0,
    }
}

/// Total interaction weight (sum of all Soft edge weights), used to bound the boundary cut.
fn total_soft_weight(edges: &[WeightedEdge]) -> f64 {
    edges
        .iter()
        .filter(|e| e.colocation == Colocation::Soft)
        .map(|e| e.weight)
        .sum()
}

/// Partition a graph of `n` entities and return elapsed seconds.
fn time_partition(n: usize, num_partitions: usize, seed: u64) -> f64 {
    let input = gen_grid_graph(n, num_partitions, seed);
    let t = Instant::now();
    let part = GreedyGrowthPartitioner::new().partition(&input);
    let secs = t.elapsed().as_secs_f64();
    // Touch the result so the optimizer can't elide the work.
    assert_eq!(part.assignment().len(), n);
    secs
}

/// PROPERTY: every entity is assigned to exactly one valid partition.
#[test]
fn partition_assigns_every_entity_once() {
    let input = gen_grid_graph(2000, 8, 1);
    let part = GreedyGrowthPartitioner::new().partition(&input);
    assert_eq!(
        part.assignment().len(),
        2000,
        "every entity must be assigned"
    );
    for &p in part.assignment().values() {
        assert!(p < 8, "partition index {} out of range", p);
    }
}

/// PROPERTY: with a per-partition CAPACITY set, partitions are balanced (no partition exceeds
/// capacity, and load is spread across partitions rather than piled into one).
///
/// NOTE ON DESIGN: with `capacity = 0` (unbounded) the greedy partitioner deliberately **packs
/// everything into one partition** — this is the design's "pack maximally, then split only under
/// resource pressure" policy (`meta-control-layer.md` §5), NOT a bug. Balance/spread across nodes
/// is therefore driven by capacity (the resource ceiling), which is what this test exercises.
/// (The Manager currently passes `capacity = 0`; wiring a real per-node capacity so multi-node
/// spread actually happens is tracked as follow-on work — see the test module docs.)
#[test]
fn partition_is_balanced_under_capacity() {
    let n = 4000;
    let k = 8;
    let mut input = gen_grid_graph(n, k, 7);
    // Give each partition room for ~1.5x the ideal share so growth must spill across partitions.
    input.capacity = (n / k) * 3 / 2; // 750
    let part = GreedyGrowthPartitioner::new().partition(&input);

    let mut sizes = vec![0usize; k];
    for &p in part.assignment().values() {
        sizes[p] += 1;
    }
    let max = *sizes.iter().max().unwrap();
    let assigned: usize = sizes.iter().sum();

    // No partition may exceed capacity.
    assert!(
        max <= input.capacity,
        "capacity violated: sizes={:?} cap={}",
        sizes,
        input.capacity
    );
    // With capacity < n, growth is forced to spread: at least half the partitions are used.
    let used = sizes.iter().filter(|&&s| s > 0).count();
    assert!(
        used >= k / 2,
        "capacity-bounded partition did not spread load: sizes={:?}",
        sizes
    );
    // Everyone that fits is placed (capacity*k = 6000 >= 4000, so all 4000 fit).
    assert_eq!(
        assigned, n,
        "all entities should be placed: sizes={:?}",
        sizes
    );
}

/// PROPERTY: with `capacity = 0` (unbounded) the partitioner packs into a single partition —
/// documenting the "pack maximally" policy explicitly so a future change to it is a conscious one.
#[test]
fn partition_unbounded_packs_into_one() {
    let n = 2000;
    let input = gen_grid_graph(n, 8, 7); // capacity defaults to 0 in gen_grid_graph
    let part = GreedyGrowthPartitioner::new().partition(&input);
    let mut sizes = vec![0usize; 8];
    for &p in part.assignment().values() {
        sizes[p] += 1;
    }
    let used = sizes.iter().filter(|&&s| s > 0).count();
    // This is the current "pack maximally, split under pressure" behavior. If this ever changes
    // to spread load without a capacity signal, update this test deliberately.
    assert_eq!(
        used, 1,
        "unbounded partition is expected to pack into ONE partition (pack-maximally policy); \
         got sizes={:?}",
        sizes
    );
}

/// PROPERTY (the thesis premise): the boundary cut is a SMALL fraction of total interaction
/// weight on a bounded-degree geometric graph. If the min-cut premise ("geometry → small
/// separators") holds, most weight stays inside partitions.
#[test]
fn partition_cut_is_small_fraction_of_total() {
    let n = 4000;
    let k = 8;
    let mut input = gen_grid_graph(n, k, 11);
    // Force a real multi-partition split (capacity < n) so there is an actual boundary to measure.
    // Without this, unbounded packing puts everyone in one partition and the cut is trivially 0.
    input.capacity = (n / k) * 3 / 2;
    let part = GreedyGrowthPartitioner::new().partition(&input);
    let cut = part.cut_cost(&input.edges);
    let total = total_soft_weight(&input.edges);
    assert!(total > 0.0);
    // Sanity: the split actually used multiple partitions (otherwise the fraction is meaningless).
    let mut sizes = vec![0usize; k];
    for &p in part.assignment().values() {
        sizes[p] += 1;
    }
    assert!(
        sizes.iter().filter(|&&s| s > 0).count() >= 2,
        "expected a real multi-partition split: {:?}",
        sizes
    );
    let frac = cut / total;
    // A capacity-forced 8-way partition of a grid still cuts only the perimeter between regions,
    // which is O(sqrt(area)) per region — a modest fraction of the O(area) total. Greedy growth
    // without a global min-cut is not optimal, so we allow a generous bound; the point is it is
    // NOT cutting most of the graph (which a random assignment would).
    assert!(
        frac < 0.50,
        "boundary cut too large: cut={:.1} total={:.1} fraction={:.3} sizes={:?}",
        cut,
        total,
        frac,
        sizes
    );
}

/// PROPERTY: refinement never worsens the cut and is cheap on the partitioner's own output
/// (the realistic Manager path: partition -> refine on an already-good partition).
#[test]
fn refine_does_not_worsen_at_scale() {
    let input = gen_grid_graph(3000, 8, 13);
    let part = GreedyGrowthPartitioner::new().partition(&input);
    let before = part.cut_cost(&input.edges);
    let refined = refine(&part, &input.edges, 8, &RefineConfig::default());
    let after = refined.cut_cost(&input.edges);
    assert!(
        after <= before + 1e-9,
        "refinement worsened the cut: before={:.1} after={:.1}",
        before,
        after
    );
    assert_eq!(
        refined.assignment().len(),
        3000,
        "refinement must not drop entities"
    );
}

/// PROPERTY: partitioning is DETERMINISTIC at scale (same input -> byte-identical assignment).
#[test]
fn partition_deterministic_at_scale() {
    let input = gen_grid_graph(3000, 8, 17);
    let p1 = GreedyGrowthPartitioner::new().partition(&input);
    let p2 = GreedyGrowthPartitioner::new().partition(&input);
    assert_eq!(
        p1.assignment(),
        p2.assignment(),
        "partition must be deterministic at scale"
    );
}

/// COMPLEXITY GUARD: partitioning is near-linear, not quadratic. This is the regression guard
/// for the O(N^2) bug these tests were written to find and fix.
///
/// We compare the time to partition a graph of size 8N against size N. Under O(N) the ratio is
/// ~8; under O(N^2) it would be ~64. We assert the ratio is well under quadratic (< 25), which
/// leaves generous slack for a noisy CI box and constant-factor/measurement effects while still
/// failing hard if quadratic behavior returns.
#[test]
fn partition_scales_near_linearly() {
    let small = 1000usize;
    let big = 8000usize;

    // Warm up (allocator, caches) so the first timing isn't penalized.
    let _ = time_partition(small, 8, 99);

    // Take the best of a few runs to reduce noise from OS scheduling.
    let t_small = (0..3)
        .map(|_| time_partition(small, 8, 99))
        .fold(f64::INFINITY, f64::min);
    let t_big = (0..3)
        .map(|_| time_partition(big, 8, 99))
        .fold(f64::INFINITY, f64::min);

    // If both are sub-millisecond the ratio is dominated by noise; only assert when there is
    // enough signal (the big case takes a meaningful amount of time).
    if t_big < 1e-4 {
        // Too fast to measure reliably; the fact that 8000 entities partitions in <0.1ms is
        // itself strong evidence of near-linearity. Pass.
        return;
    }

    let ratio = t_big / t_small.max(1e-9);
    assert!(
        ratio < 25.0,
        "partition scaling looks super-linear: t(1k)={:.4}ms t(8k)={:.4}ms ratio={:.1} \
         (expected ~8 for O(N), ~64 for O(N^2))",
        t_small * 1000.0,
        t_big * 1000.0,
        ratio
    );
}

/// SANITY: a Hard-edge clique is never split even at scale, and the cut stays finite.
#[test]
fn hard_clique_never_split_at_scale() {
    let n = 2000;
    let mut input = gen_grid_graph(n, 8, 23);
    // Add a Hard chain linking 50 entities across the grid into one rigid component.
    for i in 0..50 {
        input.edges.push(WeightedEdge {
            a: uuid_n(i * 37 % n),
            b: uuid_n((i * 37 + 37) % n),
            weight: 1.0,
            colocation: Colocation::Hard,
        });
    }
    let part = GreedyGrowthPartitioner::new().partition(&input);
    let cut = part.cut_cost(&input.edges);
    assert!(
        cut.is_finite(),
        "a Hard edge was cut (infinite cost) — hard clique was split"
    );
}
