# ECorp architecture artwork

[Open the current full-resolution architecture illustration](ecorp-architecture-retro-v2.jpg).

## Origin and branding

Created in **Google Gemini**, image mode with **Pro Extended**, on September 6, 2026
(America/Chicago). Gemini used the existing approved
[software-factory hero](../branding/source/ecorp-software-factory-hero-v2.jpg) and a PNG export of
the [outlined ECorp logo](../branding/svg/ecorp-logo.svg) as references.

The final JPEG is **2752 × 1536**, downloaded at native resolution without cropping, recompression,
retouching, or upscaling. It preserves ECorp's ivory, navy, coral, and teal retro-futuristic sprite
theme. The existing hero and logo were not modified.

The corrected diagram names the central component **ECorp Control Server**. **Rust + Axum**
appears only as a smaller implementation annotation, not as the component's architectural identity.
The [previous image](ecorp-architecture-retro.jpg) is retained for reference.

SHA-256:

```text
1bb05beb712257f7c1739986d2bc495f3ee0c260bd1aaf6b17ed633c82a36fb3
```

## Architecture scope

Grounded in [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) at
`e76adf03185d2dd93b2761ecb5cf4ceee41a79e5`. The illustration shows three logical planes,
runner-side verification, private artifact storage, and human-authorized pull-request publication.
Storage is intentionally vendor-neutral; the picture does not assert a particular cloud deployment.

This is a raster illustration, not an editable vector diagram or an exhaustive security model.
Runtime availability varies by runner and platform. Git worktrees are not complete OS sandboxes,
and publication does not authorize automatic merge or deployment.
