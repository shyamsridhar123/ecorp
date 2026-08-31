# ECorp desktop shell

This Tauri 2 application packages the same React build and connects to the same ECorp server as the
browser client. It never launches or supervises agent processes.

## Deep links

```text
ecorp://corp/<corp-id>/room/<room-id>/mission/<mission-id>/task/<task-id>/run/<run-id>
```

Segments after `corp` are optional. The shell forwards a validated `ecorp` URL to the shared web
UI, which focuses the most specific visible entity. Legacy `crony://` links remain accepted during
the compatibility window.

CI performs a Windows `cargo tauri build --debug --no-bundle` smoke build. Installer signing and
publishing remain separate release operations.
