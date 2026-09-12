# Issue #50: isolated Factory budget recovery evidence

Follow-up: [native resume after missing checkpoint proof](2026-09-11-issue-50-missing-checkpoint-result.md)
extends this captured-checkpoint baseline. Hashes and untested cases below describe
the earlier test revision, not the subsequently extended harness.

## Result and limits

On September 11, 2026 (America/Los_Angeles), the existing runtime at
`b28fd4d26309794f38c0455bbf42d22aedf7cfd1` passed the synthetic Factory recovery
sequence with a successfully captured checkpoint. An approved budget revision
followed by explicit native provider resume reused the session and workspace,
passed persisted verification and independent review, and reached Factory
`verified`. The original run remained cancelled at `suspend` with its original
6,000-token usage; the resumed run consumed 12 additional synthetic tokens.

This is not a new production recovery fix and does not close #50. In particular,
it does not reproduce #235's missing/timed-out checkpoint, prove recovery of an
already terminal Factory item, prove the browser path, or enable autonomous work.
No Rust, application UI, migration, budget policy implementation, or terminal-state
guard was changed. #236's auditor policy is still future work.

## Source and resource identity

- Issue: https://github.com/All-The-Vibes/ecorp/issues/50.
- Contribution branch: `codex/issue-50-factory-recovery`.
- Code base: `b28fd4d26309794f38c0455bbf42d22aedf7cfd1`.
- Running operator checkout remained clean at `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
- QA root: `C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery`.
- QA PostgreSQL 17.7: loopback port `55450`, dedicated `pg-data`, user `ecorp_qa50`.
- QA API: `http://127.0.0.1:18450`; runner: `runner-qa-issue50`.
- Port `18550` was reserved but no web server was needed or started for this API/CLI reproduction.
- Source fixture: a tiny local Git repository whose origin identifies `All-The-Vibes/ecorp`;
  synthetic source commit `3ef4bdb76f5efd363fc2314804422e4db7ca90a1`.
  This is not an application implementation commit and was never pushed.
- Fake GitHub project: `ecorp-qa/50`, item `PVTI_QA50`, synthetic issue 9050.
  All GitHub commands ran through `tools/fake_github_cli.mjs`, without network publication.

The fixture reuses its receipt-owned PostgreSQL cluster and preserves per-attempt
databases, logs, worktrees, and JSON evidence. Loopback-only trust authentication
is for synthetic QA data only, not production security. No operator database URL
or credential was needed. Child environments are allowlisted, and the final
harness prevents parent `.env` discovery. QA enrollment was scoped to the QA
server; runner credentials were issued with a 30-minute lifetime. Development actor identities
exercise role separation but are not proof of production OIDC authentication.

## Native capabilities used

The native Codex adapter accepts `--codex-command <node.exe>` with
`--codex-command-arg <fake-codex-app-server.mjs>`. The fixture uses the adapter's
normal thread/start, thread/resume, usage, interrupt, artifact, and verifier paths.
It does not call a real model or introduce an alternative execution harness.

The fake provider gained one explicit, resumed-thread-only
`[budget-recovery-finish]` marker. Native resume correctly retains the original
task/mission text, including `[budget-stream]`; the finish marker lets this test
model bounded remaining work instead of emitting the original exhaustion stimulus
again. Three protocol tests verify the new finish case, unchanged initial usage,
and unchanged resumed overrun behavior without the finish marker.

## Successful recovery

Final report: `attempts/20260912044104617/report.json` beneath the QA root.

| Identity | Value |
| --- | --- |
| Database | `ecorp_qa50_20260912044104617` |
| Mission | `0ac1262d-8eb4-422b-91a1-7a0fe50c4b54` |
| Task | `b8ae2584-aab6-4b7b-9082-02b9d3771bbe` |
| Original run/workspace lineage | `4d192f88-bc96-4f24-a18a-eec53672cfc3` |
| Resumed run | `3cec5222-b38b-402e-8a63-056a1989ad0e` |
| Provider session, reused | `455632a1-b20a-4dcb-86c4-dc8059b84b92` |
| Factory item | `437f6a4c-d2a4-4284-9a3a-840cd6acf4dd` |

Observed sequence:

1. Native Factory dry-run passed before claim/materialization/launch.
2. Synthetic usage reached 6,000 tokens. The run, task and mission became cancelled;
   the run stayed at breaker `suspend`, with preserved workspace and captured fingerprint.
3. Resume without increased authority returned HTTP 409 and created no new run.
4. One-shot controller reconciliation reported `checkpoint_ready`, kept the existing
   Factory item `running`, and launched no provider. That state was a recoverable
   projection, not proof of active execution.
5. Owner approved a mission ceiling of 20,000 tokens and a narrowed 4,000-token finish.
   Proposal and decision replay reused the existing revision.
