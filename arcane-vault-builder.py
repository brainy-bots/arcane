#!/usr/bin/env python3
"""
arcane-vault-builder.py
Build an Obsidian vault from Arcane chat histories + repo documentation.

Usage:
    python arcane-vault-builder.py            # estimate cost, show plan
    python arcane-vault-builder.py --sample   # process 1 small file end-to-end
    python arcane-vault-builder.py --confirm  # full build

Requires: ANTHROPIC_API_KEY in environment (source ~/.bashrc)
"""

import argparse
import hashlib
import json
import os
import re
import sys
import textwrap
from datetime import datetime
from pathlib import Path
from typing import Optional

import anthropic

# ─────────────────────────────────────── Paths ──────────────────────────────

WORKSPACE       = Path("/home/vr0n1n/Workspace")
ARCANE_REPO     = WORKSPACE / "arcane"
VAULT_DIR       = ARCANE_REPO / "arcane-vault"
CACHE_DIR       = ARCANE_REPO / ".vault-build"
CURSOR_CHATS    = WORKSPACE / "arcane-scaling-benchmarks" / "cursor-chat-export-pgp-demo"
CLAUDE_CONVOS   = WORKSPACE / "arcane-scaling-benchmarks" / "claude-conversations"

BENCHMARKS_REPO = WORKSPACE / "arcane-scaling-benchmarks"

REPOS = {
    "arcane":                  ARCANE_REPO,
    "arcane-demos":            WORKSPACE / "arcane-demos",
    # arcane_swarm and arcane live as submodules inside arcane-scaling-benchmarks
    "arcane_swarm":            BENCHMARKS_REPO / "arcane_swarm",
    "arcane (submodule)":      BENCHMARKS_REPO / "arcane",
    "arcane-scaling-benchmarks": BENCHMARKS_REPO,
    "arcane-website":          WORKSPACE / "arcane-website",
}

# ─────────────────────────────────────── Models ─────────────────────────────

MODEL_CHUNK = "claude-haiku-4-5-20251001"   # cheap + fast — chunk summaries
MODEL_SYNTH = "claude-sonnet-4-6"           # strong  — synthesis + entity pages

CHUNK_CHARS = 180_000        # ~50k tokens per chunk (safe for 200k context)
CHARS_PER_TOKEN = 3.5

PRICING = {                  # USD per 1M tokens
    MODEL_CHUNK: {"input": 0.80,  "output": 4.00},
    MODEL_SYNTH: {"input": 3.00,  "output": 15.00},
}

# ─────────────────────────────────────── Seeded entities ────────────────────

SEED_ENTITIES = [
    "Arcane Engine", "PGP Architecture", "Affinity Clustering",
    "Physics at Scale", "Heterogeneous Node Tiers",
    "ClusterManager", "ClusterServer", "arcane_swarm",
    "SpaceTimeDB", "Unreal Engine Client",
    "Benchmark System", "AWS Infrastructure",
    "Rapier", "Redis", "Spatial Grid",
    "arcane-scaling-benchmarks", "arcane-demos", "arcane-website",
    "Benchmark Journal", "CI Pipeline", "arcane-client-unreal",
]

# ─────────────────────────────────────── Utilities ──────────────────────────

