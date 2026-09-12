# PR #234: external-CLI integration contract

Observed September 12, 2026 UTC / September 11, 2026 America/Chicago.

## Failure and repair

PR #234 at `97012e2e9dc0bbed21578e24b21700c4bfdb6097` changes only the
Factory and hard-stop E2Es. Its hosted integration run `34661083625` failed
earlier, in the unchanged external-adapter E2E: Claude Code launch returned
409 because that adapter was unavailable.

This was not a request to enable unsafe execution. The native external adapter
sets `platform_supported = cfg!(windows)`. SECURITY.md requires Unix refusal
because a process group cannot provide the required descendant containment.

The maintainer follow-up:

- Reads the connected fixture runner's native OS from the authenticated Corp
  snapshot. A configured platform expectation must match that observed OS.
- On Linux/macOS, requires the exact unavailable-adapter 409 and proves that
  the owned task has no persisted run or execution journal.
- On Windows, retains provider execution, session/usage evidence, signed
  artifact download, and distinct artifact-digest assertions.
- Fails for an unrelated denial, unexpected admission, a leaked run, an
  unknown platform, or an expectation that tries to disguise a Windows runner.
- Reaps the test-owned HTTP fixture child before deleting its temporary
  directory, with bounded termination and preservation on uncertain cleanup.
- Runs the HTTP-contract regression in the existing quality job.

The original contributor's two test-file changes are preserved. No adapter,
permission, verifier, budget, source, retry, or publication policy was relaxed.

## Local checks

All application/native build inputs and frontend inputs remained unchanged
through the following complete checks:

| Check | Result |
| --- | --- |
| `node tools/check_migrations.mjs` | 40 immutable migrations |
| `cargo fmt --check` | PASS |
| Offline, locked workspace/all-target Clippy, warnings denied | PASS |
| Offline, locked serial Rust workspace tests | 452 passed, 0 failed, 200 intentionally ignored |
| All 13 recursive frontend test files | 194 passed, 0 failed |
| Native `pnpm build:web` / `pnpm lint:web` | PASS |
| Explicit native server/runner/CLI/gateway build | PASS, six compiler-reported binaries |
| Final external-adapter HTTP-contract regressions | 10 passed, 0 failed |
| JavaScript syntax / scoped whitespace | PASS |

The original driver regression retained 3 passed / 4 failed before its fix.
Independent review found that an environment override could hide Windows
unavailability; two additional cases reproduced that false green (0/2), then
passed after binding the mode to the actual runner. The final 10-case suite
also exercises termination of a child stuck waiting for HTTP.

The final review corrections change only the E2E driver and its Node regression.
They do not change the Rust/frontend inputs to the completed repository gate.
These Node cases use a synthetic HTTP server; they are **not** native provider
execution or proof of vendor-session persistence.

Local receipts and retained failures are under the task-owned
`merge-drain-20260911` evidence directory. The native binaries were copied to
an immutable, hash-checked location before reusing the task-owned build cache.

## Remaining verification boundary

The local Docker engine was unavailable. One ordinary Docker Desktop start
attempt timed out; no replacement container, database reset, manual-app change,
or alternate route around an earlier QA restart denial was used. Therefore this
follow-up does not claim a fresh local database-backed E2E or browser run.

The PR's original contributor-reported Factory/budget E2Es remain separate
evidence. Fresh hosted results must be read from the current commit's checks,
not inferred from old jobs or from this report.

This is a maintainer review correction. The original Factory publication and
its verified commit remain historical provenance; they are not rewritten to
attribute these additional changes to the producing agent.

No merge, auto-merge, branch-protection change, deployment, or issue completion
is asserted by this report.
