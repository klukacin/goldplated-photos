# Goldplated Photos — Desktop

Native photo management and gallery publishing. Rust core, Tauri shell.
See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and the reasoning.

```
desktop/
├── crates/gpp-core     all logic — catalog, import, albums, publish, sync
├── crates/gpp-cli      `gpp`, a CLI over the core (works everywhere)
└── app/                Tauri shell (macOS first; iOS target ready)
```

---

## What works today

| Capability | State |
|---|---|
| Import (scan, hash, EXIF, thumbnails, dedup, incremental) | done |
| Rating: 0–5 stars, pick/reject, colour labels, bulk | done |
| Albums: hierarchy, membership, ordering, tags, rename/move | done |
| Publish to the Astro gallery (access, proofing, download flags) | done |
| Sync planning that cannot wipe another machine's albums | done |
| Per-album two-way sync: adopt an album, contribute one, from any machine | done |
| Paths stay whole: folders travel with their albums, both directions | done |
| Develop: non-destructive adjustments, applied on publish | done |
| RAW decoding | not yet — `RawDecoder` trait is in place |

---

## Build status, honestly

The **core and CLI build and test on any platform**, including Linux CI
containers — that is the point of keeping the GUI out of the core:

```bash
cd desktop
cargo test          # 109 tests
cargo clippy --all-targets
cargo build --release -p gpp-cli
```

The **Tauri shell has been compiled and run** — on Linux/WebKitGTK, under a
virtual display. It opens a library, lists albums, renders the grid from the
thumbnail cache, rates photos from the keyboard, fills the inspector, and
drives the sync panel against a real remote. That first run found five defects
no amount of static checking would have: a missing `protocol-asset` feature, a
`PublishResult` that was never `Serialize`, no `capabilities/` file at all (so
Tauri v2 denied every plugin and core API), `.welcome`/`.app` CSS overriding
the `hidden` attribute, and a CSP with no `connect-src ipc:`.

**macOS is still unproven.** WKWebView is a different engine, and codesigning
and notarization are untested. What is now known to work everywhere is the part
that was riskiest: the command surface, the permission and CSP configuration,
and the UI's own logic.

To reproduce the run on a headless Linux box:

```bash
apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
                   patchelf xvfb openbox
Xvfb :77 -screen 0 1400x900x24 &
DISPLAY=:77 openbox &
cd desktop/app/src-tauri && cargo build
DISPLAY=:77 ./target/debug/gpp-desktop ~/Photos     # opens that library directly
```

A library path on the command line skips the folder picker — handy for scripts
and for exactly this kind of testing.

---

## Running on macOS

Prerequisites: Xcode command line tools, Rust, Node (for the Tauri CLI only).

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

cd desktop/app
npm install -g @tauri-apps/cli    # or: cargo install tauri-cli --version '^2'

cargo tauri dev                   # dev build with hot reload
cargo tauri build                 # produces .app and .dmg
```

The bundle lands in `desktop/app/src-tauri/target/release/bundle/`.

### Distribution

An unsigned build runs locally but Gatekeeper will warn other users. For
distribution you need an Apple Developer account, then codesigning and
notarization — set `APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_PASSWORD` and
`APPLE_TEAM_ID` and Tauri handles the rest during `cargo tauri build`.

---

## Using the CLI

The CLI drives the same core and is the fastest way to try things.

```bash
export GPP_LIBRARY=~/Photos

gpp init                                   # create the catalog
gpp import ~/Photos/2026-ana               # scan + thumbnails
gpp ls --min-rating 4 --sort rating
gpp rate 12,13,14 5
gpp flag 15 reject

gpp album create 2026/ana-ivan --title "Ana & Ivan"
gpp album add 2026/ana-ivan --ids 12,13,14
gpp album set 2026/ana-ivan --password tajna --share-link --proofing

gpp publish 2026/ana-ivan \
    --dest ../src/content/albums \
    --min-rating 4
```

That last command writes exactly what the web gallery reads. Deploy as usual
afterwards.

---

## Developing photos

Adjustments are recorded, never baked in. The original file is never opened for
writing — adjusted pixels only ever appear in derived places: the thumbnail
cache and whatever gets published.

```bash
gpp develop 12,13,14 --exposure -0.5 --contrast 20 --shadows 15
gpp develop 12 --show
#   exposure     -0.50 EV
#   contrast     +20
#   shadows      +15
gpp develop 12,13,14 --reset      # back to the original
```

Available: `--exposure` (stops), `--contrast`, `--saturation`, `--temperature`,
`--tint`, `--highlights`, `--shadows`, `--bw`, `--rotate 0-3`, `--flip-h`,
`--flip-v`, `--crop x,y,w,h`. Every value is an upsert, so setting a slider
twice replaces it rather than stacking, and setting it back to zero removes it.

**Why it stays fast.** Every derived image is addressed by a *render key*: the
content hash for an untouched photo, a hash of content plus edits for an
adjusted one. So a photo with no edits keeps the thumbnails it already had,
changing an edit can never show a stale thumbnail, and reverting lands back on
a key that is usually still cached. Publishing copies a cached full-size render
rather than re-developing, so an album of adjusted photos costs one render each,
not one per publish.

Geometry runs before tone, which is why cropping and then adjusting behaves the
way it looks like it should: the tone operators only see the pixels that
survived the crop.

---

## Working from more than one machine

Every album syncs on its own, in the direction you choose for it. Albums you
haven't chosen a direction for are never touched — not pushed, not pulled, not
deleted — which is what lets a laptop carry three albums out of two hundred.

```bash
# --remote and --dest are remembered in the catalog after the first use
gpp remote --remote /Volumes/gallery --dest ../src/content/albums

