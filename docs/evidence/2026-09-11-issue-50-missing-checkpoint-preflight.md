# Issue #50 missing-checkpoint preflight

Status: test prepared and dry-run reviewed; missing-checkpoint stack execution has
not been performed. This extends the earlier captured-checkpoint recovery test.

Follow-up: the operator subsequently approved execution. The
[result report](2026-09-11-issue-50-missing-checkpoint-result.md) records the passing
unchanged-README injection and two earlier attempts. This preflight preserves the
original proposed `base.txt` target, which was refined after it intercepted adapter
evidence collection rather than checkpoint capture.

## Current retained system

Read-only checks on September 11, 2026 (America/Los_Angeles):

- Web `http://127.0.0.1:5187`: HTTP 200.
- API `http://127.0.0.1:8791/health`: healthy; one connected `runner-local`.
- No Factory controller and no active run; this is an available control UI, not
  active autonomous contribution.
- Main checkout stayed clean at `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
- #235 stayed cancelled at `suspend`, with 209,664 tokens and null checkpoint fingerprint.
- Its preserved worktree stayed on the original commit with clean Git status;
  the ignored-path metadata check reported no ignored paths. This is not a
  replacement for a checkpoint captured at termination.

GET of #235's native Factory verification-recovery context returned HTTP 400:

```text
source preservation has no stopped-source checkpoint proof
```

No recovery command, budget revision, enrollment, controller start, database write,
or provider resume was sent to the retained stack.

## Mechanism and uncertainty

The native stopped-source capture has a ten-second overall deadline and separate
read/size bounds. On failure it preserves the source without inventing a fingerprint.
Checkpoint-only verification requires the persisted native preservation proof and
provider-termination evidence. The other checkpoint-capture operation rejects
suspended/stopped lineages; it must not be used to fabricate an earlier checkpoint.

Explicit native provider resume is a distinct path. The prepared test will determine
whether a missing original proof blocks that path too, or only checkpoint-only
verification/controller reporting. No result is assumed in advance.

## Prepared fault injection and preview

Contribution branch: `codex/issue-50-factory-recovery`, base
`b28fd4d26309794f38c0455bbf42d22aedf7cfd1`. Changes are test-only:

- `tools/e2e_factory_budget_recovery.mjs --missing-checkpoint`.
- `tools/qa_hold_checkpoint_file.ps1` (bounded, hidden QA helper).
- Synthetic provider `[checkpoint-read-lock]` handshake and two protocol tests.

The helper will open only the synthetic worktree's `base.txt` read-only with an
exclusive Windows sharing mode. It changes no file bytes. The provider waits for
the helper's marker before reporting budget usage. Native checkpoint capture must
then report its own read failure and retain the workspace without proof. The host
releases the handle before testing approved native resume. No persisted checkpoint
or database row is manually altered to manufacture the result.

This induces a missing-proof state through a read failure, not the exact historical
ten-second timeout. A passing test would not establish the cause of #235's timeout.

Preview command, executed successfully without starting QA services:

```powershell
$env:ECORP_ISSUE50_QA_ROOT = 'C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery'
$env:ECORP_ISSUE50_PG_BIN = 'C:\Users\aabdelsalam\.ecorp\pg\pgsql\bin'
node tools/e2e_factory_budget_recovery.mjs --dry-run --missing-checkpoint
```

Execution scope awaiting the post-preview approval: reuse the existing owned QA
cluster on 55450, start the QA API on 18450 and synthetic runner, create a separate
per-attempt QA database, run the controlled missing-proof/revision/resume sequence,
then stop only verified QA-owned processes. Retain reports/data. No model inference,
real GitHub changes, publication, production credential access, or #235 mutation.

## Validation completed

- `cargo test --locked -p crony-runner source_checkpoint::tests::issue190_ -- --test-threads=1`: 17 passed, 0 failed.
- `node --test tools/fake_codex_budget_stream.test.mjs`: 11 passed, 0 failed.
- JavaScript syntax, PowerShell helper syntax and tracked/untracked whitespace checks passed.
- The helper's actual OS-level read-lock effect and new full-stack case are not yet verified.
- No full repository acceptance gate, browser exercise, commit, PR or deployment is claimed.

Unit fixtures created and removed only their validated temporary fixture directories.
The retained stack, operator source checkout, runner identity, credentials and #235
were not changed. The separate QA ports remained unused at the final dry-run.
