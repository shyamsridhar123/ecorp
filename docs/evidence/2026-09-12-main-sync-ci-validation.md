# PR #237: latest-main synchronization and CI fixture validation

Date: September 12, 2026. This is local, deterministic validation, not deployment,
real-provider inference, hosted-CI acceptance, or completion of issue #50.

## Authorized source and scope

The operator approved synchronizing the current `All-The-Vibes/ecorp` main into a
new isolated integration worktree, preserving the existing contribution and live
factory, and revalidating the remaining CI changes before publication.

- Contribution parent: `273114b1fe5aa850f8767e1c9062c5100d885a7f`, the published
  head of draft PR #237, `codex/issue-50-factory-recovery`.
- Main parent: `b2523964e7576cafc00e84a51e1044f55826dea7`.
- Common ancestor: `b28fd4d26309794f38c0455bbf42d22aedf7cfd1`.
- Integration branch: `codex/issue-50-main-sync`.
- Integration worktree:
  `C:\Users\aabdelsalam\.ecorp\contributions\issue-50-main-sync`.

The fetch dry-run and object-only merge preview preceded the approved integration.
The preview identified 12 conflicts, all in CI workflows and test fixtures. The
application code, web code, Cargo lockfile and 41 migrations match the main parent;
no additional production implementation or budget-policy changes were introduced.
The source/merge-parent tuple above identifies the working merge candidate tested
before its local commit. The commit containing this report records both parents.

## Reconciliation decisions

- Preserve upstream's late-completion budget assertions verbatim, including the
  actual retained `result.md` read. The obsolete `failed` expectation is already
  corrected on main; the abandoned duplicate local assertion rewrite was not
  imported from the previous dirty worktree.
- Add explicit budget-fixture opt-in, API/database target guards, exact runner
  and quiescence checks, bounded no-redirect HTTP, a genuine network-free dry-run,
  and refusal to overwrite an earlier report. Configuration guards do not prove
  database ownership: the existing CI/native supervisor verifies that boundary.
- Use upstream's model/source-bound identity and artifact protocol probes.
  Reuse the shared bounded native preview barrier and exact assignment checks;
  retain listener/timer cleanup, stale-epoch/wrong-token/revocation assertions,
  and clearly labeled Windows lifecycle/readiness-only lanes.
- Use upstream's native source-selected graph staffing. Remove the obsolete
  fixture SQL that disabled demo workers. Windows still executes the original
  mixed Codex/Claude graph in addition to the source-selected graph and retries.
- Use upstream's checked Git-origin identity helper for factory fixtures, their
  policies and URLs. Preserve pre-effect readiness and native automatic Factory
  verification checks. Synthetic GitHub transport remains local and explicit.
- Retain receipt-owned CI startup/restart/stop, database-target continuity,
  operation locks, and Linux pidfd-only signaling. Incorporate upstream's bounded
  artifact-recovery environment overrides and explicit-port mismatch rejection.
  No numeric-PID Linux signal fallback or manual-port exception is introduced.
- Retain upstream's real HTTP/child driver regressions in
  `tools/e2e_external_adapters_http.test.mjs`, alongside the earlier injected
  driver tests, actual runner OS/capability checks and full Windows lifecycle.
  Unsupported dispatch must leave both the run set and execution journal empty.
- Retain all platform jobs and full Linux OIDC/RBAC, secret, approval, artifact
  fault/restart and factory/publication suites. The test-only API/database ports
  remain `18471`/`55471`; Windows native runner tests remain unfiltered and serial.

These changes reuse native ECorp planning/assignment and existing OS ownership
primitives. They do not introduce a second execution harness or grant a provider
more authority.

## Completed local validation