gpp remote                                 # what's here, what's there
# ALBUM                       FILES  LOCAL   REMOTE  SYNC
# 2026                           10  —       yes     not tracked
# 2026/weddings                   6  —       yes     not tracked   ← a folder
# 2026/weddings/ana-ivan          3  —       yes     not tracked   ← can adopt
# 2026/marko                      0  yes     —       not tracked   ← can contribute

gpp pull 2026/weddings/ana-ivan            # adopt: settings, photos, order
gpp pull 2026/weddings                     # or a whole folder at once
gpp push 2026/marko                        # contribute one of your own
gpp sync 2026/weddings --both              # from then on, both ways
gpp sync                                   # every tracked path, its own way
```

**Paths stay whole.** `pull` and `push` take a path, not just a leaf, and always
carry the folders above it: pushing `2026/weddings/ana-ivan` puts `2026/index.md`
and `2026/weddings/index.md` on the server too, because without them the gallery
cannot navigate to the album. Pulling brings those folders back, and every album
lands in its own directory — nothing is flattened.

Only the path you name is subscribed. Pulling `2026/weddings` tracks
`2026/weddings`; the `2026` folder comes along because the site needs it, not
because this machine now wants every album of the year.

A push creates a missing parent folder but never rewrites one the server
already has — another machine may have named or password-protected it, and a
leaf push makes no claim about that:

```bash
gpp push 2026/weddings/ivona-petar
# 2 parent folder(s) already on the server, configured elsewhere — left alone:
#   2026
#   2026/weddings
# push a folder directly to change it: gpp push <folder>
```

To change a shared folder, pull it first (adopting what is online), edit, then
push — otherwise the two versions are a genuine conflict and nothing is
overwritten.

A pulled album keeps the server's `token`, so gallery access cookies and share
links stay valid across machines. A one-off `pull` or `push` won't overrule a
direction you already set.

Nothing is removed from the server unless you say so:

```bash
gpp push 2026/ana-ivan
# 1 file(s) on the server that this album no longer has:
#   2026/ana-ivan/reject.jpg
# re-run with --allow-deletes to remove them
```

In the app this is the **Sync…** panel: pick the folder, choose a direction per
album, and Pull / Push / Sync per row. Deletions ask first, by name.

The remote is a directory today — a network share, an external drive, or a
folder something else keeps in sync. `SftpTransport` and `HttpTransport` slot in
behind the same trait without touching any of the logic above.

---

## Working on the core in parallel

The layout is built so several streams of work can run at once without
colliding. Three properties do the heavy lifting:

**The core has no GUI dependency.** `gpp-core` and `gpp-cli` build and test on
any machine, in any container, with no display and no webview. Anyone can work
on catalog, import, albums, publish, sync or develop without ever touching the
shell.

**The shell is outside the cargo workspace.** `desktop/app/src-tauri` has its
own `Cargo.lock`, so a platform-specific dependency there cannot break the
core's build for everyone else.

**The seams are traits and one method per command.** `RemoteTransport`,
`RawDecoder` and `Session` are the boundaries; work on either side of one only
needs the signature to stay put.

A worktree per stream keeps builds from fighting over `target/`:

```bash
git worktree add ../gpp-develop  -b feature/develop-pipeline
git worktree add ../gpp-raw      -b feature/raw-decoder
# each has its own target/ and its own catalog fixtures
```

Before pushing anything that touches the shell:

```bash
npm run check:shell          # contract checks — no GUI needed, ~50ms
cd desktop && cargo clippy --all-targets -- -D warnings && cargo test
```

`check:shell` exists because the shell is the one place unit tests cannot
reach. Every check in it is a bug that actually shipped: an unregistered
command, a missing element id, a missing capability, a CSP without `ipc:`, an
asset protocol enabled in config but not compiled in, a `hidden` attribute
beaten by CSS, and a build script that did not notice the frontend changed. CI
runs it, then compiles the shell against WebKitGTK.

---

## How this fits the web gallery

The desktop app **feeds** the existing Astro site; it does not replace it.
Publishing materializes `src/content/albums/<path>/index.md` plus the selected
photos, with frontmatter matching the site's content schema. That mapping is
asserted by a test (`publish::tests::emitted_fields_are_a_subset_of_the_site_schema`)
so a drift fails the build instead of producing an album the site refuses to
render.

Three things the app deliberately never touches:

- **`.meta/proofing/`** — client selections are written on the server. They are
  pull-only and excluded from every manifest.
- **Files this machine has never seen** — the sync planner marks them
  `LeaveAlone`. A laptop holding one year of work cannot delete the rest.
- **Files this library did not publish** — the web admin and the app share the
  content folder, so publishing prunes only what it wrote itself.

---

## Path to iPad

The core compiles for `aarch64-apple-ios` unchanged; what differs is the shell.
Two seams already exist for it:

- `sync::RemoteTransport` — SFTP on desktop, HTTP on iPad (no SSH there).
- File access — document picker instead of arbitrary paths.

```bash
cargo tauri ios init
cargo tauri ios dev
```

---

## Licensing note

Every dependency is MIT / Apache-2.0 / BSD / CC0 — nothing copyleft links into
the binary. When RAW support arrives, LibRaw is dual-licensed **LGPL-2.1 or
CDDL-1.0**; CDDL permits static linking into a proprietary product, requiring
only that modifications to LibRaw's own files be published. Keep RAW behind
`media::RawDecoder` so that decision stays at the edge of the build. Have a
lawyer confirm before shipping commercially.