def sha256_of_path(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def cache_path(key: str, ext: str = "json") -> Path:
    return CACHE_DIR / f"{key}.{ext}"


def cache_get(key: str) -> Optional[dict]:
    p = cache_path(key)
    if p.exists():
        return json.loads(p.read_text())
    return None


def cache_set(key: str, data: dict) -> None:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cache_path(key).write_text(json.dumps(data, indent=2))


def estimate_tokens(text: str) -> int:
    return int(len(text) / CHARS_PER_TOKEN)


def fmt_cost(tokens_in: int, tokens_out: int, model: str) -> str:
    p = PRICING[model]
    c = tokens_in * p["input"] / 1e6 + tokens_out * p["output"] / 1e6
    return f"${c:.2f}"


def wrap(text: str, width: int = 100) -> str:
    return textwrap.fill(text, width)


def slugify(name: str) -> str:
    return re.sub(r"[^\w\s-]", "", name).strip().replace(" ", "-")

# ─────────────────────────────────────── Source parsers ─────────────────────

def extract_cursor_text(path: Path) -> str:
    """
    Extract conversational text from a Cursor chat export.

    Priority order (determines which handler wins per line):
      1. Section headers — resets all state
      2. Tool/thinking block active → skip (track code fences to stay in sync)
      3. Tool/thinking marker → enter tool block
      4. Code block active → keep up to MAX_CODE_LINES, then truncate
      5. ID metadata → skip
      6. Normal text → keep

    Result: user turns + assistant prose only; no tool outputs, no empty turns.
    """
    MAX_CODE_LINES = 20

    lines = path.read_text(errors="replace").splitlines()
    output: list[str] = []

    in_tool_block = False
    in_code_block = False
    code_block_lines = 0

    # Track current turn's content lines to drop empty turns
    current_section_header: Optional[str] = None
    current_section_lines: list[str] = []

    def flush_section() -> None:
        """Emit the current section only if it has meaningful content."""
        if current_section_header is None:
            return
        body = "\n".join(current_section_lines).strip()
        # Drop turns with <20 meaningful chars (empty tool-only turns)
        meaningful = re.sub(r"\s+", "", body)
        if len(meaningful) >= 20:
            output.append(current_section_header)
            output.append("")
            output.append(body)
            output.append("")

    for line in lines:
        stripped = line.strip()

        # ── 1. Section header ────────────────────────────────────────────
        if stripped.startswith("### **"):
            flush_section()
            in_tool_block = False
            in_code_block = False
            code_block_lines = 0
            current_section_header = line
            current_section_lines = []
            continue

        # ── 2. Inside tool/thinking block — skip everything ──────────────
        if in_tool_block:
            # Track code fences so we can stay in sync if there's a mismatch
            if stripped.startswith("```"):
                in_code_block = not in_code_block
            continue  # skip all tool block content unconditionally

        # ── 3. Tool/thinking block markers ──────────────────────────────
        tool_markers = ("[Tool:", "[Thinking]", "Status: ✓", "Status: ✗")
        if any(stripped.startswith(m) for m in tool_markers):
            in_tool_block = True
            in_code_block = False
            continue

        # ── 4. Fenced code blocks ────────────────────────────────────────
        if stripped.startswith("```"):
            if in_code_block:
                in_code_block = False
                if code_block_lines > MAX_CODE_LINES:
                    current_section_lines.append(
                        f"[... {code_block_lines - MAX_CODE_LINES} more lines truncated]"
                    )
                current_section_lines.append(line)
                code_block_lines = 0
            else:
                in_code_block = True
                code_block_lines = 0
                current_section_lines.append(line)
            continue

        if in_code_block:
            code_block_lines += 1
            if code_block_lines <= MAX_CODE_LINES:
                current_section_lines.append(line)
            continue

        # ── 5. ID metadata ───────────────────────────────────────────────
        if re.match(r"\*\*ID\*\*:\s+`[0-9a-f-]+`", stripped):
            continue

        # ── 6. Normal content ────────────────────────────────────────────
        current_section_lines.append(line)

    flush_section()

    result = "\n".join(output)
    result = re.sub(r"\n{4,}", "\n\n\n", result)
    return result


def extract_jsonl_text(path: Path) -> str:
    """
    Flatten a Claude Code JSONL session into a readable transcript.
    Keeps: user messages, assistant text, tool names (not output).
    Skips: file-history-snapshot, permission-mode, raw tool_result content.
    """
    lines_out = []

    with open(path) as f:
        for raw in f:
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue

            msg_type = obj.get("type", "")
            msg = obj.get("message", {})
            role = msg.get("role", "")
            content = msg.get("content", "")

            # Skip non-conversational entries
            if msg_type in ("permission-mode", "file-history-snapshot", "attachment"):
                continue

            # User messages
            if msg_type == "user" and role == "user":
                if isinstance(content, list):
                    for block in content:
                        if block.get("type") == "tool_result":
                            # Skip tool results (can be huge file contents)
                            tool_id = block.get("tool_use_id", "")
                            lines_out.append(f"\n[Tool result for {tool_id} — omitted]\n")
                        elif block.get("type") == "text":
                            txt = block.get("text", "").strip()
                            if txt:
                                lines_out.append(f"\n**User:** {txt}\n")
                elif isinstance(content, str) and content.strip():
                    lines_out.append(f"\n**User:** {content.strip()}\n")

            # Assistant messages
            elif msg_type == "assistant" and role == "assistant":
                if isinstance(content, list):
                    for block in content:
                        btype = block.get("type", "")
                        if btype == "text":
                            txt = block.get("text", "").strip()
                            if txt:
                                lines_out.append(f"\n**Assistant:** {txt}\n")
                        elif btype == "thinking":
                            txt = block.get("thinking", "").strip()
                            if txt and len(txt) > 100:
                                # Keep reasoning but truncate
                                preview = txt[:500] + ("..." if len(txt) > 500 else "")
                                lines_out.append(f"\n[Thinking: {preview}]\n")
                        elif btype == "tool_use":
                            name = block.get("name", "unknown")
                            inp = block.get("input", {})
                            # Show tool name + key params only
                            params = {k: v for k, v in inp.items()
                                      if isinstance(v, (str, int, bool)) and len(str(v)) < 200}
                            lines_out.append(f"\n[Tool call: {name}({params})]\n")

    return "\n".join(lines_out)


def parse_cursor_metadata(path: Path) -> dict:
    """Extract date, title, message count from cursor chat header."""
    with open(path) as f:
        header = f.read(2000)

    title = re.search(r"^# (.+)$", header, re.MULTILINE)
    date  = re.search(r"\*\*Date\*\*:\s*(.+)$", header, re.MULTILINE)
    msgs  = re.search(r"\*\*Messages\*\*:\s*(\d+)", header, re.MULTILINE)
    workspace = re.search(r"\*\*Workspace\*\*:\s*(.+)$", header, re.MULTILINE)

    # Date from filename: 2026-02-20-20-...
    fname_date = re.match(r"(\d{4}-\d{2}-\d{2})", path.name)

    return {
        "title":     title.group(1).strip() if title else path.stem,
        "date":      (date.group(1).strip() if date else
                      fname_date.group(1) if fname_date else "unknown"),
        "messages":  int(msgs.group(1)) if msgs else 0,
        "workspace": workspace.group(1).strip() if workspace else "",
        "source":    "cursor",
        "file":      str(path),
    }


def parse_jsonl_metadata(path: Path) -> dict:
    """Extract metadata from a Claude Code JSONL session."""
    cwd = ""
    date = "unknown"
    msg_count = 0

    with open(path) as f:
        for raw in f:
            raw = raw.strip()
            if not raw:
                continue
            msg_count += 1
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if not cwd and obj.get("cwd"):
                cwd = obj["cwd"]
            if date == "unknown" and obj.get("timestamp"):
                date = obj["timestamp"][:10]
            if cwd and date != "unknown":
                break   # found both, no need to scan further

    return {
        "title":    f"Claude Code session — {Path(cwd).name if cwd else path.stem}",
        "date":     date,
        "messages": msg_count,
        "workspace": cwd,
        "source":   "claude-code",
        "file":     str(path),
    }

# ─────────────────────────────────────── API calls ──────────────────────────

def call_api(client: anthropic.Anthropic, model: str, system: str, user: str,
             max_tokens: int = 2048, retries: int = 3) -> str:
    import time
    for attempt in range(retries):
        try:
            resp = client.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system,
                messages=[{"role": "user", "content": user}],
                timeout=120.0,
            )
            return resp.content[0].text
        except (anthropic.APITimeoutError, anthropic.APIConnectionError) as e:
            if attempt < retries - 1:
                wait = 5 * (attempt + 1)
                print(f"    [retry] attempt {attempt+1}/{retries} failed: {e}. Waiting {wait}s...")
                time.sleep(wait)
            else:
                raise


