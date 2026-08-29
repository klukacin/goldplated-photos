/**
 * Bits both sync endpoints need.
 *
 * BLAKE3 rather than a Node built-in because that is what the desktop core
 * already computes for its own sync state — matching it means the client can
 * compare the server's manifest against hashes it did not have to recompute.
 */
import { blake3 } from '@noble/hashes/blake3.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import fs from 'node:fs/promises';
import path from 'node:path';

/** The tree the gallery reads, and the tree web-scope sync moves. */
export const CONTENT_ROOT = path.resolve(process.cwd(), 'src/content/albums');

/**
 * Where the desktop app's `__gpp_full__/` namespace is stored: camera
 * originals and per-album metadata from full-scope sync. Outside the content
 * tree and git-ignored, so nothing in it is built or rsynced as gallery
 * content; the same token guard, path validation and hash verification apply,
 * only the root differs. See `splitSyncPath` in `src/lib/sync-auth.ts`.
 *
 * **Being outside the content tree is not the same as being unreachable.** In
 * production the process runs with its cwd set to the web root, and Apache
 * serves static files from there directly — it denies only `.ht*`, so a
 * default `<cwd>/.sync-full` would hand out every RAW in it over plain HTTP,
 * with no token and no album password. So: set `SYNC_FULL_ROOT` to a path
 * outside the web root on any server that fronts this app with a web server.
 * The in-tree default stays for development, and `sealPrivateStore()` writes
 * a deny-all `.htaccess` into the store as a second line of defence — which
 * Apache ignores under `AllowOverride None` and nginx never reads at all, so
 * the env var is the real protection and the warning below says so out loud.
 */
export const FULL_SYNC_ROOT = process.env.SYNC_FULL_ROOT?.trim()
  ? path.resolve(process.env.SYNC_FULL_ROOT.trim())
  : path.resolve(process.cwd(), '.sync-full');

// A production server whose store sits under its own working directory is one
// misconfigured web server away from serving camera originals to anyone who
// guesses the path. Say so where an operator will see it — at startup, in the
// log — rather than leaving it to whoever reads .env.example.
if (
  import.meta.env?.PROD &&
  (FULL_SYNC_ROOT === process.cwd() || FULL_SYNC_ROOT.startsWith(process.cwd() + path.sep))
) {
  console.warn(
    `[sync] SYNC_FULL_ROOT is unset, so full-scope originals are stored at ${FULL_SYNC_ROOT}, ` +
      `inside this process's working directory. If a web server serves static files from there, ` +
      `every uploaded original is downloadable without a token. Set SYNC_FULL_ROOT to a path ` +
      `outside the web root.`
  );
}

/** Apache 2.4 and 2.2 spellings of "serve nothing from this directory". */
const DENY_ALL_HTACCESS = `# Full-scope sync store: camera originals and album metadata.
# Private by construction — reachable only through /api/sync/* with SYNC_TOKEN.
<IfModule mod_authz_core.c>
  Require all denied
</IfModule>
<IfModule !mod_authz_core.c>
  Order allow,deny
  Deny from all
</IfModule>
`;

/**
 * Create the private store and make it refuse to be served, for the case the
 * operator left it under a web root anyway. Cheap enough to call per upload:
 * one stat once the file exists. Takes the root rather than reading the
 * constant, so the caller seals the same directory it is about to write into.
 */
export async function sealPrivateStore(root: string): Promise<void> {
  await fs.mkdir(root, { recursive: true });
  const guard = path.join(root, '.htaccess');
  try {
    await fs.access(guard);
  } catch {
    await fs.writeFile(guard, DENY_ALL_HTACCESS);
  }
}

export function blake3HexOf(bytes: Uint8Array): string {
  return bytesToHex(blake3(bytes));
}

export function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
