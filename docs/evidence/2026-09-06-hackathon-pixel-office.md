# Hackathon office-floor revision — September 6, 2026

## Scope

Issue #141 is the existing GitHub Project work item. The user's immediate
priority is the hackathon UI, not additional stability work.

This revision is isolated on `codex/hackathon-pixel-office`, based on
`99440e9fbb3a88129e4a62a4ad1b16fb9c2b8838`. The manual runtime checkout, main
checkout's unrelated architecture edits, and unfinished recovery integration
were not modified. Open PR #142 was inspected; its broader operations redesign
was not silently overwritten or cherry-picked.

## Visual result

The initially authored flat furniture renderer was rejected and removed.
The final background was generated in Chrome using **the same Gemini art
conversation as the ECorp product video**. It carries that video's warm ivory,
coral, turquoise and navy space-age identity into a dense, furnished office.

The packaged scene has six empty workstations, a dusk skyline, a glass review
room and a coffee lounge. It contains no people. A second Gemini edit removed
the first candidate's unintended measurement annotations.

The static background and live agent layer are separate:

- The downloaded JPEG is unchanged; provenance and both prompts are retained
  in `apps/web/public/assets/office/GEMINI-PROVENANCE.md`.
- Six Pixel Agents/MetroCity character sheets retain their CC0 and MIT notices,
  source commit, exact byte hashes and frame metadata.
- Agent identity and current status come from ECorp's snapshot.
- A finite arrival is tied to an observed new run; typing and reading use
  actual sprite-sheet frames. Idle/blocked/offline agents do not wander.
- Pending approval means an exact matching current-run request, not merely
  any `blocked` status.
- Six-seat studios page larger rosters rather than stacking excess agents at
  modulo coordinates.
- Light, readable controls, a non-spatial crew list, fit/zoom, motion pause and
  system reduced motion remain available.
- The inspector uses the same character art, a content-sized dialog, a bounded
  scroll area, keyboard focus trapping, Escape and focus restoration.
- Hidden workspaces unmount the inspector so browser navigation cannot leave
  the body scroll-locked behind a hidden dialog.

No execution adapter, permission policy, task lifecycle or approval endpoint
was changed.

## Local gates

Run locally; no GitHub Actions dependency:

| Gate | Observed result |
| --- | --- |
| `node tools/check_migrations.mjs` | 34 immutable migrations |
| `cargo fmt --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --workspace -- --test-threads=1` | 151 passed, 0 failed |
| `pnpm build:web` | Pass |
| `pnpm lint:web` | Pass |
| `node --test tools/office_model.test.mjs` | 54 passed, 0 failed |
| `git diff --check` | Pass |

The model tests cover all eight visual states, unrelated/stale approval
requests, input-order stability, empty and partial studios through 19 agents,
invalid seat indexes, and all six finite routes against desk footprints.
The character packet separately validates six sheets, 126 nonempty source
frames, 176 frame lookups and 14 stable identity cases.

## Browser observations

The final background and all character sheets loaded successfully.

- Observed desktop viewports: **1280 × 720** and **1440 × 900**.
- Observed narrow viewport: **390 × 844**.
- No horizontal document overflow at either size.
- Six independently selectable agents and six readable crew entries.
- Fit resets zoom; 125% zoom scrolls inside the floor rather than the page.
- Idle agents report `pose=idle`, `frame=1`, `moving=false`.
- Visual pause and system reduced-motion controls are distinct from stopping
  a runner. The temporary media override was cleared.
- Inspector Shift-Tab wraps to its last control, Tab wraps to Close, Escape
  dismisses it, and focus returns to the initiating agent button.

Calculated text contrast against the declared solid surfaces: body **14.19:1**,
secondary text **5.68:1**, idle **4.90:1**, working **6.80:1**, review **6.76:1**,
blocked **6.54:1**, approval **5.87:1**, primary button **6.51:1**.
These are focused checks, not a claim of a complete WCAG audit.

## Execution-path evidence

A separate database, server, runner and frontend were prepared for acceptance.
The protected manual server/UI/game on ports 8991, 5291 and 15195 were left
running unchanged.

The first mission was dispatched through the browser. The floor received its
real `starting` state and returned to the server-reported `idle` state after
the base runner failed to prepare its isolated worktree. Both failed attempts
remain preserved; their summarized error does not establish a root cause.
This is **not** a passing provider-execution claim.

The acceptance launcher was then repaired without changing product code.
Its process-ownership guard now compares normalized timestamps, and Git
environment variables are removed rather than set to empty strings. A
read-only probe confirmed that this PowerShell environment preserved an empty
`GIT_DIR` when passed a null string, and Git rejected that environment.
A fresh README-only local Git fixture kept all test source outside ECorp.

The subsequent **browser → server → runner → isolated worktree → verifier →
artifact download** path passed:

- Mission: `9d9b877e-f094-4cd8-8964-bd890388f0ee`
- Exactly one run: `5fcd757a-4e54-4996-9621-c46b1f241a39`
- The browser observed Working (`type`, frames **3 and 4**), Awaiting approval
  (`idle`, frame **1**, no movement), Working after the fixture-only decision,
  and final Idle. Other agents remained at idle frame **1**.
- The real persisted action request was approved through the browser. Its
  “publish release” wording belongs to the existing deterministic fixture;
  the script resumes local work and does not publish anything externally.
- Mission and run reached `completed`; verification reached `passed`.
- The downloaded artifact contained **1,404 bytes**, with SHA-256
  `50e858203e0fc72344b4cbbdf1c8ef2525e835c0599fee944d83895291941d38`,
  matching the server record.
- The worktree was preserved; the agent returned to `idle` with
  `current_run_id = null`.
- Both earlier failed runs remained unchanged.

The test stack uses only the explicitly named deterministic `fake-process`
adapter. Actual Copilot/Claude/Codex/OpenCode discovery is disabled in that
separate fixture environment. No fake fixture result is presented as real AI
provider evidence.

## Delivery boundary

This is a UI/artwork change. Stability work remains separate. Publication,
merge and auto-merge are not implied by a local preview or passing local gate.