CHUNK_SYSTEM = """\
You are an expert technical writer summarizing portions of AI coding sessions for an Obsidian knowledge vault.

Your task: read this segment of a chat session and produce a dense, structured summary covering:
- What was being worked on in this segment
- Key technical decisions or approaches discussed
- Problems encountered and how they were resolved
- Files/systems/components touched
- Any conclusions or insights

Keep it factual and concise (300-600 words). Use markdown. Do NOT repeat the chunk content verbatim.
"""

SYNTHESIS_SYSTEM = """\
You are building an Obsidian knowledge vault for the Arcane multiplayer backend project.

Given multiple chunk summaries from a single chat session, synthesize them into ONE cohesive conversation note.
Return ONLY valid markdown with this EXACT structure (fill every section):

---
type: conversation
date: {date}
source: {source}
tags: [comma-separated relevant tags here]
---

# {title}

**Date:** {date}
**Source:** {source} ({messages} messages)
**File:** `{file}`

## Summary
[3-5 paragraph distillation of the full conversation. What was the goal? What was built or decided? What's the outcome?]

## What Was Built
- [bullet list of concrete artifacts: files, systems, features implemented]

## Key Decisions
- [bullet list of architectural or design decisions made with brief rationale]

## Problems Solved
- [bullet list of bugs, blockers, challenges resolved]

## Entities
[List ONLY entities from this seeded list that genuinely appear in this conversation — link them as [[Entity Name]]]
Seed list: Arcane Engine, PGP Architecture, Affinity Clustering, Physics at Scale, Heterogeneous Node Tiers, ClusterManager, ClusterServer, arcane_swarm, SpaceTimeDB, Unreal Engine Client, Benchmark System, AWS Infrastructure, Rapier, Redis, Spatial Grid, arcane-scaling-benchmarks, arcane-demos, arcane-website, Benchmark Journal, CI Pipeline, arcane-client-unreal

Also list any NEW entities not in the seed (prefix with NEW:):
- NEW: [[Some New Concept]]

## Related Conversations
[Leave as placeholder: `_to be linked_`]
"""

