---
type: entity
tags: [tooling, cli, export, cursor, node, backup, workflow, npm]
---

# cursor-history CLI

## What It Is
`cursor-history` is an npm CLI tool designed to bulk-export Cursor chat and agent conversations into structured, human-readable formats (primarily Markdown). In the Arcane project context, it was used to archive conversation history from Cursor sessions for knowledge preservation and vault ingestion purposes.

## Origin & Evolution
The need arose from a desire to create a clean, browsable record of all Cursor chat/agent conversations from the `pgp-demo` project without repeatedly navigating Cursor's UI or manually parsing raw internal files. The alternative — directly copying Cursor's internal `agent-transcripts` folder (raw JSONL files at `C:\Users\Martin\.cursor\projects\e-code-pgp-demo\agent-transcripts\`) — was considered but rejected because it required manual handling of multiple conversation types. The `cursor-history` CLI was preferred because it handles both Agent and Composer conversation types in a single pass.

## Technical Details
- **Package type:** npm CLI, installed and run via Node.js
- **Runtime requirement:** Node.js v20+ (the project encountered a version mismatch with Node v18 installed at time of use)
- **Input source:** Cursor's internal `agent-transcripts` folder, stored as JSONL files per project
- **Output:** Organized Markdown files suitable for archival and knowledge vault ingestion
- **Scope:** Bulk export covering both Agent and Composer conversation types in one pass
- **Invocation:** Standard npm CLI pattern (e.g., `npx cursor-history` or global install)

## Key Design Decisions
- **CLI over manual copy** — The CLI abstracts away JSONL parsing and handles multiple conversation types uniformly, reducing manual effort and error risk
- **Node 20+ requirement** — The tool targets modern Node.js; backward compatibility with v18 was untested and remained uncertain at time of use
- **Markdown output** — Output format chosen for human readability and compatibility with Obsidian-style knowledge vaults

## Relationships
- [[Arcane knowledge vault]] — the destination for exported conversation content
- [[pgp-demo project]] — the Cursor project whose conversations were exported
- [[Cursor agent-transcripts]] — the raw JSONL source files the CLI reads from

## Conversations That Shaped This
- [[Project conversation export options]]