# Interaction-edge taxonomy

| | |
|---|---|
| **Purpose** | Canonical vocabulary of interaction-edge kinds in the clusterer and interaction graph. Each kind defines its trigger, co-location requirement, and cut-cost implication. |
| **Related** | [meta-control-layer.md](meta-control-layer.md) §3 (physics-coupling) · [four-bucket-state-model.md](four-bucket-state-model.md) (state buckets) · `crates/arcane-affinity/src/config.rs` (edge weights) |
| **Status** | Reference — definitive for edge kind semantics. |

---

## Edge kinds: canonical table

| Edge kind | Trigger | Default weight | Co-location requirement | Cut cost implication |
|---|---|---|---|---|
| **Proximity** | Within `proximity_radius` (50.0) per tick | `weight_proximity_per_tick` (0.1) | Soft (decays frame-to-frame) | Bandwidth if cut; replicate at reduced rate |
| **GameAction** | Attack / heal / trade / game mechanic | `weight_game_action` (2.0) | Strong (same cluster preferred) | Bandwidth + latency if cut; high replication cost |
| **PartyMember** | Shared party membership | `weight_party_member` (5.0) | Prefer / keep-whole (clique) | Bandwidth + UX cost if cut; party split incurs high replication |
| **GuildMember** | Shared guild membership | `weight_guild_member` (1.0) | Soft (social affinity) | Bandwidth if cut; minor replication cost |
| **Collision** | Rapier physics contact (touching) | `weight_collision` (1.0) | Physics-coupling (must co-locate or simulate in sync) | Correctness if cut without sync; physics divergence |
| **PhysicsImpulse** | One-way knockback / force A→B | N/A (physics-coupling) | Physics-coupling, **directed** (A→B; asymmetric) | Correctness if cut; impulse must not cross cluster boundary |
| **Joint** | Rapier joint (revolute, prismatic, ball, etc.) | N/A (uncuttable) | **HARD — must co-locate** (a cross-cluster joint is invalid) | **Infinite cost if cut** — clusterer must never cut a Joint edge; min-cut treats it as infinite weight |
| **SharedDeterministic** | Shared projectile seed / destruction / random seed | N/A (cut-free) | **Cut-free — replicate as seed, no co-location needed** | **Zero cost if cut** — both sides replicate the seed, compute locally, no bandwidth penalty |

---

## Per-kind detail

### Proximity

**Trigger:** Entities within `proximity_radius` at any frame.

**Weight:** `weight_proximity_per_tick` (0.1) — the lightest edge.

**Co-location:** Soft; proximity is transient and decays naturally (moving apart drops weight below GC threshold).

**Cut-cost:** Bandwidth. If cut, the boundary edge requires full-rate replication from the owning cluster to maintain correct positions for hit tests and AI. Proximity edges are the most common; minimizing boundary density is the core clustering objective.

**Rationale:** Spatial proximity is the primary driver of co-simulation. Players standing near each other will likely interact next; the predicted graph's min-cut exploits this.

---

### GameAction

**Trigger:** Attack, heal, trade, or other explicit game mechanic targeting.

**Weight:** `weight_game_action` (2.0) — 2× proximity.

**Co-location:** Strong; players in combat should stay on the same cluster.

**Cut-cost:** Bandwidth + latency. A cut game-action edge forces replication of both entities' state and game action confirmations across the cluster boundary. Unacceptable latency for real-time combat adjudication.

**Rationale:** Game actions are intentional, high-frequency, low-latency-sensitive. Same-cluster co-location is a quality-of-life requirement.

---

### PartyMember

