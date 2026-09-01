# Artifact staging and recovery validation — September 1, 2026

## Scope

This record covers GitHub issue #58. It verifies that rejected evidence cannot leave unbounded
staged objects and that cleanup cannot delete content-addressed bytes referenced by accepted
evidence.

The implemented commit protocol is:

```text
authenticated runner event
  -> byte, digest, size, and media validation
  -> locked run, assignment, budget, and breaker validation
  -> staged metadata reservation
  -> Corp/event-specific staging object
  -> content-addressed final publication
  -> ready metadata + immutable artifact event
  -> staging cleanup
```

`staged`, `ready`, and `rejected` metadata plus the deterministic staging key make every step
idempotent and recoverable.

## Object-store unit evidence

`cargo test -p crony-server artifacts::tests::staged_objects_finalize_idempotently_and_cleanup_safely`
passed.

The test proves:

- a staged object is not readable through the verified final-object path;
- final publication validates provenance, digest, byte count, and media type;
- repeated finalization is safe after the staging copy has already been removed;
- repeated staging cleanup treats absence as success;
- staging-key discovery is bounded to valid Corp/event paths;
- discarding a rejected staging object with the same digest does not delete the accepted final
  object.

## Complete local-stack evidence

`node tools/e2e_artifact_staging.mjs` passed against Postgres, the real server, a controlled
authenticated runner connection, the local object-store backend, and the normal REST snapshot and
download paths.

### Accepted and rejected uploads sharing one digest

- accepted run `53d492fe-8f07-4813-9fd9-d734075d5808`
- accepted artifact `72daa8f8-4f11-4607-92c7-399cfa0f5a94`
- digest `6329816b50fb934537ddccad76eadfedbb79b2cc0782c751462e8fb8cc0d8f21`
- duplicate digest event `1d93e13b-c992-40a8-baa2-f70454887f97`
- competing run `181a71f1-7174-499d-b6a2-2192f3c4da63`
- competing artifact event `65a7de30-e687-4453-b9d4-0cd2a5e6c052`

Two distinct events with the same run and digest collapsed to one metadata row and one immutable
artifact event. The test then held a competing run row before sending the same digest, advanced the
breaker to `stop`, and released the row. Authoritative preparation rejected the upload before
object bytes were written, no second metadata row was accepted, and the original shared-digest
artifact remained downloadable.

### Database reservation failure before staging

- run `cb73e049-8b4a-4eec-a23a-56bc3c78a365`
- artifact event `b9578a05-b565-4e09-bef1-246b2ec98e58`

A temporary Postgres trigger raised during the artifact metadata insert. A non-transactional
sequence proved the reservation path was reached. The run failed, no staging object was written, no
artifact metadata row existed, and no final content-addressed object was published. The trigger,
function, and sequence were removed in a `finally` cleanup path.

### Database failure after object publication

- run `44971504-a891-496b-8462-24f4e69f9e01`
- artifact event `a22506c7-cd18-48a4-9aa0-d24d7e099376`

A second trigger raised when the staged metadata attempted to transition to `ready`, after both the
staging and content-addressed final bytes existed. The transaction rolled back without an artifact
event. After the trigger was removed, restart recovery reused the staged reservation, preserved the
final object, linked it to the failed run without rewriting the run's terminal status, emitted one
idempotent artifact event, marked the metadata `ready`, and removed the staging copy.

### Restart recovery and cleanup retries

- recoverable staged artifact `a990448b-a5e5-4f58-81b2-8d8806d3512b`
- missing-object artifact `e8945581-2845-4b6c-bb18-a14e772a914c`
- ready artifact with deferred cleanup `22b5a6a8-305f-4d93-b45e-44eb79953d5b`
- unreserved staging key
  `staging/corps/00000000-0000-4000-8000-000000000001/9785ab01-0254-4f49-92f0-d1263605d046`

Before restart, the test constructed four crash-boundary states:

1. valid staged metadata and staging bytes with no final object;
2. staged metadata with neither staged nor final bytes;
3. ready metadata whose redundant staging key still existed;
4. a staging object with no metadata reservation.

On restart:

- the valid staged artifact was published, marked `ready`, re-linked to its completed run, and
  received exactly one artifact event;
- the old missing-object reservation was deleted so the same event can be retried;
- the ready row's staging object was removed and its staging key cleared;
- the unreserved staging object was removed;
- the local runner reconnected before the test returned.

### Periodic recovery event delivery

With a browser WebSocket connected and fully replayed, the test returned a completed run's
artifact metadata to `staged` while preserving its verified final object. The one-second test
recovery interval finalized the metadata, restored the run-to-artifact link, and delivered the new
`run.artifact` event to the already-connected client. This proves periodic recovery updates live
clients without requiring an unrelated event or reconnect. The recovered artifact was
`22b5a6a8-305f-4d93-b45e-44eb79953d5b`, and the live event sequence was `12955`.

## Existing artifact contract

`node tools/e2e_artifacts.mjs` also passed after the change. Authorized download, non-member denial,
path-free shared state, media validation, HMAC provenance, and verifier evidence remain intact.

## Boundaries

- This validation used the local object-store backend for deterministic filesystem assertions.
- The existing MinIO evidence still covers the S3-compatible backend contract.
- Long-term retention sweeping and signing-key rotation remain separate work.
