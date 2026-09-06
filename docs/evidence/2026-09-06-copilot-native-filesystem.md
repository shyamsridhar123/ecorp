# Copilot native filesystem and application evidence

**Date:** September 6, 2026

**Work:** #144, #145, PR #150

**Hosted Actions:** Not used.

## Containment correction

Two new regressions failed against the previous provider: a dangling symlink created an external
file, and an append through a hard link modified an external sentinel. Both now fail closed.

The corrected provider uses retained `cap-std` directory capabilities and no-follow file opens,
checks the opened object before truncation or append, rejects multiply-linked files and unsafe
path aliases, and enforces the persisted task write scope. Reads and writes do not rely on a
lexical path check followed by ambient filesystem access.

The first real application attempt exposed another integration gap: Copilot's built-in create
tool refused missing parent directories before calling the filesystem writer and then fell back
to PowerShell. The SDK-registered `ecorp_mkdir` primitive now creates only scope-authorized
directories. It cannot run commands, delete files, change permissions, or access arbitrary paths.
Copilot continues to use its own create/view/edit tools for file contents.

All model-session shell remains approval-gated. Native sandbox configuration is not treated as
proof that the operating system applied isolation.

## Accepted real application

The official Rust SDK `1.0.11` and real Copilot CLI `1.0.79` built **Piper Kingdom** using the
connected account's enabled `gpt-5.6-sol` model. No application code was written by the acceptance
harness or copied into the ECorp product repository.

| Evidence | Result |
|---|---|
| Mission | `cfd59cc5-d43e-4b15-b0c9-c577823f3308` |
| Run | `9d1cfd88-a2cf-4bb6-80a5-d7236c214dab` |
| Provider session | `db76bbc6-2876-4d40-baa6-ab9f97ededd8` |
| Source base | `16fedea273cf2e7a51b35769a61f1eb54e83096b` |
| Write scope | `scenarios/piper-kingdom/**` |
| Attempts | One, no repair |
| Routine durable approvals | Zero |
| Persisted verifier | Six of six checks passed |
| Usage | 72,911 input and 7,692 output tokens |
| Configured source checkout | Unchanged |
| Source deliverable | `4cf9fc2f-0a7c-4d24-b2f0-7aaed911a56c`, ready for review |

The verifier checked the provider artifact, HTML, stylesheet, JavaScript syntax, generated Node
tests, and independently authored gameplay assertions. The last check required the original
board, non-mutating moves, both coins, and winning at the actual exit `(7,1)`.

The application and runner artifacts are beneath:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue144-secure-20260906\probe-005
```

The source archive was downloaded through the authorized server route and independently
SHA-256 checked:

```text
49,604 bytes
f6e385d90fb93e7805dc8a5e9e00be374a44e7d5545f3eb9231824569839f5f4
```

The rendered ECorp UI displayed `Completed`, `6/6 checks passed`, the archive and verification
digests, and `Ready For Review`. Its source-download action reached the server without a browser
console error. Publication and merge remained explicitly unrequested.

## Interactive browser checks

The actual generated game was served from its preserved worktree and exercised in a browser:

- a wall collision left position and move count unchanged;
- the valid keyboard route collected two coins and won in ten moves;
- another move after winning changed nothing;
- Restart reset the game, and the touch-style right button moved the player;
- desktop `1280 × 900` and narrow `390 × 844` views had no horizontal overflow;
- all five controls measured at least `44 × 44` CSS pixels;
- keyboard focus had a solid four-pixel outline;
- reduced-motion emulation matched, reducing coin animation to one `0.00001s` iteration;
- no browser console errors were recorded.

Temporary viewport and media overrides were restored. The machine-readable observations are in
`probe-005/browser-verification.json` beside `piper-kingdom-build.json`.

## Failures were not converted into success

An earlier `gpt-5-mini` mission authored incorrect winning routes in its own tests. The persisted
verifier rejected it. A later repair attempted to win beside the exit instead of at the required
exit; the independent gameplay check rejected that change.

That lineage eventually reached a hard budget stop at run
`964c7983-6eea-43ca-b299-0a6230daa1d3`, with 143,532 run tokens against 118,994 remaining
authorized tokens. Late artifact upload was rejected. The stopped lineage and its preserved
source were not resumed, reset, or relabeled as accepted. The successful run above was a
separate, newly authorized comparison mission from the original clean source.

Verifier-only recovery of useful source at this boundary remains #148. This document does not
claim that recovery acceptance is complete.

The stronger application model ended a destructive probe without the required durable approval.
A separate `gpt-5-mini` boundary-only invocation likewise ended its external-path case without
that approval. The conformance harness rejected both as insufficient permission-handler coverage;
it did not turn an early model refusal into a passed safety test. The complete live negative
matrix is therefore still **incomplete**. Generic shell and the stronger model's external-write
case did reach a durable request and explicit rejection.

A deterministic SDK-handler test separately exercises eight exception classes: a shell request
marked read-only, a pipeline, network shell, configured-source write, destructive shell,
credential shell, managed-policy escalation of a contained read, and an unknown future tool.
Each must emit an approval request, remain pending, and return the explicit rejection. This
handler-level evidence is not presented as a substitute for the incomplete live matrix.

## Local quality gate

- 34 immutable migrations.
- `cargo fmt --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace` with `RUST_TEST_THREADS=1`: 147 tests passed.
- Four deterministic process-observer tests passed.
- Seven native-filesystem tests passed in a local Linux Rust `1.94` container, including Unix
  executable modes, dangling/ancestor links, hard links, and scope enforcement.
- Web build, web lint, and diff checks passed.

The final default-parallel invocation encountered five timeout-related failures in external-CLI
process tests. The unchanged complete suite passed when run serially; no test was removed and no
timeout assertion was weakened. Both logs were retained outside the repository. The earlier
146-test parallel gate, before the additional exception-handler matrix, also passed.

The Linux exercise also exposed a test-only borrow error in the pre-existing verifier executable
resolution test. Borrowing its path rather than moving it allowed the actual Unix mode tests to
run. This did not change verifier command execution.

## Scope boundary

The accepted application proves native scoped create/read/edit/directory operations and
runner-owned verification without routine approvals. It does not prove approval-free arbitrary
shell, complete native-permission audit projection, all provider families, full Factory-to-real-PR
acceptance, verifier-only checkpoint recovery, or every remaining #144/#145 UX requirement.
No generated game was pushed into the ECorp repository. PR #150 contains product changes,
regression tests, and evidence only; auto-merge and deployment remain off.