ENTITY_SYSTEM = """\
You are building an Obsidian knowledge vault for the Arcane multiplayer backend project.
You have summaries from all chat sessions. Build an entity page for the given concept.

Return ONLY valid markdown with this EXACT structure:

---
type: entity
tags: [{relevant tags}]
---

# {entity_name}

## What It Is
[2-3 sentences: what this is, what role it plays in Arcane]

## Origin & Evolution
[How did this come to be? What problem does it solve? Key milestones from the chat history.]

## Technical Details
[Architecture, design decisions, interfaces — what matters technically]

## Key Design Decisions
- [bullet: decision — rationale]

## Relationships
[List related entities as [[wikilinks]]]

## Conversations That Shaped This
[List relevant conversation notes as [[wikilinks]]]
"""

TIMELINE_SYSTEM = """\
You are building an Obsidian knowledge vault for the Arcane multiplayer backend project.
Given all conversation summaries, write a chronological development narrative and an index.

Return TWO markdown documents separated by the exact string: ===SPLIT===

DOCUMENT 1 — timeline.md:
---
type: timeline
---
# Arcane — Development Timeline

[Narrative of how Arcane was built, from first spec to current state.
Organized by phase/milestone. Use [[wikilinks]] for conversations and entities.
Be specific: what was built when, what drove each decision, how things evolved.]

===SPLIT===

DOCUMENT 2 — 00-INDEX.md:
---
type: index
---
# Arcane Vault — Index

## About This Vault
[1 paragraph: what this vault is, how to navigate it]

## Conversation Log
| Date | Title | Source | Key Topics |
|------|-------|--------|------------|
[one row per conversation, linked as [[title]]]

## Entity Map
[Group entities by category: Architecture, Infrastructure, External Systems, Repos.
Link as [[Entity Name]]. One line description each.]

## Quick Links
- [[timeline]] — full development narrative
- [[SCHEMA]] — vault conventions
"""


