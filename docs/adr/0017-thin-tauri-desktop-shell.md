# ADR 0017: Thin Tauri desktop shell

## Status

Accepted — August 30, 2026

## Decision

The desktop application is a Tauri 2 shell over the exact React frontend and server APIs used by
the browser. It owns window lifecycle, native packaging, and `crony://` deep links only.

It does not embed the server, runner, database, provider CLI, or process supervisor. Closing the
desktop therefore cannot terminate an active run.

## Consequences

- Desktop and browser behavior share one implementation.
- Server and runner availability remain explicit prerequisites.
- Installer signing is a release concern; CI performs a no-bundle Windows packaging smoke test.
