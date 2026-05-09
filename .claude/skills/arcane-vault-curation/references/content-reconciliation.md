# Content Reconciliation

How to compare new information against existing vault content and decide what to do. This procedure runs every time you're about to write to the vault — whether you're a daily collection agent processing conversation dumps, a worker documenting a code change, or a session agent capturing a design decision.

## When this procedure applies

- You have new information (from code, a conversation, a commit, or a session) and need to decide whether it belongs in the vault
- You found an existing entity and aren't sure whether it needs updating
- You're reviewing proposed entities from the daily collection agent
- You suspect two entities describe the same concept

## Source hierarchy

When new information contradicts what's already in the vault, the source closer to ground truth wins:

```
1. Code (highest authority) — what the implementation actually does
2. Architecture docs (arcane/docs/architecture/) — the canonical design intent
3. Conversation with the founder — explicit design decisions
4. Existing vault entity — curated synthesis
5. Conversation dump (lowest) — raw session context, may contain abandoned ideas
```

**Why this order:** Code is the only source that can't lie about what the system does. Architecture docs are reviewed artifacts that describe intent. Conversations capture reasoning but also contain exploratory tangents, rejected alternatives, and ideas that were never implemented. The vault sits in the middle — it's curated, but it can go stale.

## The reconciliation procedure

### Step 1 — Identify what you have

Classify the new information:

| Type | Example | Typical source |
|------|---------|---------------|
| **New concept** | A trait that doesn't have an entity page | Code, conversation |
| **Updated fact** | A crate was renamed, a field was added | Code, commit |
| **Design decision** | "We chose X over Y because Z" | Conversation with founder |
| **Contradiction** | Vault says "4 buckets: Spine, Replicated, Ephemeral, Persistent" but code uses different names | Code vs vault |
| **Refinement** | More detail on something already captured | Conversation, code |
| **Stale content** | Entity describes a module that was deleted | Code (absence) |

### Step 2 — Find all existing vault content that overlaps

Don't stop at the first match. New information often touches multiple entities.

```
1. Search GLOSSARY.md for the primary term and synonyms
2. Search entity filenames: ls entities/ | grep -i <term>
3. Search entity content: grep -rl "<term>" entities/
4. Search briefs: grep -rl "<term>" briefs/
5. Check related entities via wikilinks (read the Relationships section of any match)
```

**The overlap problem:** A change to how ClusterManager routes players touches at least `ClusterManager.md`, `Affinity Clustering.md`, `IClusteringModel.md`, and `briefs/clustering.md`. If you only update one, the vault becomes internally inconsistent.

### Step 3 — Compare and classify the delta

For each overlapping vault entry, determine the relationship between new and existing content:

| Relationship | What it means | Action |
|-------------|--------------|--------|
| **Identical** | New info says the same thing as existing | Skip — no update needed |
| **Extends** | New info adds detail the vault doesn't have | Add to the relevant section |
| **Refines** | New info is more precise than existing | Replace the less precise statement |
| **Contradicts** | New info disagrees with existing | Apply source hierarchy to decide which wins |
| **Supersedes** | New info replaces existing (e.g., renamed, redesigned) | Update existing entity; mark old names as aliases |
| **Partial overlap** | New info is about a related but distinct concept | May need a new entity; check the "same concept?" test below |

### Step 4 — The "same concept?" test

When you find partial overlap, decide whether two things are the same concept or genuinely distinct:

**Same concept if:**
- They describe the same runtime behavior from different angles
- One is a synonym or abbreviation of the other
- Removing one would leave no information gap (the other covers it)
- The architecture docs treat them as one thing

**Distinct concepts if:**
- They have different implementations in code (different structs, traits, modules)
- They serve different purposes even if related
- The architecture docs give them separate pages
- A developer would need to understand both independently

**Real example — `Four-Bucket Data Model` vs `Four-Bucket State Model`:**
These are the same concept. The architecture doc (`arcane/docs/architecture/four-bucket-state-model.md`) calls it the "four-bucket entity and world state model." Both vault entities describe the same 4-bucket classification. The "Data Model" version uses slightly different bucket names (Spine/Replicated/Ephemeral/Persistent) while the canonical doc uses (Spine/Replicated simulation payload/Cluster-local ephemeral/Durable authoritative). Correct action: keep `Four-Bucket State Model.md` (closer to canonical name), merge any unique content from `Four-Bucket Data Model.md` into it, add "four-bucket data model" as an alias, delete the duplicate.

**Real example — `ReplicationChannel` vs `RedisReplicationChannel`:**
These are distinct concepts. `ReplicationChannel` is the trait interface; `RedisReplicationChannel` is a concrete implementation. A developer needs to understand both. Correct action: keep both, ensure the relationship is documented via wikilinks.