def summarize_chunk(client: anthropic.Anthropic, chunk: str, chunk_idx: int,
                    total: int, meta: dict, cache_key: str) -> str:
    key = f"{cache_key}_chunk{chunk_idx}"
    cached = cache_get(key)
    if cached:
        print(f"    [cache] chunk {chunk_idx+1}/{total}")
        return cached["summary"]

    print(f"    [api]   chunk {chunk_idx+1}/{total} ({estimate_tokens(chunk):,} tokens) → {MODEL_CHUNK}")
    summary = call_api(client, MODEL_CHUNK, CHUNK_SYSTEM,
                       f"Session: {meta['title']}\nDate: {meta['date']}\n\n---\n\n{chunk}",
                       max_tokens=800)
    cache_set(key, {"summary": summary, "chunk_idx": chunk_idx})
    return summary


def synthesize_conversation(client: anthropic.Anthropic, chunk_summaries: list[str],
                             meta: dict, repo_context: str, cache_key: str) -> str:
    key = f"{cache_key}_synth"
    cached = cache_get(key)
    if cached:
        print(f"    [cache] synthesis")
        return cached["note"]

    all_chunks = "\n\n---\n\n".join(
        f"[Chunk {i+1}/{len(chunk_summaries)}]\n{s}"
        for i, s in enumerate(chunk_summaries)
    )

    system = SYNTHESIS_SYSTEM.format(
        date=meta["date"], source=meta["source"],
        messages=meta["messages"], file=meta["file"], title=meta["title"],
    )
    user = (
        f"REPO CONTEXT (for entity grounding):\n{repo_context[:3000]}\n\n"
        f"CHUNK SUMMARIES:\n\n{all_chunks}"
    )

    print(f"    [api]   synthesis ({estimate_tokens(user):,} tokens) → {MODEL_SYNTH}")
    note = call_api(client, MODEL_SYNTH, system, user, max_tokens=3000)
    cache_set(key, {"note": note})
    return note


def generate_entity_page(client: anthropic.Anthropic, entity: str,
                          all_summaries: list[dict], repo_context: str) -> str:
    key = f"entity_{slugify(entity)}"
    cached = cache_get(key)
    if cached:
        print(f"    [cache] entity: {entity}")
        return cached["page"]

    # Find conversations mentioning this entity
    relevant = [
        s for s in all_summaries
        if entity.lower() in s.get("note", "").lower()
    ]
    mentions = "\n\n---\n\n".join(
        f"From [[{s['meta']['title']}]] ({s['meta']['date']}):\n{s['note'][:1500]}"
        for s in relevant[:6]
    )

    user = (
        f"ENTITY: {entity}\n\n"
        f"REPO CONTEXT:\n{repo_context[:4000]}\n\n"
        f"RELEVANT CONVERSATION EXCERPTS:\n\n{mentions or '(none found — infer from repo context)'}"
    )

    system = ENTITY_SYSTEM.replace("{entity_name}", entity)
    print(f"    [api]   entity: {entity} → {MODEL_SYNTH}")
    page = call_api(client, MODEL_SYNTH, system, user, max_tokens=2000)
    cache_set(key, {"page": page})
    return page


def generate_timeline_and_index(client: anthropic.Anthropic, all_summaries: list[dict],
                                 repo_context: str) -> tuple[str, str]:
    key = "timeline_index"
    cached = cache_get(key)
    if cached:
        print("    [cache] timeline + index")
        return cached["timeline"], cached["index"]

    digest = "\n\n---\n\n".join(
        f"## [[{s['meta']['title']}]] ({s['meta']['date']})\n{s['note'][:1200]}"
        for s in sorted(all_summaries, key=lambda x: x["meta"]["date"])
    )

    user = (
        f"REPO CONTEXT:\n{repo_context[:5000]}\n\n"
        f"ALL CONVERSATION SUMMARIES:\n\n{digest}"
    )

    print(f"    [api]   timeline + index → {MODEL_SYNTH}")
    result = call_api(client, MODEL_SYNTH, TIMELINE_SYSTEM, user, max_tokens=4000)

    parts = result.split("===SPLIT===")
    timeline = parts[0].strip() if len(parts) >= 2 else result
    index    = parts[1].strip() if len(parts) >= 2 else "# Index\n\n(generation failed)"

    cache_set(key, {"timeline": timeline, "index": index})
    return timeline, index

