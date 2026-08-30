# Tauri desktop validation — August 30, 2026

- `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` passed on Windows.
- The shell loads `apps/web/dist` and contains no server, runner, or provider process launcher.
- `crony://` URLs are registered through the Tauri deep-link plugin and forwarded to the shared UI.
- The shared UI includes focus targets for rooms, missions, tasks, and runs.
- CI runs `cargo tauri build --debug --no-bundle` on Windows.

Execution remains in the runner daemon, so closing the desktop changes no authoritative run state.
