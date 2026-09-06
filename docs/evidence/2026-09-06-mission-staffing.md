# Mission-owned Copilot studios — September 6, 2026

## Scope

This is a bounded milestone of #48 and the real-provider continuation of #146.
It also reconciles review findings on the #150 / #154 / #156 stack.
It does not establish every dark-factory, production-isolation or multi-human
authentication requirement.

The product uses server-created mission workers, native GitHub Copilot sessions,
isolated task worktrees and verified typed source handoffs. No worker rows,
task states, game implementation or fake provider results were manually inserted
to produce the real-provider observations below.

## Local product validation

- Immutable migration checker: 35 migrations.
- Strict workspace/all-target Clippy passed.
- Rust workspace: **192 passed**, zero failed or skipped in the serial run.
  The preceding parallel run had two unchanged Claude permission-fixture startup
  timeouts. That failing log is retained; its failures are not reported as passes.
  Serial execution changed test scheduling, not assertions or product permissions.
- Web build/lint and the focused office/runtime tests passed.
- Copilot probe helper regressions: **35 passed**, zero failed or skipped.
- Staffing public-API fixture: 12 workers, 6 missions, 14 tasks, exactly one
  deterministic process run; zero orphan identities. Repeated factory preflight
  created no identities; materialization replay created no duplicate graph.
  All five held plans retained zero runs. Unpinned workers were not reused across
  missions; the completed worker retired without deleting its run/history.
- The browser created and dispatched exactly one additional source-selected
  fake-process mission. The first bounded browser pass timed out after observing
  creation/launch; it is not labeled a full browser-suite pass. A separate
  read-only final pass observed completed/retired state and historical identity
  at desktop and 390px, with no console errors or horizontal overflow.
- The browser exposed inherited dark evidence/workspace fills beneath the light
  skin. The corrected surfaces, status chips and artifact controls were rechecked;
  sampled text contrast exceeded 7:1.

Local fixture evidence:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue48-staffing-20260906\fixture
```

Fixture execution is not counted as real Copilot work.

## Real-provider observations

The execution target is the separate private repository
`shyamsridhar123/ecorp-arcade-lab`, issue #1, in ECorp Build Project #3.
It started with a brief and contributor boundary, not a prebuilt game.
Repository auto-merge is disabled and there are no Actions workflows.

| Identity | Value |
|---|---|
| Source ref | `main` |
| Immutable source | `d6343f598280e986c43a32082e7f361a9af9ce22` |
| Mission | `b6d1caca-6ef8-4b63-b2e4-3a5eb3f14f83` |
| Runner | `issue48-real-copilot` |
| Runtime | Copilot Rust SDK 1.0.11 / CLI 1.0.79, automatic updates disabled |
| Model | `gpt-5.6-sol`, high reasoning, all four tasks |
| Integration run | `221e3169-2566-4a53-9a0f-bfa5a8a3277c` |

Three distinct real Copilot workers were observed in `working` concurrently.
The rendered floor showed all three working. The browser was then closed while
their execution continued independently.

| Specialist | Provider session | Verified exported file SHA-256 |
|---|---|---|
| Visual | `335ea9f4-f6b3-47c0-9e30-e55cfd3a0625` | `4fc4967ee88bfc1d67d7763f9f58f28e3a8e87875fef260572e17637366db7fa` |
| Gameplay | `9fdc46ec-0771-42b5-86de-30d5d51d5537` | `6c7c80d6009638b4d7a3fbf87906fb4d76c5b1ddbb260a2740ab4cf9a884624e` |
| Quality | `64538f6e-2894-4806-a8e4-1a55507ae50b` | `166b3d49a3cb83c50c31ac4df6fce386b7321231962d40d680fd7e842b692bbb` |

Every root passed its persisted file, artifact and UTF-8/12-KiB checks. The
source deliverables were downloaded through the artifact API and independently
checked against their signatures, envelope hashes, base commit, file bytes and
file hashes. These hashes identify exported Git-normalized source bytes.

An independent AI review found usable handoffs, with mandatory reconciliation
of the dark palette proposal, competing engine interfaces, invalid fixture
coordinates, edge-case semantics and keyboard focus. The requesting operator's
guidance was posted as a mission-linked room message and separately queued to
the integration worker. Neither message itself created or dispatched a task.

The local **Bob development test principal** exercised the root review gates
after that AI review. This is explicitly **not human sign-off or proof of
separate GitHub user authentication**. Notes on the decisions retain that scope.

Integration started only after all three accepted root completions. Its durable
`run.dependency_context` receipt contains all three source artifact/run/file
hashes; its complete context digest is:

```text
5f61a37b8bc5e156d59a3f2e78ea844711b78dc934b860b5d01acdaad2953f3a
```

At this checkpoint, no routine action approval had been requested. Final game
verification, review and PR publication are not claimed by this checkpoint;
their outcome must be recorded separately before closing the game work.

Local real-provider evidence:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue48-staffing-20260906\live
```

## Review corrections and retained gaps

- #150: credential-canary coverage now requires the exact seed, owned runner
  spawn and provider-parent chain. Missing observations cannot become a pass.
  Existing inconclusive external-write coverage is not relabeled.
- #154: selecting another studio's agent reveals that studio; a remount cannot
  replay the same arrival; blocked work leads to recovery rather than new work.
- #156: new dispatch failures are preserved before fresh/replayed success can
  be returned.
- New review corrections: same-room pinned-worker reuse, agent-wide resume
  exclusion, and verified dependency context on resumed integration.

#48 remains open for complete Clear crew / Pin / Unpin / manual Retire controls
and their full workflow. #144/#51 retain broader approval/isolation requirements.
#142's residual non-floor UX is not fully superseded by the new floor. #143/#148
recovery WIP remains preserved and requires its own reviewed integration; this
branch's migration 0035 must be considered without rewriting an already-applied
recovery database's history. No game source is added to the ECorp repository.
