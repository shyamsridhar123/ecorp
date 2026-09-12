# Issue #50: native resume after missing checkpoint proof

## Result

The approved isolated missing-checkpoint test passed on code base
`b28fd4d26309794f38c0455bbf42d22aedf7cfd1`. Native checkpoint-only verification
rejected the missing proof, but a separately approved budget revision followed by
explicit provider resume recovered the same session and worktree. Persisted
verification and independent review passed; Factory reached `verified`.

This refines the earlier diagnosis: missing checkpoint proof is not by itself a
blanket blocker to provider continuation. It remains a blocker to checkpoint-only
verification, and it is not permission to invent historical proof, bypass a hard
stop, or automatically resume a retained live mission.

Report: `C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery\attempts\20260912051035747\report.json`.
Snapshot detail: `qa-state.json` beside that report.

## Exact test scope

- Contribution branch: `codex/issue-50-factory-recovery`.
- QA PostgreSQL: 55450, dedicated existing cluster and per-attempt database
  `ecorp_qa50_20260912051035747`.
- QA API: 18450; runner: `runner-qa-issue50`.
- Only fake Codex and fake GitHub were used. No real model calls or GitHub effects.
- Synthetic Git source: `3ef4bdb76f5efd363fc2314804422e4db7ca90a1`, identifying
  `all-the-vibes/ecorp`; not a published application commit.
- The host held an unchanged synthetic `README.md` read-only with Windows
  `FileShare.None`, then released it before resume. Exact physical file bytes
  before/after were equal. Git verified that its tracked content was unchanged.
- The native checkpoint reader, not a manually edited database row, reported the
  resulting failure: `open workspace fingerprint path ... README.md ... (os error 32)`.

The injection models transient read failure, not the exact historical timeout
that occurred in #235. It does not explain the cause of that timeout or prove
that arbitrary unverified workspaces may safely be resumed.

## Persisted sequence

| Identity | Value |
| --- | --- |
| Mission | `e020bc2c-7fb9-4916-b7c3-83657fc4ff0b` |
| Task | `a77ae1d6-3daa-4670-98c1-70f54cf77e87` |
| Original run/workspace | `c0d134ed-f4ca-417b-8e89-6d3ce6313522` |
| Resumed run | `5db30f2f-3500-46c1-a681-632d362945e6` |
| Reused provider session | `4155da3e-a9be-4c53-a541-2f60e3a3e6a9` |
| Factory item | `0cc0ba1c-e8c7-41e8-bbc1-99752595ef5d` |

1. Native Factory dry-run passed before claim and dispatch.
2. At 6,000 synthetic tokens the run/task/mission cancelled at `suspend`.
   The workspace was preserved, with null fingerprint and an actual native
   checkpoint-read error. No source proof was fabricated.
3. Resume without remaining authorized mission budget returned HTTP 409.
4. Controller reconciliation returned HTTP 400 from its checkpoint context read:
   `source preservation has no stopped-source checkpoint proof`. The Factory item
   stayed `running`; that projection did not mean a provider was still active.
5. A synthetic owner approved a 20,000-token mission ceiling with a narrower
   4,000-token finish allocation. Proposal and decision replays reused the same
   revision. Member proposals/decisions returned 403, and a stale version returned
   400 while leaving the pending proposal unchanged.
6. Explicit native resume returned 200, reusing the session, source and workspace
   lineage. A duplicate resume returned 409, without another run.
7. The resumed provider consumed 10 input and 2 output tokens. Artifact and
   `resumed.txt` verification passed. Requester self-review returned 403; the
   distinct synthetic reviewer approved the result.
8. Run/task/mission completed and Factory reached `verified`, version 17.
   Original usage remained 6,000, original breaker remained `suspend`, and the
   original fingerprint remained null. A new fingerprint belonged only to the
   successfully verified resumed run. Total reported usage was 6,012 tokens.

## Earlier attempts retained

