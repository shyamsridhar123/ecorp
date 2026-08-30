# Artifact storage validation

Date: August 30, 2026

## What was verified

The runner uploaded artifact bytes through its authenticated server channel.
The server:

- rejected local-path persistence and stored only durable artifact metadata;
- verified the declared byte count, SHA-256, and `text/markdown` content;
- wrote a Corp-namespaced content-addressed object;
- recorded producer agent, producer runner, run, task, verifier, and retention;
- generated and revalidated an HMAC-SHA256 provenance signature; and
- served the object only through a Corp- and room-authorized download route.

Alice downloaded the artifact with matching media type, digest, and signature.
Eve, who is not a member of the room, received HTTP 404. Neither the public
artifact event nor the shared run projection contained artifact bytes, an
object-store key, or a runner-local path.

## S3-compatible proof

The live end-to-end flow ran against MinIO through the `object_store` S3
backend:

- run: `4f5480c3-54eb-47d7-bb51-2293c214c9fa`
- artifact: `4e99b8a4-9f0c-4ad0-bfd9-eec102065417`
- digest: `d01ed31037976b3d35c6bebc5993ab69edac6bbc618f7d59eaf55c54e5b5cea5`
- bytes: 1,168
- verifier: `crony-server:artifact-ingest-v1`
- unauthorized download: HTTP 404

MinIO listed the object at:

`corps/00000000-0000-4000-8000-000000000001/sha256/d0/d01ed31037976b3d35c6bebc5993ab69edac6bbc618f7d59eaf55c54e5b5cea5`

The deterministic local-object-store lane is recorded in
`output/e2e-artifacts.json` and runs in CI.
