# ECorp public name, art, and compatibility review

**Review date:** August 31, 2026
**Scope:** engineering and product-risk note, not legal advice

## Decision

The product is now branded **ECorp** at the user-facing layer. The requested creative reference is
the fictional conglomerate in *Mr. Robot*: severe corporate minimalism, a black/white/red system,
controlled institutional language, and a sense that every operation is observed and attributable.

This repository does **not** copy the television production's exact E Corp logo, wordmark, screen
assets, slogans, characters, or story material. The ECorp mark in this project is original: a block
E interrupted by a white governance slash. It communicates an execution boundary rather than
reproducing the show's logo.

## Legal and launch risk

`E Corp` is strongly associated with *Mr. Robot*. The series and its official branding are owned by
their respective rights holders. Using the same name and a closely similar mark for a public product
may imply affiliation, endorsement, or licensed merchandise status.

Engineering can implement the requested internal/private rebrand, but a public commercial launch
must remain gated on counsel reviewing:

1. the ECorp name in the intended software and hosted-service classes;
2. the final wordmark and icon against entertainment and merchandise marks;
3. marketing copy for false-affiliation or passing-off risk;
4. domains, package names, application identifiers, and store listings.

## Original visual system

The approved implementation uses:

- **Control black** `#101112` for the operations network and audit surfaces;
- **ECorp red** `#E01D25` for authority, blocked work, and the signature slash;
- **Compliance white** `#F7F7F4` for documents and high-contrast work surfaces;
- **Infrastructure gray** `#C8C9C7` for the office grid and inactive systems;
- **Verified green** `#517764` only for healthy or accepted state.

Typography is condensed and institutional for the wordmark and headings, with monospaced utility
text for runtime state, event IDs, and approval records. Motion remains operational: sprites move
only while real provider processes are alive.

## Compatibility policy

The public product name is ECorp. Existing technical identifiers remain temporarily compatible:

- Rust crates and binaries retain `crony-*` names;
- environment variables retain the `CRONY_` prefix;
- HTTP headers retain `X-Crony-*` names;
- Git worktree branches retain `crony/task-...`;
- the desktop accepts both `ecorp://` and legacy `crony://` deep links.

These are migration interfaces, not visible product branding. Renaming them requires a versioned
compatibility release rather than a search-and-replace change.

## Provenance

- Repository license: Apache-2.0.
- The ECorp SVG icon and CSS wordmark were authored for this repository.
- No production stills, logos, fonts, character likenesses, or artwork from *Mr. Robot* are bundled.
- Munder Difflin and Buzz remain research references; their art and branding are not vendored.
