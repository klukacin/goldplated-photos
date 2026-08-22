#!/usr/bin/env node
/**
 * Hold the Rust dependency tree to the project's licence policy.
 *
 * The rule this enforces is stated in `desktop/Cargo.toml` and has already been
 * broken once by accident: switching TLS backends quietly pulled in a vendored
 * root-certificate bundle under CDLA-Permissive-2.0, and it was caught by
 * reading a dependency list by hand. That is not a process that survives.
 *
 * Two things make this harder than grepping `cargo metadata`:
 *
 * 1. `cargo metadata` lists the whole resolve graph, including dependencies
 *    that are gated to platforms we do not ship and never actually compiled.
 *    That same CDLA crate is still *in* the graph and still linked into
 *    nothing. Checking it would cry wolf, and the obvious way to silence the
 *    wolf — allowing the licence — is how the real thing gets through next
 *    time. So the check runs per shipping target and asks what is genuinely
 *    linked.
 * 2. A licence field is an SPDX expression, not a name. `MIT OR Apache-2.0`
 *    needs one of the two to be acceptable; `Apache-2.0 AND ISC` needs both.
 */
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const DESKTOP = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'desktop');

/** Everywhere the core is meant to compile. The portability contract in `gpp-core/src/lib.rs`. */
const TARGETS = [
  'x86_64-unknown-linux-gnu',
  'aarch64-apple-darwin',
  'x86_64-pc-windows-msvc',
  'aarch64-apple-ios',
];

/**
 * Permissive, no source-disclosure obligation, no attribution burden beyond a
 * notice file. Anything outside this list is a decision for a person, not a
 * default — which is the whole point of the check.
 */
const ALLOWED = new Set([
  'MIT',
  'MIT-0',
  'Apache-2.0',
  'Apache-2.0 WITH LLVM-exception',
  'BSD-2-Clause',
  'BSD-3-Clause',
  '0BSD',
  'ISC',
  'CC0-1.0',
  'Unlicense',
  'Zlib',
  // The Unicode licence, carried by the ICU data crates. Permissive and
  // universally accepted; listed explicitly so nobody has to re-decide it.
  'Unicode-3.0',
]);

/** Our own crates carry no third-party obligation. */
const OURS = new Set(['gpp-core', 'gpp-cli', 'gpp-ffi', 'gpp-desktop']);

/**
 * Is this SPDX expression satisfied by something we allow?
 *
 * Parsed rather than split, because splitting gets `(MIT OR Apache-2.0) AND
 * Unicode-3.0` wrong — and wrong in the direction that matters. Ignore the
 * parentheses and the leading `MIT` looks like a complete alternative, so the
 * expression is accepted while the licence it actually also requires goes
 * unexamined. AND binds tighter than OR; the grammar below says so.
 */
export function acceptable(expression) {
  const tokens = tokenise(expression);
  if (!tokens.length) return false;

  let at = 0;
  const peek = () => tokens[at];
  const take = () => tokens[at++];

  // expr := term (OR term)*   — any alternative will do.
  function expr() {
    let value = term();
    while (peek()?.toUpperCase() === 'OR') {
      take();
      // Evaluate both sides: a malformed tail should not be skipped silently.
      value = term() || value;
    }
    return value;
  }

  // term := factor (AND factor)*   — every conjunct applies, so all must pass.
  function term() {
    let value = factor();
    while (peek()?.toUpperCase() === 'AND') {
      take();
      value = factor() && value;
    }
    return value;
  }

  function factor() {
    if (peek() === '(') {
      take();
      const value = expr();
      if (peek() === ')') take();
      return value;
    }
    let id = take() ?? '';
    // "Apache-2.0 WITH LLVM-exception" is one identifier, not two.
    if (peek()?.toUpperCase() === 'WITH') {
      take();
      id = `${id} WITH ${take() ?? ''}`;
    }
    return ALLOWED.has(id);
  }

  const result = expr();
  // Anything left over means the expression had a shape this parser does not
  // understand. Refuse to vouch for it rather than guess.
  return at === tokens.length && result;
}

function tokenise(expression) {
  return expression
    // The historical `MIT/Apache-2.0` form predates SPDX expressions.
    .replaceAll('/', ' OR ')
    .replaceAll('(', ' ( ')
    .replaceAll(')', ' ) ')
    .split(/\s+/)
    .filter(Boolean);
}

/** package name+version -> { licence, targets } for everything actually linked. */
function linkedPackages() {
  const found = new Map();
  for (const target of TARGETS) {
    let out;
    try {
      out = execFileSync(
        'cargo',
        ['tree', '-e', 'normal', '--target', target, '--prefix', 'none',
         '--format', '{p}|{l}', '--workspace'],
        { cwd: DESKTOP, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
      );
    } catch (err) {
      console.error(`cargo tree failed for ${target}:\n${err.stderr || err.message}`);
      process.exit(2);
    }

    for (const line of out.split('\n')) {
      const trimmed = line.trim();
      if (!trimmed || !trimmed.includes('|')) continue;
      const [pkg, rawLicence] = trimmed.split('|');
      const name = pkg.trim().split(' ')[0];
      if (OURS.has(name)) continue;

      // cargo tree marks a subtree it has already printed with a trailing
      // "(*)", which lands inside the licence field and would otherwise be
      // read as part of the expression.
      const licence = (rawLicence ?? '').replace(/\s*\(\*\)\s*$/, '').trim();

      const key = pkg.trim();
      const entry = found.get(key) ?? { licence, targets: new Set() };
      // A repeat sighting carries the same licence; keep whichever is non-empty
      // so a dedup line cannot blank out a known one.
      if (!entry.licence && licence) entry.licence = licence;
      entry.targets.add(target);
      found.set(key, entry);
    }
  }
  return found;
}

function main() {
  const packages = linkedPackages();
  const offenders = [];
  const unknown = [];

  for (const [pkg, { licence, targets }] of packages) {
    if (!licence || licence === '(unknown)') {
      unknown.push({ pkg, targets });
    } else if (!acceptable(licence)) {
      offenders.push({ pkg, licence, targets });
    }
  }

  if (offenders.length || unknown.length) {
    console.error('Licence policy violated.\n');
    for (const { pkg, licence, targets } of offenders) {
      console.error(`  ${pkg}`);
      console.error(`    ${licence}`);
      console.error(`    linked on: ${[...targets].join(', ')}\n`);
    }
    for (const { pkg, targets } of unknown) {
      console.error(`  ${pkg}\n    no licence declared\n    linked on: ${[...targets].join(', ')}\n`);
    }
    console.error(
      'Allowed: ' + [...ALLOWED].join(', ') + '\n' +
      'If one of these is genuinely fine, add it to ALLOWED with a comment saying why.',
    );
    process.exit(1);
  }

  console.log(
    `Licences OK — ${packages.size} third-party crates across ${TARGETS.length} targets, ` +
    'all permissive.',
  );
}

// Importable for its expression parser, which is the part worth testing; only
// shells out to cargo when run as a command.
if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) main();
