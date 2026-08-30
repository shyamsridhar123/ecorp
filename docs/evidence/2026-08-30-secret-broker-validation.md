# Scoped secret broker validation — August 30, 2026

`node tools/e2e_secrets.mjs` validates the broker through the real server, Postgres store, runner,
worktree, and child-process path.

The scenario proves:

- only an owner or admin can create a secret;
- a task references the secret by ID, environment name, tool, and resource;
- actor, task, run, runner, tool, resource-prefix, and expiry checks precede delivery;
- the fake child process receives the scoped value through its environment;
- the plaintext canary is absent from snapshots, events, logs, and the produced artifact;
- an unauthorized requester cannot launch a task using the secret; and
- revocation prevents future grants.

Machine-readable evidence is written to `output/e2e-secrets.json` and uploaded by CI.
