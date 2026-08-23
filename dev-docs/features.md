# Feature flags — turning parts of the system off

Two mechanisms, chosen to match what each side is:

- **The web side is configured at runtime** — environment variables, one shared
  module, no rebuild for SSR routes.
- **The desktop side is configured at build time** — Cargo features gate whole
  dependencies out of the binary, plus a tiny UI-level switch file for
  controls that cost nothing to compile.

A flag that hides UI without disabling the route is a decoration, not a flag.
Every switch below disables the *capability*: the route answers 404, the
extension leaves the accept lists, the dependency leaves the binary.

## Web gallery + admin: `src/site-features.mjs`

One plain-ESM module is the single source of truth, loaded by Astro/TS
(`src/config.ts` re-exports it as `siteConfig.features`), by
`src/lib/image-formats.ts` (extension lists derive from it), by
`admin/server.js` directly, and handed to the admin's browser JS through
`GET /api/config` — classic `<script>`s cannot import, and hand-copied lists
are how the drift this module killed got here in the first place.

Flags come from the environment (or `.env`) and default **on**, so an
unconfigured install behaves exactly as before flags existed. `0`, `false`,
`off`, `no` switch one off.

| Variable | Default | Off means |
|---|---|---|
| `FEATURE_HEIC` | on | `.heic`/`.heif` leave every extension list: gallery discovery, admin upload filter, media routes |
| `FEATURE_WATERMARK` | on | `/api/watermark` answers 404; the share flow loses its Instagram image |
| `FEATURE_SEARCH` | on | `/photos/search` 404s and the `/photos` search box is not rendered |
| `FEATURE_TAGS` | on | tag pages and tag pills gone |
| `FEATURE_PROOFING` | on | global kill-switch over the per-album setting: `/api/proofing` 404s, no hearts render |
| `FEATURE_PHOTO_SHARING` | on | per-photo share links/OG deep links off |
| `FEATURE_VIDEO_THUMBNAILS` | **off** | video poster generation |
| `SLIDESHOW_INTERVAL_MS` | 5000 | not a switch — the slideshow cadence |

**The prerender caveat:** SSR routes read flags at server start, so a change is
one restart away. Tag pages and the `/photos` search box are *prerendered* —
baked by `astro build` — so for those a flag change takes effect on the next
build. The module doc says the same thing; it is the one non-obvious rule.

`BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS` is deliberately **not** flag-widened:
no configuration can make Chrome paint a HEIC, so no flag may claim otherwise.

## Desktop core: Cargo features

```toml
# default build                        # HEIF-less build
cargo build                            cargo build --no-default-features
cargo test --workspace                 cargo test -p gpp-core --no-default-features
```

| Feature | Default | Off means |
|---|---|---|
| `heif` | on | `heif-oxide` (the heaviest dependency the crate links) leaves the binary; `.heic`/`.heif` stop being photo extensions, so the scan does not catalogue them — exactly the pre-HEIF behaviour, rather than cataloguing files nothing can decode |

The feature is plumbed through `gpp-cli`, `gpp-ffi` and the Tauri shell, so a
`--no-default-features` build is HEIF-less end to end. Both configurations run
the full suite and clippy in verification; HEIF-specific tests are gated on
the feature, and the off-configuration has its own test asserting a `.heic`
classifies as nothing.

## Desktop UI: `desktop/app/ui/features.js`

A plain const object loaded before `app.js`, for controls that cost nothing to
compile but a photographer may not want on screen:

| Key | Default | Off means |
|---|---|---|
| `crop` | true | the ⛶ button and the framing overlay never render, and `enterCrop` refuses |

## Adding a flag — the checklist

**Web:** add it to `resolveFeatures` in `site-features.mjs` with a one-line
doc; enforce it at the *route* (404) and only then in the UI; if any consumer
is prerendered, say so; add a test proving OFF actually disables (the
route-level tests in `tests/feature-flags-routes.test.ts` are the template,
`tests/site-features.test.ts` for the pure resolution).

**Core:** a Cargo feature gating the dependency (`dep:` syntax) and every code
path; propagate through the three consumer crates; decide what OFF *means* and
write it at the feature declaration (the `heif` comment is the template); both
configurations through tests and clippy; if the feature affects publish
naming or extension lists, check `published_filename` and the fixtures.

**Desktop UI:** a key in `features.js` with a doc line; hide at boot *and*
refuse in the action — hiding alone leaves the keyboard path alive.
