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

/** The tree the gallery reads, and the only tree sync may touch. */
export const CONTENT_ROOT = path.resolve(process.cwd(), 'src/content/albums');

export function blake3HexOf(bytes: Uint8Array): string {
  return bytesToHex(blake3(bytes));
}

export function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
