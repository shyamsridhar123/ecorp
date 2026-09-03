# Windows verifier command-resolution validation

**Issue:** #97

**Source base:** `e48e807968926a801d9290576198f2605978e13a`

## Failure reproduced

The enrolled Windows toolchain places an extensionless Unix `npm` script and `npm.cmd` in the same
`PATH` directory. The previous verifier delegated bare-name lookup to `Command::new("npm")`, which
did not apply `PATHEXT`. An exact-first prototype made the failure more explicit by selecting the
non-Windows `npm` file and failing to spawn it. The authoritative verifier therefore could not
reproduce a provider-shell command that had already passed.

## Resolution contract

The runner now resolves commands before spawning:

- bare names use only absolute entries in the runner process's existing `PATH`;
- Linux and macOS require an exact regular file with an executable bit;
- Windows names with an extension require an exact regular file, and extensionless names use
  validated alphanumeric entries from the existing `PATHEXT` in order;
- relative `PATH` entries and implicit worktree/current-directory searches are excluded;
- explicit absolute or workspace-relative paths are canonicalized; traversal, dot, rooted,
  drive-relative, directory, non-regular, and canonical containment escapes fail closed;
- the canonical executable and arguments remain separate inputs to Rust's process API; no shell
  path or joined command string is accepted from the policy;
- timeout, kill-on-drop, null stdin, and 16 KiB stdout/stderr bounds are retained.

Persisted command evidence includes `requested_program`, `resolution_mode`, and
`resolved_executable`. The resolved identity contains only a filename bounded to 128 characters and
a SHA-256 digest of its canonical path. The canonical path and full mutable `PATH` are not emitted.
Failures retain the requested program and attempted mode, use a `null` identity if unresolved, and
include a precise bounded diagnostic.

## Deterministic coverage

Focused runner tests cover:

- Windows `npm`, `pnpm`, `npx`, and another executable through synthetic `PATH`/`PATHEXT`;
- declared `npm.cmd` and an explicit workspace-relative `.cmd` path;
- an injection-shaped batch argument that must remain one argument and cannot create its marker;
- missing tools, relative `PATH` exclusion, bounded evidence, explicit canonicalization, directory
  rejection, workspace traversal rejection, canonical containment, and ambiguous drive-relative
  rejection;
- unchanged exact executable lookup on Linux/macOS; and
- the installed Windows npm shim executing
  `npm --prefix scenarios/incident-command test` through the candidate verifier.

`tools/e2e_windows_verifier_resolution.mjs` is the Windows integration entry point for the installed
npm regression.

## Verification

Validation ran on Windows on September 3, 2026 from the preserved issue #97 worktree through a
temporary short `S:` mapping. The mapping was removed after each command, and generated Cargo
targets were deleted before checkpointing.

```text
cargo fmt -p crony-runner -- --check
passed
```

```text
cargo test -p crony-runner verifier::tests -- --nocapture
7 passed, 0 failed, 40 filtered out
```

The focused suite includes the installed-toolchain policy:

```text
program: npm
args: --prefix scenarios/incident-command test
resolution_mode: path_pathext
resolved executable: npm.cmd
result: passed
```

```text
node tools/e2e_windows_verifier_resolution.mjs
status: passed
```

```text
cargo clippy -p crony-runner --all-targets -- -D warnings
passed
```

```text
git diff --check
passed
```

Operator review additionally tightened workspace-relative executable containment, rejection of
traversal and ambiguous components, Unix executable-bit PATH semantics, and the total
128-character executable-name evidence bound.

The producing ECorp lineage reached a token-budget `stop` after the implementation and focused
Windows regression completed but before accepted completion. This evidence is an operator-verified
checkpoint, not an ECorp verifier acceptance claim. A fresh bounded recovery mission must restore
this exact checkpoint, rerun the persisted verifier, obtain independent review, and publish one
reviewable PR.