### Step 5 — Apply the update

Based on the classification from Step 3:

**For "Extends" or "Refines":**
1. Update the relevant section of the existing entity
2. If the source is a conversation, add it to "Conversations That Shaped This"
3. Check whether the change affects the brief — update if so

**For "Contradicts":**
1. Check the source hierarchy — which source is more authoritative?
2. If code contradicts vault → vault is stale; update vault to match code
3. If conversation contradicts vault → check whether the conversation reflects a decision or just exploration. Decisions update the vault; exploration doesn't.
4. If two vault entities contradict each other → check architecture docs for the canonical answer. If no canonical source, flag for founder review.

**For "Supersedes":**
1. Update the existing entity with the new information
2. Add the old name as an alias in frontmatter
3. If a file needs renaming (canonical name changed), rename it and update all wikilinks

**For "Same concept, duplicate entity":**
1. Identify which entity has the canonical name (check architecture docs)
2. Read both entities fully — identify content unique to each
3. Merge unique content from the non-canonical entity into the canonical one
4. Add the non-canonical name as an alias
5. Delete the duplicate file
6. Search all vault files for wikilinks to the deleted file and update them
7. Update GLOSSARY.md (or entity frontmatter aliases so the builder will)

## Staleness detection

An entity is stale when:

- **Code referent removed**: the struct, trait, module, or file it describes no longer exists in code. Verify with: `grep -r "<EntityName>" ../arcane/src/` (or the relevant repo).
- **Code referent renamed**: same concept exists but under a different name in code. Update the entity's canonical name and add the old name as alias.
- **Behavior changed**: the entity describes behavior that the code no longer implements. Update Technical Details and Key Design Decisions.
- **Relationships broken**: wikilinks point to entities that no longer exist. Check `[[linked entity]]` — if the target file is missing, the link is broken.

**Staleness check commands:**
```bash
# Check if an entity's code referent still exists
grep -r "ClusterManager" ../arcane/src/ --include="*.rs" -l

# Find broken wikilinks in an entity
grep -oP '\[\[([^\]]+)\]\]' entities/ClusterManager.md | while read link; do
  name=$(echo "$link" | sed 's/\[\[//;s/\]\]//')
  [ ! -f "entities/${name}.md" ] && echo "BROKEN: $link"
done

# Find entities with no code referent (potential staleness)
for f in entities/*.md; do
  name=$(basename "$f" .md)
  if ! grep -rq "$name" ../arcane/src/ --include="*.rs" 2>/dev/null; then
    echo "NO CODE REF: $name"
  fi
done
```

Not every entity without a code referent is stale — some entities describe concepts (Affinity Clustering) rather than code artifacts (ClusterManager). Use judgment.

## Confidence levels for automated updates

The daily collection agent operates at different confidence levels depending on the type of change:

| Confidence | What the agent does | Example |
|-----------|-------------------|---------|
| **High — auto-apply** | Add conversation to "Conversations That Shaped This"; add alias to frontmatter; update broken wikilinks | New session discussed ClusterManager; add session link |
| **Medium — apply with `[proposed]` tag** | New entity from conversation; extend Technical Details with new info from code | Conversation introduced a new trait not in vault |
| **Low — flag for review** | Contradiction between vault and code; entity appears stale; merge two entities | Vault says 4 buckets but code shows 5 |

The `[proposed]` tag in frontmatter means: "this change was made by an automated agent and hasn't been reviewed." The daily PR reviewer checks all `[proposed]` items before merging.

## Reconciliation for briefs

Briefs are higher-stakes than entities — they're the Layer 1 summary that many agents read. Reconciliation rules are stricter:

1. **Never auto-update a brief.** All brief changes go through review.
2. **When an entity update changes the brief-level story**, add a `[brief-update-needed]` comment at the top of the brief with a one-line description of what changed.
3. **When reconciling multiple entity updates**, batch them into a single brief update rather than applying them one at a time.
4. **The "Common misunderstandings" table is especially sensitive** — incorrect entries here propagate misinformation to every agent that reads the brief.

## Edge cases

**New information from an abandoned conversation:** Not every conversation produces lasting decisions. If a conversation explored an idea but the founder didn't approve it, don't add it to the vault as fact. Check: did the conversation end with approval, or was the idea dropped? When in doubt, don't update.

**Conflicting conversations:** Two sessions may reach different conclusions about the same topic (e.g., the founder changed their mind). The most recent conversation wins, but only if it reflects a deliberate reversal, not just a different framing. When in doubt, flag for review.

**Entity that spans multiple briefs:** Some entities (like SpacetimeDB) are relevant to clustering, replication, AND persistence. The entity belongs in the brief that's most central to its purpose. Other briefs reference it via wikilink but don't duplicate its content.
