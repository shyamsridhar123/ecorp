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
time. Crony selected `gpt-5-mini`, created the requested proof file in an
isolated Git worktree, verified the evidence, and completed the run.

Recorded result:

- run: `6e8122cf-c632-44c4-85b7-810c3e6845aa`
- provider session: `5418ecd4-d23d-4c36-bdd6-756539277f32`
- input tokens: 42,866
- output tokens: 1,048
- durable approvals: 0
- proof: `copilot-live-proof.txt`

The live catalog is account- and policy-dependent and can change independently
of Crony. The machine-readable report is written to
`output/e2e-copilot-live.json`; CI retains only the deterministic report.

## Quality gates

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- TypeScript project build
- Vite production build
- Oxlint