# ─────────────────────────────────────── Vault writers ──────────────────────

def write_vault_file(rel_path: str, content: str) -> None:
    dest = VAULT_DIR / rel_path
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(content)


def write_schema() -> None:
    schema = """\
---
type: schema
---

# Arcane Vault — Schema & Conventions

## Note types

| type | folder | description |
|------|--------|-------------|
| `conversation` | `conversations/` | One note per chat session |
| `entity` | `entities/` | Concept, system, or component page |
| `timeline` | root | Chronological development narrative |
| `index` | root (`00-INDEX.md`) | Master navigation |
| `schema` | root | This document |

## Linking conventions

- **Always use `[[Wikilink Name]]`** for cross-references — Obsidian graph view requires this.
- File names match the wikilink exactly (spaces OK, case-sensitive on Linux).
- Entity file names = entity name exactly (e.g. `entities/Arcane Engine.md`).
- Conversation file names = `YYYY-MM-DD Title.md`.

## Frontmatter

Every note has YAML frontmatter with at least `type` and `tags`.

## Graph view tips

- Filter by `type: entity` to see the concept map.
- Filter by `type: conversation` to see the session timeline.
- Connections = `[[wikilinks]]` in note bodies.

## Updating the vault

Re-run `arcane-vault-builder.py --confirm` to regenerate. Cached summaries are reused;
delete `.vault-build/<key>.json` to force regeneration of a specific file.
"""
    write_vault_file("SCHEMA.md", schema)

# ─────────────────────────────────────── Discovery ──────────────────────────

def discover_sources() -> list[dict]:
    sources = []

    # Cursor chat exports (markdown)
    if CURSOR_CHATS.exists():
        for p in sorted(CURSOR_CHATS.glob("*.md")):
            size = p.stat().st_size
            meta = parse_cursor_metadata(p)
            sources.append({"path": p, "size": size, "meta": meta, "format": "cursor"})

    # Claude Code JSONL sessions (skip tiny stubs: agent-*.jsonl and near-empty files)
    if CLAUDE_CONVOS.exists():
        for p in sorted(CLAUDE_CONVOS.rglob("*.jsonl")):
            if p.stat().st_size < 5_000:
                continue  # empty stubs and agent init files
            if p.name.startswith("agent-"):
                continue
            meta = parse_jsonl_metadata(p)
            sources.append({"path": p, "size": p.stat().st_size,
                            "meta": meta, "format": "jsonl"})

    # Sort by date
    sources.sort(key=lambda s: s["meta"]["date"])
    return sources


def collect_repo_context() -> str:
    """Read READMEs, WHY_ARCANE, CHANGELOG from all repos."""
    parts = []
    for repo_name, repo_path in REPOS.items():
        if not repo_path.exists():
            continue
        for fname in ["README.md", "WHY_ARCANE.md", "CHANGELOG.md",
                      "BRAND.md", "DESIGN.md"]:
            fp = repo_path / fname
            if fp.exists():
                content = fp.read_text(errors="replace")
                # Truncate very long files
                if len(content) > 20_000:
                    content = content[:20_000] + "\n\n[... truncated ...]"
                parts.append(f"## {repo_name}/{fname}\n\n{content}")

    return "\n\n---\n\n".join(parts)

# ─────────────────────────────────────── Cost estimate ──────────────────────

