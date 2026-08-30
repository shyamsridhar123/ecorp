# Public name, art, and licensing review

**Review date:** August 30, 2026  
**Scope:** preliminary product clearance, not a legal opinion

## Decision

**Do not launch publicly as “Crony” or “Crony Corp.”** Keep it as the repository and internal
codename until counsel clears a replacement. The preferred public candidate from this pass is
**Guildframe**.

Why:

- “Crony” is already used by multiple active software projects, especially cron/task schedulers.
- The exact `crony` package name is occupied on npm, crates.io, and PyPI.
- `cronycorp.com` is unavailable.
- “Crony” has a negative dictionary meaning that works against a trusted-workplace product.

“Guildframe” better communicates a persistent structure in which people and agents work together.
It is not cleared by counsel: a GitHub project named `guildframe-site` exists, so a professional
word-mark and class search is still required before launch.

## Search record

### Product and source-code search

GitHub search for repositories with `crony` in the name returned numerous active software
projects, including distributed schedulers, cron managers, monitoring utilities, and a Windows GUI
utility. This is a direct software-category collision signal.

GitHub search for `guildframe` returned:

- `Awmair/guildframe-site`, a site for tabletop creators;
- `petterm/HH-GuildFrame`, a World of Warcraft addon.

These are less adjacent than the Crony software results, but they still require counsel review.

### Package registries

| Registry | `crony` | `guildframe` |
|---|---|---|
| npm | occupied (`crony` 0.5.0) | registry lookup was inconclusive because the configured feed failed TLS |
| crates.io | occupied (`crony` 0.3.1) | no result in the recorded search |
| PyPI | occupied (`crony` 0.2.1) | no matching distribution |

Recommended package namespaces:

- npm: `@guildframe/core`, `@guildframe/mcp`
- Rust: `guildframe-*`
- Python: `guildframe-*`

Do not reserve or publish packages until the public name is approved.

### Domain options

Availability is a point-in-time result and can change.

| Domain | Result on August 30, 2026 |
|---|---|
| `cronycorp.com` | unavailable |
| `cronycorp.ai` | available, quoted at USD 160 for two years |
| `crony-corp.com` | available, quoted at USD 11.25 for one year |
| `cronyhq.com` | available, quoted at USD 11.25 for one year |
| `runcrony.com` | available, quoted at USD 11.25 for one year |
| `cronylabs.com` | unavailable |
| `guildframe.com` | unavailable |
| `guildframe.ai` | available, quoted at USD 160 for two years |

Recommendation: if the candidate survives legal review, reserve `guildframe.ai` plus a defensive
package namespace. Do not spend money based only on this engineering review.

### App stores and social handles

General App Store and Play Store searches did not provide a reliable clearance result from the
available automated interfaces. Before public launch, counsel or the product owner must repeat
exact and phonetic searches in:

- USPTO Trademark Search and WIPO Global Brand Database;
- Apple App Store and Google Play;
- X, Bluesky, LinkedIn, YouTube, Discord, and major package registries.

Suggested handles to test and reserve: `guildframe`, `guildframehq`, and `runguildframe`.

## Approved original art direction

The approved direction is **operational editorial office**, not a parody of *The Office*:

- warm paper, ink, marigold, cobalt, mint, and signal-red palette;
- top-down workplace geometry tied to real operational state;
- document stamps, ledger lines, routing marks, and restrained industrial iconography;
- abstract original worker/agent forms rather than actor likenesses or television characters;
- no Dunder Mifflin marks, paper-company trade dress, Buzz identity, or copied source artwork.

This direction preserves the legibility and charm of a shared office while making every visual
element communicate state.

## License and provenance audit

- Repository license: Apache-2.0.
- `NOTICE` states that the implementation is greenfield and includes no Munder Difflin artwork,
  television-character likenesses, or Buzz branding.
- Dependency manifests and lockfiles are the authoritative third-party inventory.
- Tracked visual assets were introduced in the repository's original vertical-slice commit.
- The unused Vite and React sample assets should not be treated as product identity.
- No code or artwork from the two research repositories is vendored.

## Launch gate

Public launch remains blocked until:

1. counsel completes exact, phonetic, and related-goods searches for Guildframe;
2. the owner approves and reserves the final domain and handles;
3. package namespaces are reserved;
4. final logo files include source provenance and license metadata.
