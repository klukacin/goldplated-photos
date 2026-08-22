#!/usr/bin/env node
/**
 * Contract checks for the Tauri shell.
 *
 * The shell is the one part of this project that cannot be unit-tested: it only
 * exists once a platform webview is running. Every check here corresponds to a
 * real defect that shipped and was only caught by launching the app by hand —
 * a class of bug where the build is green, the tests pass, and the window comes
 * up empty or inert.
 *
 * Fast, deterministic, no GUI. Runs in CI on every push.
 */

import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const ui = join(here, 'ui');
const tauri = join(here, 'src-tauri');

const failures = [];
const notes = [];

function fail(check, detail) {
  failures.push(`${check}\n    ${detail}`);
}
function ok(check, detail) {
  notes.push(`${check}${detail ? ` — ${detail}` : ''}`);
}

const appJs = readFileSync(join(ui, 'app.js'), 'utf8');
const indexHtml = readFileSync(join(ui, 'index.html'), 'utf8');
const styleCss = readFileSync(join(ui, 'style.css'), 'utf8');
const libRs = readFileSync(join(tauri, 'src', 'lib.rs'), 'utf8');
const cargoToml = readFileSync(join(tauri, 'Cargo.toml'), 'utf8');
const conf = JSON.parse(readFileSync(join(tauri, 'tauri.conf.json'), 'utf8'));

// ---------------------------------------------------------------- commands

