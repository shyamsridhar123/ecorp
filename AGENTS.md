# Crony Corp contributor guide

## Product contract

Before a non-trivial change, read:

1. `docs/PRODUCT_AND_TECHNICAL_PLAN.md`
2. `docs/ARCHITECTURE.md`
3. `docs/SECURITY.md`
4. `docs/EVALS.md`

## Invariants

- The office is a projection of authoritative state.
- The runner survives every UI client.
- The server never executes untrusted agent shell commands.
- Messages are not tasks; tasks have explicit state machines.
- Completion requires evidence.
- Irreversible effects require authorization and idempotency.
- Tenant and Corp scope must be explicit on every stored object.
- Never put long-lived secrets in prompts, logs, command arguments, or agent-readable files.

## Validation

Run before committing:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

For user-visible behavior, start the complete local stack and exercise the browser-to-server-to-runner
path. Unit tests alone do not prove the product works.

