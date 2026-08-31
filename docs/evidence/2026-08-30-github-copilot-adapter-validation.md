# GitHub Copilot adapter validation

Date: August 30, 2026

## Deterministic lane

`tools/e2e_copilot.mjs` ran the CI-only Copilot fixture through the live
server-to-runner path. It verified:

- three fixture models, including a disabled policy state;
- rejection of an unavailable model;
- explicit `high` reasoning selection;
- persisted model and reasoning metadata;
- normalized usage and SHA-256 evidence; and
- resume into the same provider session.

## Authenticated lane

`tools/probe_copilot_live.mjs` used `github-copilot-sdk` 1.0.11 and a real
GitHub-authenticated Copilot runtime. The account returned 25 models at probe
time. ECorp selected `gpt-5-mini`, created the requested proof file in an
isolated Git worktree, verified the evidence, and completed the run.

Recorded result:

- run: `c5e758a5-abd8-4673-86cc-d15165918f59`
- provider session: `98ae4d6d-a463-4e02-b09e-d9c3ae453ab7`
- artifact URI: `/api/corps/00000000-0000-4000-8000-000000000001/artifacts/177cb755-6d4d-43e9-b2cc-d281015db448`
- input tokens: 45,006
- output tokens: 1,802
- durable approvals: 0
- proof: `copilot-live-proof.txt`

The live catalog is account- and policy-dependent and can change independently
of ECorp. The machine-readable report is written to
`output/e2e-copilot-live.json`; CI retains only the deterministic report.

## Quality gates

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- TypeScript project build
- Vite production build
- Oxlint
