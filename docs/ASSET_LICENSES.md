# Asset licensing and attribution

This document distinguishes the project's own material from third-party character artwork and marks. It records provenance and the limits of the available notices; it is not a grant of rights from the character owners.

## Scope

| Material | Applicable terms and boundary |
| --- | --- |
| Original Rust/TypeScript/Python code and documentation prose | [Apache-2.0](../LICENSE), except separately identified third-party material |
| Original geometric wordmark: `docs/assets/wordmark.svg`, `wordmark-light.png`, `wordmark-dark.png` | Apache-2.0 for the original vector design and exports. The glyph geometry is drawn in the SVG; no third-party font or character image is embedded. This does **not** grant rights to the Eevee name or related marks. |
| Original installer text animation | The shell/PowerShell implementation and punctuation-based rolling-ball frames are original project code under Apache-2.0. No franchise image file or third-party font is embedded; the Poké Ball reference does not grant rights in related character designs or marks. |
| Eevee artwork displayed in the README | Third-party Pokémon artwork with an alpha-transparent background, referenced remotely from a pinned upstream revision. It is **not** covered by this project's Apache-2.0 license. An independent image-use or commercial-use permission has not been established. |
| Optional runtime Pokémon artwork | Third-party artwork downloaded only by explicit user action, or supplied by the user. Not bundled in the binary or repository; the software license does not license these images. |
| Rust dependencies and optional agent SDK/providers | Their own licenses and service terms. Native verification packages include generated dependency license texts and notices; the optional SDK/provider installations are not relicensed by Eevee. |

The [project NOTICE](../NOTICE) accompanies source. Native-package `THIRD_PARTY_NOTICES` incorporates that notice alongside the dependency notices; no artwork is added to packages.

## README artwork

| Field | Value |
| --- | --- |
| Subject | Eevee, Pokémon #133 |
| Distributor/source | [PokéAPI/sprites](https://github.com/PokeAPI/sprites) |
| Fixed revision | `0dc105d5b12416932f29b594078282ab4be045f5` |
| Source path | [`sprites/pokemon/other/official-artwork/133.png`](https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/other/official-artwork/133.png) |
| Original format/size | RGBA PNG with alpha transparency, 475 × 475 pixels, 142,266 bytes |
| Original SHA-256 | `33ea993d3478ee920062b0bf27792a045b3a2f75d941bdb8cb3912ef50b1fb44` |
| Repository use | Remote image reference in `README.md`; original bytes are not checked in or embedded in the wordmark |
| Modifications | None to the source image; README display sizing only |

The [upstream notice at that exact revision](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) begins:

> All image contents within are Copyright The Pokémon Company.
>
> This repository is distributed under CC0 1.0 Universal

Both statements matter. Do not shorten them to “the Eevee image is ours,” “Apache-2.0 artwork,” or an unqualified “CC0/public-domain Pokémon.” The included CC0 text itself excludes trademark rights and disclaims clearance of other parties' rights. Upstream hosting, a checksum, attribution, and a remote image URL do not establish the permissions required for a particular use. GitHub may proxy/cache the README image; using a remote reference is not a rights exemption.

## Optional runtime images

The opt-in downloader uses the same pinned repository revision, under `sprites/pokemon/other/official-artwork/`, for IDs `133`, `134`, `135`, `136`, `196`, `197`, `470`, `471`, and `700`. The README references the same Eevee source image remotely; runtime downloads remain a separate explicit choice. `src/artwork.rs` records the exact source hashes; a completed user download writes a `sources.json` manifest with revision, URL, attribution and SHA-256 for each image. User-supplied images retain their respective owners' terms; unmanifested local images are labelled source-unverified.

No image is fetched on startup or form selection, and no character image is packaged in `ev`. Optional downloads and source availability are not evidence of permission to redistribute the downloaded images.

## Names, endorsement, and distribution

Eevee and other Pokémon names and character-related marks remain with their respective rights holders. This independent project is not affiliated with, sponsored by, or endorsed by The Pokémon Company, Nintendo, Game Freak, or Creatures. Its original wordmark is not an official Pokémon logo.

Making the source repository public does not resolve artwork, naming, trademark, commercial-use, or provider-service eligibility. Version 0.1.0 distributes original code and dependency notices without franchise image bytes; that software release is not artwork/trademark clearance or a grant of provider access. Before a use requiring additional permission, obtain the relevant authorization or replace/remove the affected artwork and branding; this notice is not a substitute for that authorization.

When adding or replacing an asset, record its actual source, fixed version, owner/notice, applicable permission, modifications and package scope. Do not copy character artwork into the source tree or release archive under the project's software license.
