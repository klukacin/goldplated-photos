# Developer documentation

Everything a developer needs to work on Goldplated Photos, in one folder.
`CLAUDE.md` at the repo root stays the operational quick-reference (commands,
troubleshooting, the rules an AI session must follow); the documents here are
the long form — why the system is shaped the way it is, what invariants hold,
and how to extend it without breaking them.

| Document | What it covers |
|---|---|
| [architecture.md](architecture.md) | The whole system: gallery, admin, desktop core, shells — and how they fit |
| [core.md](core.md) | `gpp-core` internals: catalog, import, develop, publish |
| [sync.md](sync.md) | The sync engine: three-way state, per-album subscriptions, transports, measured throughput |
| [gallery.md](gallery.md) | The Astro gallery and the admin panel: routes, access control, media pipeline |
| [security.md](security.md) | Threat model and the invariants that hold it up |
| [features.md](features.md) | Feature flags: what can be switched off, where, and how to add a new switch |
| [extending.md](extending.md) | Recipes: add an image format, an edit op, a Session method, an API route, a transport |
| [testing.md](testing.md) | Every suite and check, how to run the app headless, CI, the licence policy — and the traps that have already cost time |

## Ground rules that shape everything

These come up in every document because every design decision leans on them:

- **The filesystem is the source of truth for pixels; everything else is derived.**
  The catalog, thumbnails, renders and published trees can all be rebuilt.
  Nothing in the core ever writes to an original photograph.
- **`gpp-core` stays portable**: no GUI dependencies, no spawning processes,
  platform-specific work behind traits. This is what keeps an iPad build a
  packaging exercise. Breaking it is how that target quietly dies.
- **Every dependency is MIT / Apache-2.0 / BSD / CC0**, enforced per shipping
  target by `scripts/check-licences.mjs`. LGPL and MPL are out, even for
  build-time tools — which is why the C header is hand-written and why HEIF
  decoding is pure Rust.
- **Sync never destroys data it cannot prove is meant to be destroyed.**
  Conflicts are reported, never guessed; deletions need `allow_deletes`;
  untracked albums are untouched on both sides; a pull never overwrites a
  library original.
- **Access control is enforced server-side on every media route**, not in
  markup. A protected album's URLs never appear in page source.
