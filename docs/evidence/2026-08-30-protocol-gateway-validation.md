# Protocol gateway validation — August 30, 2026

`crony-gateways` contains three binaries:

- `crony-mcp`: JSON-RPC over stdio with scoped tools and structured results;
- `crony-acp`: local session initialize/new/prompt/load/cancel mapping;
- `crony-a2a`: HTTP agent-card, JSON-RPC task/message methods, and SSE streaming.

Unit tests verify version negotiation and ensure private internal field names are absent from public
descriptions. `node tools/e2e_gateways.mjs` exercises all three binaries against a live Crony server
and validates MCP tools, an ACP-created mission/run, A2A discovery, message submission, and
streaming event frames.
