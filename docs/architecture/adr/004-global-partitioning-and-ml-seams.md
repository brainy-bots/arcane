# ADR-004: Global graph partitioning is the clustering core; per-entity scoring is refinement; ML enters only at policy seams

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-07-12 |
| **Supersedes** | The per-entity greedy scoring model as the *primary* clustering algorithm (introduced in `arcane-affinity` `AffinityEngine`, wired into the Manager by epic #208) |
| **Related** | [meta-control-layer.md](../meta-control-layer.md) §5 (clustering policy), §6 (Manager scalability), §12.1 (partitioning algorithm) · [interaction-edge-taxonomy.md](../interaction-edge-taxonomy.md) · [interface-iclusteringmodel.md](../interface-iclusteringmodel.md) |
| **Tracking** | Initiative `initiative/meta-control-layer`; follow-on to epic #208 |

## Context

Epic #208 wired the Manager's evaluation loop to `AffinityEngine::compute_entity_assignments`, which scores **one entity at a time**: for each entity independently, compute its best cluster and migrate if the gain exceeds a threshold. A verification probe during #208 revealed the failure mode this design produces directly: two heavily-interacting entities A (in C1) and B (in C2) each independently conclude the *other's* cluster is better, so both migrate and **cross past each other** — a symmetric swap that never co-locates them. The `convergence.rs` pass added in #208.1 detects and cancels that specific 2-cycle.

That patch is a symptom fix. The root cause is a modeling error: **cluster assignment is a global, coupled optimization, and we were solving it as N independent local decisions.** Moving entity A changes B's incentive; deciding them independently double-counts the shared (symmetric) interaction edge. The convergence pass only unwinds 2-cycles; a 3-cycle (A wants B's cluster, B wants C's, C wants A's) slips through untouched.

Three questions from the founder review (2026-07-12) sharpened the correction:

1. Should we decide player-by-player at all, or partition the full graph and let assignments follow the partition?
2. The graph is already made of predictions (`p` at horizon `T`) — is it worth predicting the *next* graph?
3. Can ML beat a classical multilevel partitioner (METIS), and where does ML actually help?

## Decision

### 1. The global graph partitioner is the clustering core.

Cluster assignment is defined as **balanced graph partitioning** of the predicted interaction graph: partition entities so the sum of interaction-edge weights crossing partition boundaries is minimized, subject to per-node capacity constraints. **The partition IS the assignment.** Entities move to match the partition; we do not decide entity-by-entity in isolation.

This is not a preference; it is forced by the cost identity from the design doc: **cut cost = boundary size = replication cost = per-node compute.** Minimizing the cut and minimizing the work each node does are the *same* objective. Only a global solve finds the cheap seam (the thin front line of a battle) instead of thrashing individual entities across it.

