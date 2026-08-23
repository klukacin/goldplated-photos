# Testing — every check, and the traps that already cost time

## The suites

| Command | What it covers |
|---|---|
| `npm test` | Vitest: access control, rate limiting, media-info escaping, image formats, sync endpoints (spawns the real servers), admin cross-site guard, develop-queue scheduling, licence-expression parsing |
| `npm run check` | `astro check` — must be 0 errors |
| `npm run build` | the production build (also what CI ships) |
| `cd desktop && cargo test --workspace` | the core: unit tests + `develop_end_to_end`, `two_machines`, `geometry_order` (680-case button matrix), `hostile_remote`, `heif_and_webp`, FFI `foreign_caller` + header-matches-code |
| `cd desktop && cargo clippy --workspace --all-targets -- -D warnings` | must be clean |
| `cargo doc -p gpp-core --no-deps` | doc build — a bad intra-doc link is a warning here and nowhere else |
| `node desktop/app/check-shell.mjs` | the shell contract (below) |
| `npm run check:licences` | every Rust dependency MIT/Apache/BSD/CC0, per shipping target |

CI (`.github/workflows/ci.yml`) runs three jobs on ubuntu: web gallery, desktop
core, desktop shell. macOS/Windows are **not** in CI — a claim about them is
untested until someone adds runners.

## House rules for tests

- **A defect fix needs a test you watched fail first.** Quote the failure in
  the commit. A fix without a demonstrated failure is a hypothesis.
- Prove your test can fail: reintroduce the bug (or a plausible wrong
  implementation) and watch it catch it. Several tests in this repo exist
  precisely because a previous test *couldn't* fail.
- Tests pin behaviour, not representation. When orientation storage went
  canonical, two tests asserting the old op layout broke while behaviour was
  *better* — they now assert pixels and emptiness instead of layouts.

## The shell contract (`check-shell.mjs`)

The Tauri shell cannot be unit-tested, so it has contract checks encoding
defects that shipped and were only found by launching the app: every
`invoke()` registered, every element id present, capabilities granted, CSP
allows `ipc:`, `[hidden]` not overridable, `build.rs` watching **every** UI
file (the list is derived from the directory — a new file cannot be
forgotten), and no bare `confirm()` — WebKitGTK's returns `true` without
asking, which once made "Delete album" delete unasked.

**The UI is embedded into the binary at compile time.** Rebuild after every UI
edit or you are testing the previous JavaScript.

## Running the app without a screen

`desktop/app/run-headless.sh <library>` starts Xvfb, openbox (without a WM,
synthetic clicks land nowhere), a session bus and the XDG portal (without it
the folder picker opens nothing and reports no error). `GPP_DISPLAY`/`GPP_BUS`
let several instances coexist. Drive with `xdotool`, screenshot with
`import -window root`, and *read the screenshots* — several UI defects were
found only by looking.

Create a throwaway library for it with the CLI:
`GPP_LIBRARY=/tmp/lib cargo run -p gpp-cli -- init /tmp/lib && … import`.

## Traps this repo has already paid for

- **Do not share `CARGO_TARGET_DIR` between git worktrees.** One worktree's
  build gets served another's artifacts: it has produced a phantom test
  failure of the very test covering a fix that was present, and a phantom
  extra dozen tests in a count. Each worktree gets its own target dir.
- **Count Rust tests from the `test result:` lines, and check none says
  FAILED.** A naive sum across binaries has hidden two failures in this
  session's history — a failing binary stops the run early and the total still
  looks plausible.
- **Test fixtures vs `.gitignore`.** `*.png`, `*.heic`, `*.webp` are ignored
  so personal photographs stay out — which silently excluded the app icons
  once and the HEIF fixtures once. Everything under
  `desktop/crates/gpp-core/tests/fixtures/` is negated in `.gitignore`; if CI
  fails on a file that plainly exists locally, check `git check-ignore -v`
  first.
- **`git worktree` lives inside the repo** (`.claude/worktrees`). The dev
  server, `npm test` and `astro check` are all scoped to ignore it; a watcher
  that isn't will exhaust file descriptors (`EMFILE`) and die mid-session.
- **A push is not real until `git ls-remote` shows it.** An agent has reported
  "committed and pushed" for a branch that never reached the remote.
- Types crossing the Tauri IPC boundary are built in JS and read by serde: a
  field name serde doesn't recognise is dropped in silence. Multi-word fields
  need `rename_all = "camelCase"` on those structs, and `deny_unknown_fields`
  turns the next typo into an error.
