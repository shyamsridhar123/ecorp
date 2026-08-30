# Worktree isolation validation — August 29, 2026

## Scope

This record covers issue #12: dedicated branches and linked worktrees, path containment, source
checkout preservation, resumable workspace identity, and fail-safe cleanup.

## Unit coverage

Runner tests create source and managed directories whose names contain spaces and verify:

- two runs receive different `crony/task-.../run-...` branches and worktrees
- changes in one worktree do not appear in the source checkout or sibling worktree
- preparing the same task/root-run pair reuses the exact dirty worktree
- dirty and untracked work is preserved
- ignored files are preserved
- committed work ahead of the base is preserved
- clean work is reclaimed after fast-forward integration
- clean empty worktrees and branches are removed
- detached or unverifiable worktrees are preserved
- occupied non-worktree targets fail without shared-checkout fallback
- unsafe refs and paths fail closed

## Full-path validation

`tools/e2e_worktrees.mjs` ran through HTTP, Postgres, the runner WebSocket, both adapter types, Git,
and artifact verification.

- The source checkout's HEAD and porcelain status were identical before and after.
- Parallel fake-process run `dfff16a7-ff87-4716-b875-85ddfd556870` and Codex run
  `d8755ecd-b9cb-43f7-9bdd-a23b5ec87c5c` used different task directories, worktrees, and branches.
- Both dirty worktrees finished as `preserved`.
- Clean evidence-only run `ad955120-96c4-4e5c-898c-72cbe920fd74` finished as `removed`; its linked
  worktree no longer existed and its branch ref was deleted.
- Ignored-output run `41919cf2-eb0e-44b0-be71-db933454c201` remained `preserved` because
  `valuable.log` was intentionally ignored by Git but still potentially valuable.
- Artifact bytes and SHA-256 values remained verifiable after lifecycle cleanup.

## Authenticated real-provider validation

Codex CLI `0.150.0-alpha.8` also ran against the production app-server adapter:

- Initial run `91457004-2991-4346-a398-aa2e41b53028` created `worktree-proof.txt` in branch
  `crony/task-90243bb360df4d7d804d7652f6000029/run-9145700429914346a398aa2e41b53028`.
- Resume run `ed0f9577-3b63-4dae-a0e7-e6972bc74786` reused that exact branch, path, provider
  thread, and root workspace ID, then created `resume-proof.txt`.
- The source checkout's HEAD and porcelain status were unchanged; neither proof file appeared
  there.
- The initial and resumed evidence SHA-256 values were
  `b24465d7d672c605cf6c3a1f8a495e7d900f49910891558709ab2989ae659221` and
  `e1a6de3b68ceb0173e9c08af73dcfc693645386064a975b3832e7c3e8f6e0b12`.

## Safety boundary

The current implementation preserves work but does not yet garbage-collect preserved branches.
Any future collector must prove a clean tree plus integration into the current base before removal.