- `20260912050630179`: locking changed `base.txt` intercepted Codex's adapter
  evidence collection before checkpoint capture. The provider process stopped,
  but adapter evidence reading failed with OS error 32. The run/mission failed at
  `suspend`, and Factory changed from `blocked` to `failed` during reconciliation.
  The test stopped at its diagnostic assertion before any budget revision or
  approved resume. This is a distinct execution/evidence-failure observation;
  its recoverability was not tested or claimed.
- `20260912050936713`: a cross-checkout byte assertion failed because Git produced
  CRLF in the worktree from the LF source. The harness now verifies tracked content
  with Git and compares exact physical bytes within the same worktree before/after
  the lock. This was a fixture assertion failure, not application acceptance.
- `20260912051035747`: the unchanged-README checkpoint-read injection and complete
  native budget-revision/resume/review sequence passed.

All attempts stopped only their verified QA services. The lock helper exited and
released its handle; data, source worktrees, process receipts and logs were retained.
The final QA port set was empty. The retained system's before/after state matched.

## Validation and reproducibility

```powershell
$env:ECORP_ISSUE50_QA_ROOT = 'C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery'
$env:ECORP_ISSUE50_PG_BIN = 'C:\Users\aabdelsalam\.ecorp\pg\pgsql\bin'
node tools/e2e_factory_budget_recovery.mjs --dry-run --missing-checkpoint
# Run only after approval of the displayed QA scope.
node tools/e2e_factory_budget_recovery.mjs --execute --missing-checkpoint
node --test tools/fake_codex_budget_stream.test.mjs
node --check tools/e2e_factory_budget_recovery.mjs
git diff --check
```

- Final provider-protocol regression: 11 passed, 0 failed.
- JavaScript and PowerShell helper syntax and whitespace checks passed.
- The prior turn's 17 native checkpoint tests passed; they were not rerun here.
- This is native CLI/API/server/runner evidence, not browser acceptance or a real
  provider conformance test. No full repository gate, commit, PR or hosted CI is claimed.
- No production Rust/runtime logic or authorization rule was changed.

Test harness SHA-256:
`67E5C65BCD2C21DEACCE00225F65D46BFBFD0888461BA4CB47AAA12A3EEAAC3E`.
Read-lock helper SHA-256:
`4B3AA487FAA511697605BBB7F403D774360147EC4568B01F25C630F12455543B`.
Synthetic provider SHA-256:
`BC750332A4DC6BC633CB4291945FB311AD419A4B9D03539F27A9A626460D7381`.

## Retained #235 and the next budget decision

Read-only live inspection confirmed:

- Mission ceiling: 200,000 tokens; original ceiling: 200,000.
- Task contract ceiling: 200,000 tokens; version 1; one attempt used of two.
- Consumed usage: 208,704 input + 960 output = 209,664.
- No budget revisions, no resume, and no autonomous controller.
- Existing write scope remains `scenarios/maf-contributor-team/**`.
- Source stays at `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.

Raising the mission ceiling alone would not raise the resumed run's task ceiling.
`budget_revision.rs` permits a finish scope to reduce, not increase, task budget;
native resume takes the minimum of task, mission-remaining and rolling authority.
Resume contract revision also forbids changing budget authority. The budget-revision
implementation is byte-identical in the running and tested checkouts (SHA-256
`AC1B4CCA4CA84A5D6470DF93FF1A75E0C5A0C6D4E768FFF7B559CB4C01F6930A`).

Therefore the agreed #236 target (larger initial allowance plus audited extensions)
is not already active and cannot be obtained merely by raising #235's mission
total. A live trial at the existing task ceiling and a new governed task-budget
extension are different decisions. Do not silently increase database fields,
reset usage, duplicate the mission, or claim the auditor exists. Keep #50 open for
remaining acceptance/review and track the separate adapter-evidence failure with
its actual observed scope before proposing a fix.