6. Explicit native resume reused the provider session, workspace lineage and source.
   A duplicate resume returned HTTP 409; no third run was created.
7. Persisted checks required both an artifact and `resumed.txt`. The file is only
   written by the resumed synthetic provider, so checkpoint-only verification cannot
   satisfy this test. Requester self-review returned HTTP 403; the distinct synthetic
   reviewer approved after the checks passed.
8. The resumed run, task and mission completed. Controller reconciliation reached
   Factory `verified` at version 17. Total synthetic usage was 6,012 tokens; the
   source run's 6,000-token usage and `suspend` history were unchanged.

An earlier successful report, `attempts/20260912043550490/report.json`, independently
observed the same successful path before the additional authorization assertions.

## Negative cases

Hard-stop report: `attempts/20260912043955367/report.json`.

- Original source run: `0a086726-6853-4683-9656-8e71adae64a9`.
- Resumed run: `0f2da6d9-7c06-4ae4-86aa-7ada0db49e1d`.
- Revised mission limit: 10,000 tokens; finish limit: 4,000 tokens.
- Resumed synthetic usage: 6,000 tokens. The run cancelled at breaker `stop`.
- Ancestor resume returned HTTP 409: `provider workspace lineage reached a stop-stage breaker and cannot be resumed`.
- No extra run was created and no run was accepted as completed.
- The Factory projection remained `running` with checkpoint-oriented reconciliation;
  this must not be interpreted as live provider work or permission to bypass `stop`.

Both final test paths rejected member budget proposals/decisions (HTTP 403),
replayed the same proposal/approval identities, and rejected a stale decision
version (HTTP 400, the current API contract), leaving that proposal pending until
the valid decision. Successful recovery also rejected requester self-review.

Source drift, quarantined workspaces, lost network responses, process crashes,
missing checkpoints, expired claims, production identity isolation, and the UI
were not exercised by this fixture. Duplicate calls are not a complete network
fault-injection test.

## Commands and validation

From the isolated contribution worktree:

```powershell
cargo build --locked -p crony-server -p crony-runner -p crony-cli
node --check tools/e2e_factory_budget_recovery.mjs
node --test tools/fake_codex_budget_stream.test.mjs
cargo test --locked -p crony-cli issue206_
node tools/check_migrations.mjs
cargo fmt --check
git diff --check

$env:ECORP_ISSUE50_QA_ROOT = 'C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery'
$env:ECORP_ISSUE50_PG_BIN = 'C:\Users\aabdelsalam\.ecorp\pg\pgsql\bin'
node tools/e2e_factory_budget_recovery.mjs --dry-run
# The operator approved this exact QA resource/effect scope before execution.
node tools/e2e_factory_budget_recovery.mjs --execute
node tools/e2e_factory_budget_recovery.mjs --execute --overrun
```

- Binaries built successfully in the contribution worktree's own target directory.
- Synthetic provider tests: 9 passed, 0 failed.
- Existing #206 checkpoint/controller tests: 7 passed, 0 failed.
- Migration check: 40 immutable migrations through version 40, passed.
- Formatting, JavaScript syntax, and tracked whitespace checks passed.
- The complete workspace clippy/test/web gates and browser-to-server-to-runner path
  were not run. No commit, PR, merge, deployment, or hosted-CI result is claimed.

Initial fixture attempts failed on empty HTTP response parsing, fake repository
case matching, an omitted mandatory review policy, source/runtime admission timing,
and an incorrect expected HTTP status for stale versions. These were fixture
setup/assertion failures, not successful product tests. Their reports were retained.
All eight attempts stopped their three verified QA-owned processes and reported
unchanged retained-stack baselines; none left a QA listener behind.

Final harness SHA-256:
`B6272C8A38280F8FF66004ACF14A00D21A262732D248C95E41DE327A385E099F`.
Fake provider SHA-256:
`C68C9DE6FB82FEC5CDC87EE1EFFC02C5D091C87AE1600816371E5F5BD80BCDCD`.

## Retained-stack check and next decision

Before/after comparisons retained two missions, two tasks, two runs, one Factory
item, no controller, and no publication. #235 remained cancelled at `suspend`, with
208,704 input plus 960 output tokens (209,664 total), preserved workspace, and no
checkpoint fingerprint. Its budget, provider identity, source checkout and
credentials were not changed or resumed.

The evidence supports using existing recovery mechanisms when their prerequisites
hold; it does not support weakening terminal-state guards or resetting usage.
Next investigate the missing-checkpoint/terminal-projection case separately, then
prepare a reviewed runtime-upgrade/recovery preview for the retained stack. Keep
#50 open and #236 dependent on its remaining recovery acceptance work. Do not
enable the broad watcher or resume #235 based on this synthetic success.