/** Command names the UI calls. */
const invoked = [...appJs.matchAll(/invoke\('([a-z_]+)'/g)].map((m) => m[1]);
/** Command names registered in generate_handler!. */
const handlerBlock = libRs.slice(
  libRs.indexOf('generate_handler!['),
  libRs.indexOf('])', libRs.indexOf('generate_handler![')),
);
const registered = [...handlerBlock.matchAll(/^\s+([a-z_]+),$/gm)].map((m) => m[1]);

const unregistered = [...new Set(invoked)].filter((c) => !registered.includes(c));
if (unregistered.length) {
  fail('invoke() calls a command that is not registered', unregistered.join(', '));
} else {
  ok('every invoke() is registered', `${new Set(invoked).size} commands`);
}

const undefined_ = registered.filter(
  (c) => !new RegExp(`^(async )?fn ${c}\\(`, 'm').test(libRs),
);
if (undefined_.length) {
  fail('registered command has no function', undefined_.join(', '));
} else {
  ok('every registered command is defined', `${registered.length} commands`);
}

// ------------------------------------------------------------- element ids

const wanted = [...new Set([...appJs.matchAll(/\$\('([a-z-]+)'\)/g)].map((m) => m[1]))];
const present = new Set(
  [...indexHtml.matchAll(/id="([a-z-]+)"/g)].map((m) => m[1]),
);
const missing = wanted.filter((id) => !present.has(id));
if (missing.length) {
  fail('UI reaches for an element id that is not in the markup', missing.join(', '));
} else {
  ok('every element id exists', `${wanted.length} ids`);
}

// ------------------------------------------------------------ permissions

// Tauri v2 denies every core and plugin API unless a capability grants it.
// Without this file the window comes up and nothing works: no events, no
// dialogs, and the failure is a rejected promise the UI usually swallows.
const capDir = join(tauri, 'capabilities');
if (!existsSync(capDir) || readdirSync(capDir).filter((f) => f.endsWith('.json')).length === 0) {
  fail(
    'no capability file',
    'Tauri v2 denies all core and plugin APIs without capabilities/*.json',
  );
} else {
  const perms = readdirSync(capDir)
    .filter((f) => f.endsWith('.json'))
    .flatMap((f) => JSON.parse(readFileSync(join(capDir, f), 'utf8')).permissions ?? []);

  // Which plugin APIs the UI actually reaches for, and what has to allow them.
  const needs = [
    { used: /window\.__TAURI__\.event|listen\(/, perm: /^core:(default|event)/, what: 'event.listen' },
    { used: /openDialog\(|\.dialog\b/, perm: /^dialog:/, what: 'dialog.open' },
    { used: /askConfirm\(/, perm: /^dialog:(default|allow-confirm|allow-ask)/, what: 'dialog.confirm' },
  ];
  for (const { used, perm, what } of needs) {
    if (used.test(appJs) && !perms.some((p) => perm.test(p))) {
      fail(`UI uses ${what} but no capability grants it`, `permissions: ${perms.join(', ')}`);
    }
  }
  ok('capabilities grant what the UI uses', perms.join(', '));
}

// ----------------------------------------------------- confirmation dialogs

// window.confirm() inside a Tauri webview is not a prompt on every platform:
// WebKitGTK has no script-dialog handler, so it returns true and the guarded
// action happens unasked. Deleting an album and deleting files off the server
// both hang off one of these.
const bareConfirm = /(^|[^.\w])confirm\s*\(/m.test(
  appJs.replace(/askConfirm\s*\(/g, 'askConfirmCall('),
);
if (bareConfirm) {
  fail(
    'UI calls the webview\'s own confirm()',
    'it returns true without asking on Linux — use the dialog plugin instead',
  );
} else {
  ok('no reliance on the webview\'s confirm()');
}

// -------------------------------------------------------------------- CSP

const csp = conf.app?.security?.csp ?? '';
// invoke() rides the ipc: scheme. Some webviews fall back to postMessage and
// survive a missing connect-src; the fetch-based IPC does not, and the whole
// app is then inert.
if (!/connect-src[^;]*\bipc:/.test(csp)) {
  fail('CSP has no connect-src for ipc:', `every invoke() is a policy violation\n    csp: ${csp}`);
} else {
  ok('CSP allows the ipc: scheme');
}
if (/convertFileSrc/.test(appJs) && !/img-src[^;]*asset:/.test(csp)) {
  fail('UI loads images via convertFileSrc but CSP has no asset: in img-src', csp);
}

// --------------------------------------------------------- asset protocol

// convertFileSrc is served by the asset protocol, which is a cargo feature.
// Enabling it in the config alone makes the build fail with an allowlist error.
if (/convertFileSrc/.test(appJs)) {
  const enabled = conf.app?.security?.assetProtocol?.enable === true;
  const feature = /tauri\s*=\s*\{[^}]*features\s*=\s*\[[^\]]*"protocol-asset"/s.test(cargoToml);
  if (!enabled) fail('UI uses convertFileSrc but assetProtocol is not enabled', 'tauri.conf.json');
  if (!feature) fail('assetProtocol needs the protocol-asset cargo feature', 'src-tauri/Cargo.toml');
  if (enabled && feature) ok('asset protocol enabled and compiled in');
}

// ----------------------------------------------------------- hidden panels

// The UI shows and hides panels with the `hidden` attribute. A class that sets
// `display` beats it, and the panel stays on screen — which is how the welcome
// screen once covered a fully working app forever.
if (/\[hidden\]\s*\{[^}]*display:\s*none/.test(styleCss)) {
  ok('[hidden] is enforced in CSS');
} else {
  const hiddenIds = [...indexHtml.matchAll(/id="([a-z-]+)"[^>]*\shidden/g)].map((m) => m[1]);
  fail(
    'no CSS rule enforces the hidden attribute',
    `${hiddenIds.length} elements rely on it; any class setting display will override it`,
  );
}

// ---------------------------------------------------------- frontend rebuild

// The UI is embedded at compile time. Without rerun-if-changed the build script
// does not re-run, and `cargo build` silently produces a binary carrying the
// previous frontend.
// Read the UI directory rather than a list written down here: a list would
// itself need remembering, and the file somebody forgets to add is exactly the
// one whose edits then vanish from the build.
const buildRs = readFileSync(join(tauri, 'build.rs'), 'utf8');
const watched = readdirSync(ui)
  .filter((f) => /\.(html|js|css)$/.test(f))
  .filter((f) => !buildRs.includes(f));
if (watched.length) {
  fail('build.rs does not watch every UI file', `missing: ${watched.join(', ')}`);
} else {
  ok('build.rs watches the UI files');
}

// -------------------------------------------------------------------- done

for (const n of notes) console.log(`  ok  ${n}`);
if (failures.length) {
  console.error(`\n${failures.length} shell contract failure(s):\n`);
  for (const f of failures) console.error(`  ✗ ${f}\n`);
  process.exit(1);
}
console.log(`\nShell contract: ${notes.length} checks passed.`);