def estimate_run_cost(sources: list[dict], repo_context: str) -> None:
    print("\n── Cost Estimate ──────────────────────────────────────────────────")

    total_in_haiku  = 0
    total_out_haiku = 0
    total_in_synth  = 0
    total_out_synth = 0

    for s in sources:
        p = s["path"]
        if s["format"] == "cursor":
            text = extract_cursor_text(p)
        else:
            text = extract_jsonl_text(p)

        n_tok = estimate_tokens(text)
        n_chunks = max(1, len(text) // CHUNK_CHARS + 1)

        # Chunk summarization (Haiku)
        total_in_haiku  += n_tok
        total_out_haiku += n_chunks * 700   # ~700 output tokens per chunk

        # Synthesis (Sonnet): chunk summaries + repo context
        synth_in = n_chunks * 700 + estimate_tokens(repo_context[:3000])
        total_in_synth  += synth_in
        total_out_synth += 2500

        print(f"  {p.name[:55]:<55}  {n_tok:>7,} tok  {n_chunks} chunk(s)")

    # Entity pages
    n_entities = len(SEED_ENTITIES)
    total_in_synth  += n_entities * 6000
    total_out_synth += n_entities * 1500

    # Timeline + index
    total_in_synth  += estimate_tokens(repo_context[:5000]) + len(sources) * 1200
    total_out_synth += 4000

    ph = PRICING[MODEL_CHUNK]
    ps = PRICING[MODEL_SYNTH]
    cost_haiku = total_in_haiku * ph["input"] / 1e6 + total_out_haiku * ph["output"] / 1e6
    cost_synth = total_in_synth * ps["input"] / 1e6 + total_out_synth * ps["output"] / 1e6
    total_cost = cost_haiku + cost_synth

    print(f"\n  Chunk summarization ({MODEL_CHUNK}):")
    print(f"    Input:  {total_in_haiku:>10,} tokens  → ${total_in_haiku * ph['input'] / 1e6:.2f}")
    print(f"    Output: {total_out_haiku:>10,} tokens  → ${total_out_haiku * ph['output'] / 1e6:.2f}")
    print(f"  Synthesis + entities ({MODEL_SYNTH}):")
    print(f"    Input:  {total_in_synth:>10,} tokens  → ${total_in_synth * ps['input'] / 1e6:.2f}")
    print(f"    Output: {total_out_synth:>10,} tokens  → ${total_out_synth * ps['output'] / 1e6:.2f}")
    print(f"\n  ── TOTAL ESTIMATED COST: ${total_cost:.2f} ──")
    print()

# ─────────────────────────────────────── Processing ─────────────────────────

def process_source(client: anthropic.Anthropic, s: dict, repo_context: str) -> dict:
    p    = s["path"]
    meta = s["meta"]

    print(f"\n▶  {meta['date']} — {meta['title']}")
    print(f"   {p.name}  ({p.stat().st_size / 1024:.0f} KB)")

    fhash = sha256_of_path(p)

    # Extract text
    if s["format"] == "cursor":
        text = extract_cursor_text(p)
    else:
        text = extract_jsonl_text(p)

    print(f"   Extracted {len(text):,} chars ({estimate_tokens(text):,} tokens)")

    # Fast path: if synthesis is already cached, skip chunk API calls entirely
    synth_cached = cache_get(f"{fhash}_synth")
    if synth_cached:
        print(f"    [cache] synthesis (skipping chunks)")
        note = synth_cached["note"]
        return {"meta": meta, "note": note, "fhash": fhash}

    # Chunk
    chunks = [text[i:i+CHUNK_CHARS] for i in range(0, max(1, len(text)), CHUNK_CHARS)]

    # Summarize chunks
    chunk_summaries = []
    for idx, chunk in enumerate(chunks):
        summary = summarize_chunk(client, chunk, idx, len(chunks), meta, fhash)
        chunk_summaries.append(summary)

    # Synthesize into conversation note
    note = synthesize_conversation(client, chunk_summaries, meta, repo_context, fhash)

    return {"meta": meta, "note": note, "fhash": fhash}


def conversation_note_filename(meta: dict) -> str:
    safe_title = re.sub(r"[^\w\s-]", "", meta["title"])[:60].strip()
    return f"{meta['date']} {safe_title}.md"

# ─────────────────────────────────────── .gitignore ─────────────────────────

def ensure_gitignore() -> None:
    gi = ARCANE_REPO / ".gitignore"
    content = gi.read_text() if gi.exists() else ""
    entries = ["arcane-vault/", ".vault-build/"]
    added = []
    for e in entries:
        if e not in content:
            content += f"\n{e}"
            added.append(e)
    if added:
        gi.write_text(content)
        print(f"Added to .gitignore: {', '.join(added)}")

# ─────────────────────────────────────── Main ───────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="Build Arcane Obsidian vault")
    parser.add_argument("--sample",  action="store_true",
                        help="Process one small file end-to-end (sanity check)")
    parser.add_argument("--confirm", action="store_true",
                        help="Run the full build")
    args = parser.parse_args()

    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        print("ERROR: ANTHROPIC_API_KEY not set. Run: source ~/.bashrc")
        sys.exit(1)

    client = anthropic.Anthropic(api_key=api_key)

    print("── Arcane Vault Builder ───────────────────────────────────────────")
    print(f"Vault:  {VAULT_DIR}")
    print(f"Cache:  {CACHE_DIR}")

    sources = discover_sources()
    print(f"\nFound {len(sources)} source files:")
    for s in sources:
        print(f"  [{s['format']:11s}] {s['meta']['date']}  {s['path'].name[:60]}")

    print(f"\nCollecting repo context from {len(REPOS)} repos...")
    repo_context = collect_repo_context()
    print(f"  Repo context: {len(repo_context):,} chars ({estimate_tokens(repo_context):,} tokens)")

    estimate_run_cost(sources, repo_context)

    if not args.sample and not args.confirm:
        print("Run with --sample to test one file, or --confirm for the full build.")
        return

    ensure_gitignore()
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    VAULT_DIR.mkdir(parents=True, exist_ok=True)

    if args.sample:
        # Pick smallest source for a quick sanity check
        smallest = min(sources, key=lambda s: s["size"])
        print(f"\n── SAMPLE MODE: processing {smallest['path'].name} ──")
        result = process_source(client, smallest, repo_context)
        fname  = conversation_note_filename(result["meta"])
        write_vault_file(f"conversations/{fname}", result["note"])
        write_schema()
        print(f"\n✓ Sample note written: arcane-vault/conversations/{fname}")
        print("  Review it, then run --confirm for the full build.")
        return

    # ── Full build ──────────────────────────────────────────────────────────
    print("\n── FULL BUILD ─────────────────────────────────────────────────────")

    all_summaries = []

    for s in (src for src in sources if src["size"] > 500):
        result = process_source(client, s, repo_context)
        all_summaries.append(result)
        fname = conversation_note_filename(result["meta"])
        write_vault_file(f"conversations/{fname}", result["note"])
        print(f"   ✓ conversations/{fname}")

    # Discover all entities (seeded + new ones found in notes)
    print("\n── Collecting entities ─────────────────────────────────────────────")
    all_entities = list(SEED_ENTITIES)
    new_pattern  = re.compile(r"NEW:\s*\[\[([^\]]+)\]\]", re.IGNORECASE)
    for s in all_summaries:
        for match in new_pattern.finditer(s["note"]):
            candidate = match.group(1).strip()
            if candidate not in all_entities:
                all_entities.append(candidate)
                print(f"  + Discovered entity: {candidate}")
    print(f"  Total entities: {len(all_entities)}")

    # Generate entity pages
    print("\n── Generating entity pages ─────────────────────────────────────────")
    for entity in all_entities:
        page  = generate_entity_page(client, entity, all_summaries, repo_context)
        fname = f"{entity}.md"
        write_vault_file(f"entities/{fname}", page)
        print(f"   ✓ entities/{fname}")

    # Generate timeline + index
    print("\n── Generating timeline + index ─────────────────────────────────────")
    timeline, index = generate_timeline_and_index(client, all_summaries, repo_context)
    write_vault_file("timeline.md",  timeline)
    write_vault_file("00-INDEX.md",  index)
    write_schema()
    print("   ✓ timeline.md")
    print("   ✓ 00-INDEX.md")
    print("   ✓ SCHEMA.md")

    print(f"\n╔══ Vault complete ═══════════════════════════════════════════════╗")
    print(f"║  {VAULT_DIR}")
    print(f"║  {len(all_summaries)} conversation notes")
    print(f"║  {len(all_entities)} entity pages")
    print(f"╚═════════════════════════════════════════════════════════════════╝")


if __name__ == "__main__":
    main()
