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
| RAW decoding, develop pipeline | not yet — `RawDecoder` trait is in place |

---

## Build status, honestly

The **core and CLI build and test on any platform**, including Linux CI
containers — that is the point of keeping the GUI out of the core:

```bash
cd desktop
cargo test          # 61 tests
cargo clippy --all-targets
cargo build --release -p gpp-cli
```

The **Tauri shell has not been compiled here**. It needs a platform webview
toolkit (WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux) which
this development container does not have. What *was* verified mechanically:
the config parses, the UI's JavaScript parses, and every one of the 16
`invoke()` calls in the UI matches a `#[tauri::command]` that is registered in
`generate_handler!`. First compile on a Mac may still surface something.

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

## How this fits the web gallery

The desktop app **feeds** the existing Astro site; it does not replace it.
Publishing materializes `src/content/albums/<path>/index.md` plus the selected
photos, with frontmatter matching the site's content schema. That mapping is
asserted by a test (`publish::tests::emitted_fields_are_a_subset_of_the_site_schema`)
so a drift fails the build instead of producing an album the site refuses to
render.

Two things the app deliberately never touches:

- **`.meta/proofing/`** — client selections are written on the server. They are
  pull-only and excluded from every manifest.
- **Files this machine has never seen** — the sync planner marks them
  `LeaveAlone`. A laptop holding one year of work cannot delete the rest.

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
