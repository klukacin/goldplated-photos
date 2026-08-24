/**
 * Bits both sync endpoints need.
 *
 * BLAKE3 rather than a Node built-in because that is what the desktop core
 * already computes for its own sync state — matching it means the client can
 * compare the server's manifest against hashes it did not have to recompute.
 */
import { blake3 } from '@noble/hashes/blake3.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import path from 'node:path';

/** The tree the gallery reads, and the tree web-scope sync moves. */
export const CONTENT_ROOT = path.resolve(process.cwd(), 'src/content/albums');

/**
 * Where the desktop app's `__gpp_full__/` namespace is stored: camera
 * originals and per-album metadata from full-scope sync. Deliberately
 * **outside** the content tree (and git-ignored), so nothing in it is ever
 * built, served, or rsynced to the live site as gallery content — the same
 * token guard, path validation and hash verification apply, only the root
 * differs. See `splitSyncPath` in `src/lib/sync-auth.ts`.
 */
export const FULL_SYNC_ROOT = path.resolve(process.cwd(), '.sync-full');

export function blake3HexOf(bytes: Uint8Array): string {
  return bytesToHex(blake3(bytes));
}

export function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