The edge taxonomy (#211) feeds the partitioner directly: `Joint` edges carry **infinite** weight (never cut — can block a split, forcing vertical scale), `SharedDeterministic` edges carry **zero** weight (free to cut), everything else contributes its aggregate weight to the cut cost.

### 2. Two-tier solve: periodic global partition + per-frame incremental refinement.

We cannot re-partition the whole graph every frame (too expensive), and we do not need to (the design doc's incrementality argument). The structure:

- **Global re-partition** (~1s cadence, over the *active subgraph* only): multilevel partitioning (METIS-class — coarsen → partition the tiny coarsened graph → refine while uncoarsening, ~O(V+E)). This sets the skeleton.
- **Incremental refinement** (per-frame): move only entities whose optimal side flipped (`O(changed edges)`), using **Fiduccia-Mattheyses-style pair/vertex moves with gain recomputation**. Because FM recomputes each vertex's gain *after every move*, it never produces the independent-greedy swap by construction.

### 3. Per-entity scoring is demoted to the refinement layer, upgraded to pair moves.

The per-entity path is **not deleted** — it becomes the fast, reactive, anytime refinement layer *under* the global solve, with two changes:

- It runs **on top of** a global partition, not standalone. Standalone per-entity greedy is the retired design.
- Its move primitive is upgraded from **single-vertex** to **pair moves** (the Kernighan-Lin swap primitive). Evaluating a pair (a, b) jointly considers the four real options — both in P1, both in P2, split a|b, split b|a — using the shared symmetric edge exactly once. This eliminates the 2-cycle swap by construction and makes `convergence.rs` obsolete (it will be removed once pair-move refinement lands).

Retained benefits of the local layer that justify keeping it: incrementality/latency (a single spawn shouldn't trigger a world re-partition), the anytime property (always emits *a* decision under budget pressure, degrades gracefully), and shardability (needs only a neighborhood).

### 4. ML enters only at policy seams, never at mechanism. Rule-based baseline is permanent.

Every place ML can plug in is a **policy/prediction** decision behind a trait, with a **rule-based implementation as the permanent baseline** (the C4 scientific control, the safe-mode fallback, and the shadow-comparison target). Correctness **mechanisms** (migration executor, single-writer ownership resolution) are deterministic and get **no ML seam** — a non-deterministic component must never sit on a correctness invariant.

Discipline: **never add a trait that isn't already exercised by a real rule-based implementation.** The seam is defined now; the ML implementation is deferred until data and measured need exist.

#### Why the mechanism/policy line falls where it does (runtime-assurance framing)

The line is **not** "keep ML away from migration" — ML belongs in the *decision* to migrate (when/where an entity moves), which is exactly the partitioner seam above, a control/optimization problem where learned controllers are appropriate. The line separates that **decision** from the **enforcement** of exactly-once ownership. This is the standard **runtime-assurance / safety-filter (Simplex) architecture** from safety-critical control: a learned controller proposes actions; a small, verified, deterministic layer admits or clamps them to keep the system in a safe set. So this is a specific, well-established form of "AI for control," not an exception to it.

`resolve_authoritative` (the enforcement layer) must be deterministic for three reasons that come from the *problem's structure*, not from conservatism about ML:

1. **It is a distributed-agreement problem, not an optimization.** It runs independently on the source and destination clusters, which do not communicate at resolution time. Exactly-once ownership holds only because both sides compute the *identical deterministic function* on the *identical inputs* (the shared flip record + shared tick) and therefore reach the same conclusion at the same tick — agreement-by-determinism. A learned, stochastic, or even hardware-nondeterministic-float function could make the two sides disagree for a window, which is precisely the double-owner (corruption) / zero-owner (dropped-write) failure. This is why consensus (Paxos/Raft) uses deterministic state machines, not learned policies.
2. **It enforces a hard invariant, not a soft objective.** ML-in-control optimizes soft objectives where "mostly optimal" is fine. Ownership is binary correctness — exactly-one-owner is true or false at every tick, with no partial credit; a rare violation is state corruption, not degraded quality. ML gives *statistical* guarantees on a training distribution; a consensus invariant needs a *logical* guarantee over all inputs, including unseen ones.
3. **It must be verifiable.** `if tick < effective_tick { from } else { to }` is provable by exhaustive case analysis (what the exactly-once integration test does); a network can only be sample-tested. Safety-critical standards (DO-178C, ISO 26262) reflect exactly this — an unbounded network is never the *sole* safety mechanism; it is wrapped in a verified monitor.

Corrected statement of the boundary: **ML proposes ownership changes; a deterministic, verified rule enforces exactly-once.** That is the runtime-assurance architecture, i.e. the mature form of AI-for-control, applied here.

Where ML helps, ranked, and explicitly where it does NOT:

- **Predictor `p` (highest leverage).** Better `p` → better graph → better partition; everything inherits. The shipped heuristic is the C4 baseline the model must beat.
- **Region selection (the strong, novel use).** ML predicts *where the contested boundary will be* — a battle forming, a raid converging — and pulls that region into the active subgraph *before* it shows up as boundary edges. This is **attentional** (where to focus the solver), not **regenerative** (regenerating all weights). It shrinks the problem the partitioner sees; it does not compete with the partitioner.
- **Partition warm-start.** A GNN produces a candidate partition in one fixed-latency forward pass; classical FM polishes it and provides the guarantee.
- **True-objective optimization.** METIS minimizes edge cut; our real objective is staleness `Σ p·dynamism·age` under *heterogeneous, capacitated* nodes. ML can be trained end-to-end on the real objective and constraints a cut-minimizer only approximates.

Explicitly **rejected** framings:
- **"Predict the next graph."** The graph's weights already *are* `p` at horizon `T` — forecasting a forecast double-counts, or is equivalent to increasing `T`. The residual value (partition stability over time) is a **temporal-coherence regularizer on the solver**, not a separate prediction model.
- **"ML replaces METIS on cut quality/speed."** Multilevel partitioning is near-linear and decades-refined; learned partitioners have not decisively beaten it on quality-at-speed for the pure cut objective. ML wraps METIS (region selection, warm-start, true objective); it does not replace it.

## Consequences

**Positive:**
- The symmetric-swap class of bug is eliminated by construction (pair moves + global solve), not patched.
- The clustering objective is now explicitly the replication/compute cost, so partition quality is directly measurable against the paper's cost model.
- ML has a bounded, well-typed set of insertion points (policy seams), each with a rule-based control for measurement — swappability and the C2/C4 measurement harness are the same feature.

**Negative / costs:**
- Requires implementing a multilevel partitioner (or integrating one) — the "deferred" §12.1 item is re-scoped as the **core**, not a nice-to-have.
- Two-tier solve adds a scheduling concern (when to full-re-partition vs incrementally refine) and a stability requirement (predictions must change smoothly frame-to-frame or incremental refinement degrades toward full re-cut).

**Migration path (issues to be filed as the #208 follow-on):**
1. `IPartitioner` trait + rule-based multilevel/greedy-growth implementation; Manager consumes a partition, diffs old vs new, emits moves through the existing #207 executor (unchanged).
2. Pair-move (KL) refinement layer replacing single-entity greedy; delete `convergence.rs`.
3. `IRegionSelector` trait + rule-based (boundary + spatial expansion) implementation; ML impl deferred.
4. `IPartitioner` ML warm-start seam; deferred impl.

The migration executor (#207), rate field (#210), predictor (#209), and edge taxonomy (#211) are **unaffected** — they consume/produce assignments and edge weights and do not care whether a greedy scorer, a multilevel partitioner, or a GNN produced the partition.
