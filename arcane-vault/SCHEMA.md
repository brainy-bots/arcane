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
