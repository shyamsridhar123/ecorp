# ADR 0018: Normalized Claude Code and OpenCode CLI adapters

## Status

Accepted — August 30, 2026

## Decision

Claude Code and OpenCode use one normalized external-CLI adapter. It checks availability, launches
inside the assigned worktree, parses JSONL or text, normalizes sessions and usage, and writes a
provider-neutral evidence artifact with a stdout digest.

Claude Code is invoked in print mode with streamed JSON and `acceptEdits` permission mode. OpenCode
is invoked with `run --format json`. Resume uses each provider's session flag.

Live steering remains explicitly unsupported in batch CLI mode. Stop and interrupt terminate the
supervised process. Crony's durable approval policy remains authoritative.
