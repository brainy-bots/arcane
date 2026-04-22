---
type: entity
tags: [tooling, cursor, storage, export, internals, workflow, backup]
---

# agent-transcripts

## What It Is
`agent-transcripts` is the folder where Cursor IDE stores raw conversation history for Agent and Composer sessions as JSONL files on disk. In the Arcane project context, it serves as the underlying data source for exporting and archiving development conversations — the chat sessions where architectural decisions, debugging sessions, and design discussions took place.

## Origin & Evolution
The concept surfaced during a one-time bulk export effort for the `pgp-demo` project (April 2026), when the goal was to capture all Cursor chat history into a browsable, human-readable format for backup and archival. Rather than manually reading the raw JSONL files from the `agent-transcripts` folder, the preferred path was tooling (`cursor-history` npm CLI) that could process both Agent and Composer conversation types in a single pass and emit structured Markdown. The folder's existence and path were documented as a fallback reference in case the CLI approach failed.

## Technical Details
- **Location on disk:** `C:\Users\Martin\.cursor\projects\e-code-pgp-demo\agent-transcripts\`
- **Format:** Raw JSONL files (one record per line, JSON-encoded)
- **Scope:** Contains both Agent and Composer conversation types
- **Access pattern:** Read-only for export purposes; Cursor writes to these files during active sessions
- The `cursor-history` npm CLI (requires Node 20+) is the recommended tool for converting these files to organized Markdown output; a Node v18 environment may fail due to version mismatch

## Key Design Decisions
- **Use CLI over manual JSONL parsing** — `cursor-history` handles both conversation types in one pass and produces structured output, reducing manual effort and error risk
- **Document raw path as fallback** — recording the `agent-transcripts` path directly ensures recovery options if tooling fails or version constraints block the CLI approach
- **Node 20+ requirement is a hard constraint** — the export workflow should validate the Node version before running; backward compatibility with v18 was not confirmed

## Relationships
- [[cursor-history CLI]]
- [[arcane-scaling-benchmarks]]
- [[pgp-demo project]]
- [[conversation export workflow]]

## Conversations That Shaped This
- [[Project conversation export options]]