**Trigger:** Shared party membership (registered in the game's party system).

**Weight:** `weight_party_member` (5.0) — the highest game-semantic weight.

**Co-location:** Prefer / keep-whole. Party members should form a clique; splitting a party across clusters is undesirable.

**Cut-cost:** Bandwidth + UX cost. Party members stay aware of each other's state (health, status effects, resources); cutting the edge means replicating all party members' state across the boundary and incurs high replication overhead. More subtly, UX expectations (instant party chat, shared quest markers) are strained by cross-cluster latency.

**Rationale:** Party is a strong social and mechanical unit in group-based games. Keeping parties co-located is a key clustering heuristic.

---

### GuildMember

**Trigger:** Shared guild membership (loosely-affiliated group in the game's guild system).

**Weight:** `weight_guild_member` (1.0) — same as collision (physics-coupling) but distinct semantics.

**Co-location:** Soft; guilds are large and geographically dispersed. No co-location requirement; edges decay if guild members drift far apart or stop interacting.

**Cut-cost:** Bandwidth. If cut, guild-member edge contributes linearly to boundary size. Guild chat and shared guild events may incur some replication, but it is non-critical.

**Rationale:** Guilds are social affiliations and should be _weakly_ preferred for clustering, but they are not a hard constraint like party or physics.

---

### Collision

**Trigger:** Rapier physics engine detects bodies in contact (touching, not penetrating; Rapier's `is_in_contact()` flag true).

**Weight:** `weight_collision` (1.0) — same as guild; physics-coupling weight.

**Co-location:** Physics-coupling (must co-locate or simulate in sync). Two entities in collision must either live on the same cluster or be in a carefully synchronized replication loop.

**Cut-cost:** Correctness if cut without synchronization. If the edge is cut and both clusters simulate independently, collisions diverge: cluster A may allow a pass-through that cluster B prevents. **The clusterer must treat all collision edges with high weight to avoid cuts**, or the game physics becomes non-deterministic.

**Rationale:** Rapier contacts are an internal physics invariant; violating them on a per-cluster basis breaks game rules. Same-cluster co-location is the safe choice; cross-cluster simulation requires explicit bidirectional sync (not part of this edge taxonomy, see ADR-002).

---

### PhysicsImpulse (NEW)

**Trigger:** One-way knockback, force, or impulse applied by entity A to entity B (e.g., A casts a spell that knocks B back; A charges B and applies a directional impulse).

**Weight:** N/A (physics-coupling, not in AffinityConfig).

**Co-location:** Physics-coupling, **directed** (A→B; asymmetric). A's cluster must own the impulse computation and replicate the resulting velocity to B's cluster. The edge is directed because A is the authority on the impulse and B receives it as a replicated delta.

**Cut-cost:** Correctness if cut. If A and B are on different clusters, the impulse application must not split the decision (e.g., only A computes it, or both compute it differently). **Best practice:** A's cluster applies the impulse, replicates B's new velocity to B's cluster. Cutting the edge means asynchronous impulse propagation, risking dropped or double-applied impulses.

**Rationale:** Knockback and directional forces are critical to moment-to-moment gameplay (dodge, positioning, group control). They require low-latency, high-fidelity application. Co-locating A and B, or dedicating a replication stream from A to B, is necessary.

**Directed:** Unlike Collision (symmetric; both entities are equals in the contact), PhysicsImpulse is one-way: A is the source, B is the target. This is the first directed edge kind.

---

### Joint (NEW)

**Trigger:** Rapier joint constraint between two bodies (revolute, prismatic, ball-and-socket, fixed, spring, or any Rapier joint type).

**Weight:** N/A (uncuttable; infinite weight).

**Co-location:** **HARD — must co-locate**. A cross-cluster joint is mathematically invalid. Rapier solves joint constraints in its physics loop; constraint satisfaction requires both bodies' motion and velocities to be synchronized in real time. A joint spanning two independent physics simulations (separate clusters) will violate the constraint, causing bodies to separate or violate the joint's degrees of freedom.

**Cut-cost:** **Infinite cost if cut** (must not cut). The clusterer's min-cut algorithm treats Joint edges as infinite-weight edges. They must never appear on a cluster boundary. If two jointed entities approach a split boundary, the clusterer must move them both to one side or refuse the split entirely.

**Rationale:** Physics joints are hard constraints in the Rapier engine. Distributing them across clusters is infeasible without a global physics solver on the master. Keeping jointed pairs co-located is non-negotiable.

**Consequence:** This is the only edge kind that can **block a split**. If a cluster is over its resource ceiling and all possible cuts would sever Joint edges, the cluster cannot split; the only recourse is vertical scaling (bigger hardware).

---

### SharedDeterministic (NEW)

**Trigger:** Shared deterministic seed or outcome (e.g., projectile spawn from a shared seed; destruction sequence seeded by one entity applied to another; random terrain effect both players experience).

**Weight:** N/A (cut-free; zero cost).

**Co-location:** **Cut-free — replicate as seed, no co-location needed**. Both clusters replicate the seed (a small deterministic input, e.g., initial position + angle + random number generator state), compute the effect locally (projectile trajectory, destruction, effect), and produce the same result without synchronization. No live state replication required.

**Cut-cost:** **Zero cost if cut**. Both sides compute from the seed; no bandwidth penalty. This is the only edge kind where cutting incurs no replication burden.

**Rationale:** Some game events are deterministic and reproducible from a seed. If entity A fires a projectile at entity B, and both clusters replicate the seed (position, direction, RNG state), both can compute the projectile path and impact locally without live replication. This is efficient for effects, crowd projectiles, and destruction effects. It trades computation (run the deterministic function twice) for zero replication.

**Constraint:** Seed-based events are only safe when both implementations are guaranteed identical. Game code must ensure the projectile sim, destruction sequence, or effect is deterministic and reproducible across clusters. If there is any per-tick variation (per-cluster local state), it must go into a different edge kind (Collision or PhysicsImpulse).

---

## How the clusterer uses these edges

The clusterer's core task is **finding the minimum cut of the interaction graph**—partitioning entities into clusters such that the sum of edge weights crossing cluster boundaries is minimized.

### Uncuttable (Joint)

Joint edges have infinite weight. The min-cut algorithm (e.g., METIS, KL-style local refinement) treats them as hard constraints:

- **No edge crossing.** A cut is invalid if it severs a Joint edge.
- **Clique constraint.** Jointed pairs must remain in the same partition.
- **Blocking.** If a cluster is over-loaded and all possible cuts would sever Joint edges, the cluster cannot split. This is a rare corner (geometrically, a true rigid body clique is small) but a valid failure mode requiring vertical scaling.

### Cut-free (SharedDeterministic)

SharedDeterministic edges have zero weight and do not contribute to cut cost:

- **Free to cut.** The min-cut algorithm ignores these edges; cutting them incurs no penalty.
- **Replicate seed.** When cut, both clusters replicate the seed and compute the effect independently.
- **No correctness risk.** Deterministic computation on both sides produces the same outcome.

### Weighted (all others: Proximity, GameAction, PartyMember, GuildMember, Collision, PhysicsImpulse)

These edges contribute weight to the cut cost proportional to their interaction weight:

```
cut_cost = Σ over (edges crossing boundary) weight(edge)
```

The min-cut algorithm minimizes this sum, finding the partition that trades off interaction weight against cluster balance. Higher-weight edges (PartyMember, GameAction) are less likely to cross a boundary; lower-weight edges (Proximity) are more expendable.

**Replication strategy:** When a weighted edge crosses a boundary, the owning cluster replicates the entity to the neighbor cluster at a rate proportional to the edge weight (see [meta-control-layer.md](meta-control-layer.md) §7 for graceful degradation). The boundary entity is kept up-to-date at all consumers.

---

## Mapping to the four-bucket state model

Each edge kind determines what state crosses cluster boundaries and which bucket it lives in:

### Spine (bucket 1)

**Always replicated:** `entity_id`, `cluster_id`, `position`, `velocity`.

All edges, regardless of kind, assume spine replication (entities must be locatable and have current pose for correctness).

### Replicated simulation payload (bucket 2)

**Conditional replication** based on edge kind:

- **Proximity, GameAction, PartyMember, GuildMember:** Boundary entities' `user_data` is replicated at full or reduced rate (per the Router's rate law; see [meta-control-layer.md](meta-control-layer.md) §4). Game state (HP, buffs, position errors) must cross.
- **Collision:** Both entities' `user_data` includes collision-relevant fields (physics state, body type, collision group) replicated for collision detection.
- **PhysicsImpulse:** A's cluster computes the impulse and sends B's resulting velocity change (bucket 2 delta: `velocity` update).
- **Joint:** Co-located; no cross-boundary replication needed (same cluster simulation).
- **SharedDeterministic:** Only the seed (small deterministic input) crosses; typically stored in metadata, not `user_data`. Both clusters compute the effect from the seed.

### Cluster-local (bucket 3)

**Never crosses cluster boundaries** (per spec: `skip_serializing` + `skip_deserializing`).

- Local cache state, per-cluster animation state, transient per-tick scratchpads.
- All edge kinds respect bucket 3 encapsulation; boundary entities retain only bucket 1 and 2.

### Durable (bucket 4)

**SpacetimeDB tables and reducers** (event-driven or throttled writes; not on the hot path).

- Shared outcomes (quest progress, destruction state, battle results) that must persist and sync across clusters.
- All edge kinds may populate durable state (e.g., GameAction triggers a quest reducer; SharedDeterministic seed is persisted for recovery).

---

## Four-bucket mapping reference

| Edge kind | Bucket 1 (Spine) | Bucket 2 (Replicated) | Bucket 3 (Local) | Bucket 4 (Durable) |
|---|---|---|---|---|
| **Proximity** | ✓ position, velocity | ✓ cosmetic pose, range indicators | — | — |
| **GameAction** | ✓ position, velocity | ✓ action state, target, cooldown | — | ✓ action outcome (quest, loot) |
| **PartyMember** | ✓ position, velocity | ✓ party state, member status | — | ✓ party roster (durable) |
| **GuildMember** | ✓ position, velocity | ✓ guild affiliation | — | ✓ guild roster (durable) |
| **Collision** | ✓ position, velocity | ✓ collision group, body type, shape | — | — |
| **PhysicsImpulse** | ✓ position, velocity (delta) | ✓ velocity change from impulse | — | — |
| **Joint** | ✓ position, velocity (both bodies, co-located) | ✓ joint-relevant state | — | ✓ permanent joint definition |
| **SharedDeterministic** | ✓ position, velocity (seed params only) | ✓ seed (deterministic input) | — | ✓ outcome, destruction state |

---

## Demo-side game-design notes (out of scope)

The [Arcane Arena](https://github.com/brainy-bots/arcane-arena) repository contains a **`COMBAT_DESIGN.md`** describing the four physics-authority buckets in the demo game context: terrain authority (single), self authority (owned cluster), replica authority (cross-cluster sync), and client-predicted authority (cosmetic). Cross-reference that document for gameplay-specific edge semantics.

---

## Related readings

- **[meta-control-layer.md](meta-control-layer.md)** §3 (Communication model) and §5 (Clustering policy) explain how these edges feed the min-cut algorithm and replication strategy.
- **[four-bucket-state-model.md](four-bucket-state-model.md)** defines Buckets 1–4 and the trust boundaries for each.
- **[adr/002-cross-cluster-physics.md](adr/002-cross-cluster-physics.md)** dives into Collision and PhysicsImpulse correctness requirements and cross-cluster sync options.
- **`crates/arcane-affinity/src/config.rs`** is the source of truth for default weights and the only place to update them.
- **`crates/arcane-affinity/src/interaction_graph.rs`** records edges and computes pairwise weights.

---

*Arcane Engine — Interaction-edge taxonomy — Reference*
