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

- accepted run `bc8a174a-a289-4ab3-ba3f-35a6b4272597`
- accepted artifact `9d32a172-fbff-4369-a065-6f9faead8f0e`
- digest `b496a76ce1f0dd4c0b0471514ffa472d3e6fbeea5ffddf357df0bc60aa859f6f`
- duplicate digest event `9603692b-bf80-44af-8f17-47ea77029c9a`
- competing run `50ed1273-e00d-4671-8e95-81907a4580d5`
- competing artifact event `41e52188-ad2a-45e9-a7c4-c676e7539ef3`

Two distinct events with the same run and digest collapsed to one metadata row and one immutable
artifact event. The test then held a competing run row before sending the same digest, advanced the
breaker to `stop`, and released the row. Authoritative preparation rejected the upload before
object bytes were written, no second metadata row was accepted, and the original shared-digest
artifact remained downloadable.

### Database reservation failure before staging

- run `33433f89-02f1-4300-b00c-d66a9cdcca67`
- artifact event `99518c7f-8f88-4767-9ab4-928036188409`

A temporary Postgres trigger raised during the artifact metadata insert. A non-transactional
sequence proved the reservation path was reached. The run failed, no staging object was written, no
artifact metadata row existed, and no final content-addressed object was published. The trigger,
function, and sequence were removed in a `finally` cleanup path.

### Database failure after object publication

- run `9914980b-8839-4dfd-a55f-f13b18538b93`
- artifact event `5d05a7dd-d38a-4ed4-b7bd-4329345313be`

A second trigger raised when the staged metadata attempted to transition to `ready`, after both the
staging and content-addressed final bytes existed. The transaction rolled back without an artifact
event. After the trigger was removed, restart recovery reused the staged reservation, preserved the
final object, linked it to the failed run without rewriting the run's terminal status, emitted one
idempotent artifact event, marked the metadata `ready`, and removed the staging copy.

### Restart recovery and cleanup retries

- recoverable staged artifact `455ee3cd-7a13-45c1-b168-fb72226d7aa7`
- missing-object artifact `89a7b063-a177-4ee3-826b-760eb5f0f83f`
- ready artifact with deferred cleanup `7eaaf10f-7fe1-4933-b0b6-d44f5506147d`
- unreserved staging key
  `staging/corps/00000000-0000-4000-8000-000000000001/da74d3dc-45ee-4e9d-a118-bbfc478f986a`

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

## Existing artifact contract

`node tools/e2e_artifacts.mjs` also passed after the change. Authorized download, non-member denial,
path-free shared state, media validation, HMAC provenance, and verifier evidence remain intact.

## Boundaries

- This validation used the local object-store backend for deterministic filesystem assertions.
- The existing MinIO evidence still covers the S3-compatible backend contract.
- Long-term retention sweeping and signing-key rotation remain separate work.
