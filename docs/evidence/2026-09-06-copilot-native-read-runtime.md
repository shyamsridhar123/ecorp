# Copilot native reads and governed runtime selection

**Date:** September 6, 2026

**Work:** #153, #144, #145; PR #150

**Hosted Actions:** Not used.

## Failure reproduced without weakening permissions

The dedicated native-read probe uses an immutable disposable Git source, a small non-secret
marker file, the real Copilot SDK, and the existing ECorp server/runner. It requires:

1. A successful native `view` of the source by relative path.
2. A successful native `view` of the same source by absolute worktree path.
3. Native creation and successful `view` of a contained readback file.
4. A persisted artifact/file/exact-hash verifier, unchanged source bytes and mtime, zero routine
   approvals, and zero model-session shell executions.

Actual correlated tool responses are checked. A provider's claim, an existing output file, or
missing event data cannot pass the read gate. The first relevant failed view or unexpected
approval stops only that diagnostic run before it can waste the remaining provider budget.

SDK `1.0.11` with the auto-selected CLI `1.0.83` reported `Path does not exist` for the committed
seed. The scoped filesystem callback returned successful metadata for that exact path.
Metadata-only wire observation confirmed a correctly correlated response, `isFile=true`,
`isDirectory=false`, valid timestamps, numeric size, and no RPC/filesystem error. The native
view still failed before reading contents.

A private, task-owned fixture outside the user's home reproduced the same failure without
changing sandbox policy. Removing home-directory protections was therefore not adopted.
SDK `1.0.13` with CLI `1.0.83` also failed the same native-read case; that dependency experiment
was reverted. No permission or filesystem containment check was relaxed.

## Runtime drift identified

The SDK's cached `v1.0.79` runtime archive matched GitHub's published SHA-256:

```text
ae87705442b502853374a58938ca48309b44ad1aef201e3de56b9ff89fe3b6bd
```

Running its CLI without the no-update control reported `1.0.83`; running with
`--no-auto-update` reported the packaged `1.0.79`. The CLI's own help documents automatic
updates as enabled by default outside CI. A nominal versioned SDK cache alone was therefore
not sufficient to hold the tested runtime constant.

With SDK `1.0.11`, CLI `1.0.79`, and automatic updates disabled, all three native views succeeded.
The first LF-seeded round-trip still failed its strict byte hash because the native Windows
patch tool wrote CRLF. That failure is retained; neither the file nor its expected hash was
rewritten into success.

A separate immutable Windows-native CRLF fixture was then used unchanged for the before/after
comparison. Every case retained the same source commit and exact expected hash:

```text
Source commit: f9f8ffb42f773bbfe743b215f309fe83ac750d67
Source SHA-256: 7a892d437324c3c4bd935708a9efa7ce0cc6b799443c1971ef16d42ff0436a1b
```

| SDK / CLI | Run | Native reads and exact round-trip |
|---|---|---|
| 1.0.11 / 1.0.83 | `41c7ee0f-244a-40a9-9911-945c3434f9d9` | Failed on the first native view |
| 1.0.13 / 1.0.83 | `b747a11e-f7a7-4a2f-96a1-3f55e95a614f` | Failed on the first native view |
| 1.0.11 / 1.0.79, product no-update flag | `7b1ac95d-69c3-4968-a530-fc42863f6206` | All three views and all persisted checks passed |

The passing source and output hashes were identical. The real provider was `gpt-5.6-sol`,
with 62,093 input and 484 output tokens, one attempt, no repair, no routine approvals, and no
model-session shell. The canary observer saw the product's `--no-auto-update` argument on
both catalog and provider processes; the caller did not supply `COPILOT_AUTO_UPDATE`.

The rendered ECorp UI showed the completed mission, one completed task/attempt, and all three
persisted checks: verified provider artifact, the 81-byte readback file, and successful exact-hash
Node verification. No browser console errors were recorded.

## Product change

Managed local clients now include `--no-auto-update` alongside the existing experimental,
sandbox, and no-temp-directory controls. Discovery, create, and resume check the connected
runtime version. The current supported pair remains the checked-in SDK `1.0.11` / CLI `1.0.79`.
Unsupported explicit binaries or remote runtimes are not advertised as working native-filesystem
providers. The check is compatibility control, not security attestation.

The post-change `1.0.83` negative control was rejected before creating any mission or run:
the snapshot remained at 28 missions / 31 runs, and the probe-owned runner disconnected.

After adding that runtime-version fence, the final real invocation also passed:

| Evidence | Value |
|---|---|
| Run | `8144fd85-6839-477d-b7e3-282be0762762` |
| Provider session | `389486f4-cb3f-4ea6-8259-d0b1a2f1ea77` |
| Source and output SHA-256 | `7a892d437324c3c4bd935708a9efa7ce0cc6b799443c1971ef16d42ff0436a1b` |
| Native views | Relative source, absolute source and readback: all successful |
| Persisted verification | Passed |
| Routine durable approvals | Zero |
| Model-session shell executions | Zero |
| Product no-update argument | Observed on catalog and provider processes |
| Caller auto-update environment override | Absent |
| Usage | 62,160 input and 466 output tokens |

Both successful runs used the same immutable CRLF fixture. The final version-fenced case did
not reuse an accepted report, reset a budget, retry a stopped lineage, or weaken the hash check.

The Windows path regression separately proved that ordinary and extended namespace spellings
could reject the same authorized file. Prefix matching now compares the drive or UNC
server/share identity while retaining every subsequent component check and capability-based I/O.
Other drives, shares, sibling prefixes, device namespaces, traversal, Git metadata, alternate
streams, symlinks and hard links remain denied.

## Diagnostic safety

Native filesystem debug output records operation, path form, hashed path and success metadata,
not paths or file contents. The optional test-only wire observer copies original buffers
unchanged, bounds headers/frames/pending requests, and records only whitelisted stat/exists
response shape. It does not log file bytes, paths, credentials or unrelated RPC payloads.
Tests cover fragmented frames, payload non-disclosure, identifier-type mismatches, oversized
frames, and exact subprocess byte forwarding.

Artifacts, versioned diagnostic runtimes, fixtures, logs and screenshots are outside the ECorp
repository under:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue153-native-read-20260906
```

The existing demo/game was not reset, stopped application lineages were not restarted, and no
probe source was pushed into the product repository. The prior main-checkout edits were
preserved. Merge, auto-merge and deployment remain separate and unrequested.

## Local validation

- Migration validation: 34 immutable migrations.
- `cargo fmt --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace` with `RUST_TEST_THREADS=1`: **151 passed**.
- Web build and lint passed.
- **25 Node tests passed**, including the process observer, boundary completeness, exact-session
  native read results, and transparent/redacted wire observation.
- **Seven Linux filesystem tests passed** in the local Rust `1.94` container. Its read-only
  `/src` mount was verified to be this current worktree, and source hashes were recorded before
  execution. No stale source copy was counted.
- Runner build and diff checks passed.

Current Windows filesystem coverage includes ordinary/extended namespace equivalence and rejects
other drives, shares, sibling prefixes and device aliases. Existing dangling-link, hard-link,
scope, root-protection and mode checks remain enabled.

## Scope still open

This restores native reads for the verified runtime pair. It does not claim to fix CLI `1.0.83`,
preserve arbitrary text-tool line endings, complete the entire live negative permission matrix,
implement #148 verifier-only checkpoint recovery, or finish the broader #144/#145/enterprise
factory acceptance.
