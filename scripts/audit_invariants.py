"""INVARIANT AUDIT: verify the founder's stated design rules mechanically
against the source, not against the agent's claims.

Each check names the founder's rule, then greps for the code that must (or
must not) exist. Exit code 1 if any invariant is violated.
"""
import io, re, sys

ARC = r"E:\code\pgp-demo\arcane\crates"
VIZ = r"E:\code\pgp-demo\arcane-viz"

def read(p):
    return io.open(p, encoding="utf-8", errors="replace").read()

failures = []
def check(name, rule, ok, detail=""):
    status = "PASS" if ok else "FAIL"
    print(f"[{status}] {name}\n        rule: {rule}")
    if detail:
        print(f"        {detail}")
    if not ok:
        failures.append(name)

ek = read(rf"{ARC}\arcane-infra\src\entity_keys.rs")
nc = read(rf"{ARC}\arcane-infra\src\node_core.rs")
mg = read(rf"{ARC}\arcane-infra\src\manager.rs")
cfg = read(rf"{ARC}\arcane-affinity\src\config.rs")
launch = read(rf"{VIZ}\scripts\launch_stack.py")

# --- 1. Ownership: only the owner writes; only the owner transfers.
check(
    "owner-gated writes",
    "only the owner can write an entity record (enforced in Redis)",
    "if cur and cur ~= ARGV[1] then return 0 end" in ek.split("WRITE_SCRIPT")[1][:400],
)
check(
    "handoff is owner-gated",
    "ownership moves ONLY by the current owner's write",
    "if cur and cur ~= ARGV[1] then return 0 end" in ek.split("HANDOFF_SCRIPT")[1][:500]
    and "'owner', ARGV[5]" in ek,
)
check(
    "no claim path exists",
    "the receiver must NEVER claim ownership",
    "CLAIM_SCRIPT" not in ek and "EntityWriteOp::Claim" not in nc and "Claim {" not in ek,
)
check(
    "handoff fires on release (not adoption)",
    "the OLD owner hands off when it loses the entity",
    "HANDOFF on release" in nc and "report\n                        .lost" in nc.replace("\r\n", "\n"),
)
check(
    "handoff carries final state",
    "the transfer write includes the last simulated state",
    "self.server.get_entity(*id)?" in nc,
)

# --- 2. Graph: predictions only, no accrual, no decay in predicted mode.
check(
    "predicted-graph flag exists",
    "graph holds ONLY the predictor's current p(a,b)",
    "predicted_graph" in cfg and "prediction_edge_scale" in cfg,
)
check(
    "no decay in predicted mode",
    "never degrade the current best estimate over time",
    "if !self.config.predicted_graph {" in mg and "interaction_graph.tick(" in mg,
)
check(
    "edge assignment not accrual",
    "an edge update REPLACES the value (set_edge), never adds",
    "set_edge(" in mg and "pub fn set_edge" in read(rf"{ARC}\arcane-affinity\src\interaction_graph.rs"),
)
check(
    "proximity accrual gated off in predicted mode",
    "raw proximity must not be summed into the graph",
    "if accrual_graph {" in mg,
)

# --- 3. Adopt-always: no stay/deadband/hold in pure-fresh.
check(
    "pure-fresh adopts always",
    "every cycle's fresh clustering is adopted; never keep the old one",
    "PURE FRESH (founder design, stated twice and now final)" in mg
    and "wave: true," in mg,
)
check(
    "no candidate hold",
    "no durability gate / candidate machinery in pure-fresh",
    "pending_wave" not in mg and "wave_candidate_cycles" not in mg,
)
check(
    "no stay-vs-fresh deadband in pure-fresh",
    "no incumbent comparison gating adoption",
    "desired_stay" not in mg,
)

# --- 4. Fresh solve is truly global (multilevel), not seeded local search.
ml = read(rf"{ARC}\arcane-affinity\src\multilevel.rs")
check(
    "multilevel is the fresh solver",
    "from-scratch solve must find k communities as k groups",
    "multilevel_partition(" in mg and "coarsen" in ml.lower(),
)
check(
    "hungarian label alignment",
    "groups are label-free; labels chosen to minimize migrations",
    "max_agreement_labels" in mg,
)

# --- 5. Demo config matches the stated model.
check(
    "demo runs pure fresh",
    "MANAGER_SEED_FROM_CURRENT=0",
    '"MANAGER_SEED_FROM_CURRENT": "0"' in launch,
)
check(
    "demo runs predicted graph",
    "MANAGER_PREDICTED_GRAPH=1",
    '"MANAGER_PREDICTED_GRAPH": "1"' in launch,
)
check(
    "demo runs entity keys",
    "ARCANE_ENTITY_KEYS=1",
    '"ARCANE_ENTITY_KEYS": "1"' in launch,
)

print()
if failures:
    print(f"INVARIANTS VIOLATED: {len(failures)} -> {failures}")
    sys.exit(1)
print("ALL INVARIANTS HOLD")
