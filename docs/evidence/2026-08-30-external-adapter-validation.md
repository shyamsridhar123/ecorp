# Claude Code and OpenCode adapter validation — August 30, 2026

The provider-independent unit harness runs both adapters against
`scripts/fake-external-agent.mjs` and verifies session, stream, usage, artifact, completion, and
evidence behavior.

`node tools/e2e_external_adapters.mjs` runs the same mission through the real
server-to-runner-to-worktree path for `claude-code` and `opencode`. Both runs select the requested
adapter, persist a provider session and usage, produce normalized JSON evidence, and complete
through the normal verifier.

The fake provider executable is CI-only. Real use requires an authenticated CLI configured through
`CRONY_CLAUDE_COMMAND` or `CRONY_OPENCODE_COMMAND`.