- `node tools/check_migrations.mjs`: 41 immutable migrations.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo build --locked -p crony-server -p crony-runner -p crony-cli -p crony-gateways`: passed.
- `cargo test --locked --workspace -- --test-threads=1`: 547 passed, 323 explicitly
  ignored. The runner component passed 213 with one opt-in test ignored, in
  334.65 seconds. The two earlier Windows connection-fixture timeouts passed.
  Ambient `DATABASE_URL` was removed; ignored tests were inventoried with
  `--ignored --list`, not executed or counted as passes.
- All eleven CI Node regression files: 121 passed, zero skipped, including the
  native temporary child startup/two-restart/idempotent-stop case and retained
  upstream HTTP-driver cases. `ECORP_OWNED_PROCESS_TEST=1` explicitly opted in.
- All `apps/web/src/*.test.mjs` plus `tools/fake_codex_budget_stream.test.mjs` and
  `tools/factory_budget_fixture_config.test.mjs`: 265 passed, zero skipped.
  These are web-state/component and deterministic fixture tests, not a rendered
  browser-to-server acceptance run.
- `python3 -B tools/owned_test_process_linux_test.py -v` in existing Ubuntu WSL:
  nine native pidfd regressions passed. Only temporary Python/socket children
  were used; no PostgreSQL, Docker, ECorp or real-provider process was started.
- `pnpm install --frozen-lockfile --offline`, `pnpm build:web`, `pnpm lint:web`:
  passed, with dependencies reused from the existing cache.
- Node syntax, PowerShell AST, parsed workflow YAML/environment mappings and
  `git diff --check`: passed. The initial AST probe was blocked by sandbox
  constrained language mode and was not counted; the explicit read-only
  full-language rerun succeeded.

Intermediate merge-validation failures were retained in the task transcript:
24/29 focused tests exposed a test object overwriting its probe/socket; 118/119
then exposed the missing explicit-port consistency check. Both were corrected
before the final 121/121 run. Neither required a production-code change.

## Owned native QA

The existing `tools/ci_external_adapters_windows.ps1` supervisor was invoked with
`-DryRun`, then the approved `-Execute`, using:

- Fixture: `C:\Users\aabdelsalam\.ecorp\qa\ecorp-external-adapters-ci-20260912j`.
- API: `http://127.0.0.1:18453`; native PostgreSQL port: `55453`.
- Start: `2026-09-12T21:21:28.5719812Z`.
- Finish: `2026-09-12T21:23:10.9866986Z`.
- Independent synthetic source commit:
  `4f083c02f9a1a3781aa2d242ef55b65d5c1e4c48`.

The Windows external-adapter lifecycle, native source-selected graph, original
mixed-provider graph, bounded retries, controlled artifact readiness, identity
lifecycle and complete budget matrix all passed. Both graphs observed two
concurrent roots, three distinct worktrees, verified dependency handoffs and
accepted synthesis. The source checkout remained unchanged.

Identity probe `identity-probe-27b9dd77-db8a-4aeb-a14d-f993ae4a54d7` needed nine
read-only previews and received the exact model/source assignment. Rotation,
replay, epoch/token fencing and active revocation passed. Revocation produced
run `lost`, task `blocked`, mission `failed`, agent `idle`. This protocol fixture
reports `provider_execution: false` and `oidc_executed: false`.

The budget fixture measured synthetic usage of 6,000 against the unchanged
5,000-token test ceiling. The fake provider's native terminal outcome was
`completed`, but ECorp retained exactly one cancelled run/attempt at `stop` with
pending verification. The actual source file was retained, artifact events and
accepted completion events were both zero, and a late approval returned `400`.
Spend/loop/requester/Corp stages and the healthy-conversation exemption passed.

The supervisor verified shutdown of its runner, server and PostgreSQL. An
independent listener check then confirmed zero listeners on both test ports.
Data, credentials, worktrees and evidence remain retained in the owned fixture.
Real provider calls and real GitHub mutations were zero.

Evidence SHA-256 (under the fixture's `evidence` directory):

| File | SHA-256 |
| --- | --- |
| `fixture-report.json` | `a49d0a863c795db891dc7bf4473e31b1bf0ffd2a759a5efdfda63c503a9a2bcf` |
| `e2e-budgets.json` | `5d94bcc2d99262ad67761bbe762799bff6eede514cf9560c7a3de0caabf48892` |
| `e2e-identity-lifecycle.json` | `46597173e4823b093ab00497a82c0d95250bea2dcb25cd920cef78e83d4bdd32` |

## Preserved work and remaining gates

The configured source checkout `C:\dev\ecorp` remains clean at `971445e1`.
The existing `issue-50-factory-recovery` worktree remains at `273114b` with its
pre-existing pending recovery files and interrupted local CI draft preserved.
The four protected recovery-file hashes remain unchanged. They were not copied
over, committed, or executed as part of this integration.

The live factory, its database on `54329`, API/UI on `8791`/`5187`, original
runner identity and credentials were not restarted, migrated, re-enrolled or
dispatched. No budget was raised, no breaker disabled, and no pending work was
resumed. Secret values were not disclosed; trusted environment delivery retains
its reduced-assurance label.

PR #237 is still a draft review contribution. Publication requires the operator's
separate approval after the personal-account push dry-run. The combined candidate
has not run hosted CI yet; full Linux integration/OIDC/artifact fault and
publication recovery remain hosted gates. Local Windows results and Linux-native
ownership tests do not substitute for that complete hosted chain. Merge,
auto-merge, deployment, issue closure and autonomous factory intake remain off.
