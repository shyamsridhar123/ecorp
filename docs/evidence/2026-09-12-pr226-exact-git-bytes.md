# PR #226: literal Git-byte repair and remaining acceptance

Observed September 12, 2026 UTC (September 11 in America/Chicago).

## Scope and live baseline

The initial worker's read-only GitHub checks against `All-The-Vibes/ecorp` confirmed:

- Issue #225 still requires the two original SHA-256 values below.
- PR #226 is open at `ec1700db3074ecaf2045314c71ef893434c4d79e`, with base
  `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
- Review `5173100817` requests changes at that exact head.
- Hosted integration and macOS runner checks are failed at that head. The
  existing PR comment attributes them to foundation work; this repair neither
  changes that stack nor claims to have rerun those checks.

The assigned worktree is
`C:\Users\shyamsridhar\.codex\worktrees\ecorp-pr226-ready-20260911`,
branch `codex/pr226-merge-ready`. That initial worker performed no commit,
push, PR mutation, service, container, browser, database, or provider operation.

### Maintainer integration follow-up

After the worker released its five paths, the parent merged current main
`b28fd4d26309794f38c0455bbf42d22aedf7cfd1` into the candidate without a
conflict. All five byte-repair files retained their exact worker hashes.
The ten behavior/Git-byte tests passed again on the integrated candidate.
The existing quality workflow now invokes both canary test files, so Git-byte
regressions cannot silently disappear from normal checks.

The candidate's entire `crates/`, `db/`, web source, Cargo manifests/lockfile,
and pnpm manifests/lockfile were compared against main and found identical.
They therefore share the source inputs of the fresh PR234 local gate:
452 Rust tests passed, 200 deliberately ignored, workspace format and
all-target Clippy passed, 40 immutable migrations, and 194 recursive frontend
tests passed. The canary behavior and Git-byte checks are additional, not
replacements for those repository checks.

This integrates the already-landed native connection, Codex hard-stop, and
macOS fixture fixes; it does not alter those implementations. The separate
external-adapter E2E correction is tracked in PR234.

The repair is a maintainer follow-up to the producing agent's commit.
Neither its later commit identity nor these additional files are attributed
to the original Factory run. The original signed export, publication record,
and failed history remain unchanged. The remaining native acceptance
requirements below are not waived by this source repair.

## Minimal repair

The Windows working copies already had the required CRLF bytes, although their
raw Git blobs were LF. Merely hashing the working copies would reproduce the
false-positive byte acceptance.

Two exact, root-anchored `.gitattributes` entries use `-text` to preserve these
files' literal CRLF bytes in Git. `text eol=crlf` would not fix the stored LF
representation. `whitespace=cr-at-eol` recognizes the intentional CR without a
repository-wide whitespace override. The existing migration rule is unchanged.

Only those two tracked files were re-added under the new attributes. Their
content, including final newlines, is otherwise unchanged. The original
`status.test.mjs` remains unchanged, with raw Git SHA-256
`02879d294e5fda29719497e05aabb8155ddc72da88e9d5c17718d80a638911f2`.
The separate `git-bytes.test.mjs` keeps Git-dependent regression checks out of
the original six portable, file-relative behavior tests.

This is a scoped maintainer repair, not a claim that the original native
three-file run produced the added attributes, regression, or this report.

## Actual staged Git bytes

Each blob was read as a Buffer using `git cat-file blob :<path>`, not text
redirection or a Windows checkout. Its SHA-256 was checked against the unchanged
issue contract. Its Git SHA-1 was also independently recomputed over
`blob <byte-count>\0` plus the raw bytes and matched the index object ID.

| File under `scenarios/factory-live-canary/` | Bytes | CRLF | Staged Git blob SHA-1 | Required and observed SHA-256 |
| --- | ---: | ---: | --- | --- |
| `status.mjs` | 1100 | 47 | `bd7b73b1e6b067603660f277506694ccac67a397` | `df8af8ce2c756d230c1d303c2121c88a8495c608c3616592504b42a3d47c520b` |
| `README.md` | 331 | 16 | `4fa83c475d82a56e7ccdbf4bc84f3b47665ca237` | `123cc52972ec592813f99069b97b79423bff1b04fa9025822327190fd646fa8e` |

Both blobs have no BOM and no bare LF. Removing only the CR in each CRLF gives
the original published blob exactly. Fresh read-only GitHub Git-blob downloads
also matched the old local HEAD and the supplied review's retained raw files.
That old head still has the rejected hashes:

- `status.mjs`: `1c02e7e18433d419039da13ccfc1789b62cc227c6c907ee0cb5e457a635c6091`
- `README.md`: `75091bd742b2714b2e164ca10dd6ecd912f8f3b516160ad8ca4a22cf6f1bbda8`

The proof boundary is the **staged candidate**, not a corrected commit or
published head. No expected digest or historical receipt was rewritten.

## Local validation

Environment: Windows, PowerShell 7.6.6 with `login:false`, Node v25.9.0,
Git 2.55.0.windows.3. Test subprocesses received only named operating-system
environment fields and noninteractive Git settings, not provider/database
credentials. The Git regression disables system/global Git configuration and
user attributes and creates no temporary files.

Commands from the assigned repository:

```powershell
node --check scenarios/factory-live-canary/status.mjs
node --check scenarios/factory-live-canary/status.test.mjs
node --check scenarios/factory-live-canary/git-bytes.test.mjs
node --test --test-reporter=tap scenarios/factory-live-canary/status.test.mjs scenarios/factory-live-canary/git-bytes.test.mjs
git diff --cached --check
git diff --check
git diff --cached --ignore-space-at-eol --exit-code -- scenarios/factory-live-canary/status.mjs scenarios/factory-live-canary/README.md scenarios/factory-live-canary/status.test.mjs
```

- Before the fix: the actual run passed all six behavior tests and failed all
  four new Git-byte/attribute tests, including both raw index digests.
- After staging the fix: **10/10 passed** from the repository and **10/10
  passed** using absolute test paths from an unrelated working directory.
- The new regression checks raw index bytes, working-copy equality, staged
  `-text` attributes, and Git clean/checkout conversion with `core.autocrlf`
  set to `false`, `true`, and `input`, without modifying the index.
- Fresh extraction of the three original files directly from the index,
  without Git checkout filters, passed the original **6/6 behavior tests**
  from the unrelated directory; extracted bytes remained unchanged.
- All three syntax checks and both whitespace checks passed. The
  ignore-end-of-line comparison found no semantic change to the three
  original scenario files.

These local tests are **not** the eight persisted native checks in #225.
This scoped change adds CI wiring that runs the same local canary regressions in
`.github/workflows/ci.yml`; those checks still do not replace the eight
persisted native checks and records. You can also run the explicit regression
command after staging or in a fresh checkout whose index matches the candidate.
No Cargo, SQLx, web build/lint, hosted rerun, or full-stack validation was run.

## Bounded historical evidence audit

The only historical filesystem root inspected for this audit was:

`C:\Users\shyamsridhar\.codex\dogfood\issue224-planned-attempt-policy-20260910\pr226-review-20260910T225936768`

It contains nine files: `byte-review.json`, `result.json`,
`github-review.json`, `review-body.md`, `stdout.log`, empty `stderr.log`, and
the three raw scenario files. The raw files match the published head.
`result.json` explicitly reports byte acceptance failed, overall acceptance not
established, and `native_factory_replay_or_review_receipts_verified: false`.
No sanitized native acceptance receipt was found in this supplied root; this
is not a claim that no such receipt exists elsewhere.

The September 10 PR comment asserts native success for work item
`a68fd3ef-a113-4a1d-bae9-d6cfdce225a3`, mission
`8097d270-3527-47b4-aff5-8012a5984f75`, run
`23bc53f3-0bcb-4a6b-b0df-453b4f379a10`, export
`915b77c5-edb2-4316-ad8f-968e5120d0ad`, and publication
`4437be95-8fc8-4a26-a915-e78ac1df8851`. These are useful lookup identifiers,
not substitutes for the missing records.

| Requirement | What remains unproven by the available evidence |
| --- | --- |
| Eight persisted checks | Exact persisted policy and eight passing result records bound to the run, verification digest, and exported SHA. |
| Independent outcome review | Native accepted decision with requester/producer exclusion and exact run/export binding. The GitHub changes-requested review is a different decision. |
| Native publication and Project transition | Source-export/publication provenance, authorization and verifier binding, one-attempt evidence, and the exact Project item moving only after PR creation. GitHub proves the old PR exists, not this whole chain. |
| Native replay | Original/replay responses with the same record/version and unchanged object/effect counts. |
| Watcher restart | Before/after watcher identity or epoch, reconnect/reconciliation records, and unchanged work-item/mission/run/publication identities. |
| Corrected-head acceptance | Verification, applicable scope, independent review, export, and publication evidence for the exact corrected commit. Old `ec1700d` evidence cannot attest the repaired bytes. |

The native pipeline was not recreated or replaced. This local byte-repair proof
therefore does **not** establish corrected-head acceptance, native publication,
native replay, watcher restart, or the required persisted native records for
PR #226.

## Sanitized local artifacts

New evidence root:

`C:\Users\shyamsridhar\.codex\dogfood\merge-drain-20260911\pr226-byte-repair-20260911T215048171`

- `github-before.json`: live issue/PR/review snapshot with pinned identities.
- `published-raw.json` and `published-raw/`: freshly downloaded old Git blobs.
- `red.json` and `red.stdout.tap`: the observed pre-fix failures.
- `staged-byte-verification.json`: actual index blob IDs/hashes and test commands.
- `green-*.json`, stdout/stderr logs, and `staged-raw/`: successful local runs.
- `final-validation.json` and `final-*.stdout.log`: final regression readback,
  including the additional checkout-conversion assertions.
- `prior-evidence-audit.json`: all nine retained files' hashes and missing-proof matrix.
- `handoff.json` and `candidate.patch`: final scoped index manifest and patch,
  with no commit or external publication.
