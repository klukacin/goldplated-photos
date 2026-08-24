/**
 * What the server currently holds, so the client can send only what is missing.
 *
 * One request covers a whole album — that single round trip is the entire
 * benefit rsync's file list ever gave us here. Hashes are BLAKE3, matching what
 * the desktop core computes, so the comparison is cryptographic rather than an
 * mtime heuristic.
 */
import type { APIRoute } from 'astro';

import fs from 'node:fs/promises';
import path from 'node:path';
import { checkSyncAuth, safeScope, splitSyncPath } from '../../../lib/sync-auth';
import { cachedHash, flushHashCache } from './_hash-cache';
import { CONTENT_ROOT, FULL_SYNC_ROOT, jsonError } from './_shared';

export const prerender = false;

export const GET: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const scope = safeScope(url.searchParams.get('scope'));
  if (scope === null) return jsonError('Invalid scope', 400);

  const files: Array<{ path: string; hash: string }> = [];
  try {
    if (scope === undefined) {
      // The whole of what sync may see: the gallery tree plus the private
      // full-scope store, the latter reported under its reserved prefix so
      // the same key names the same file on both sides of the wire.
      await walk(CONTENT_ROOT, CONTENT_ROOT, '', files);
      await walk(FULL_SYNC_ROOT, FULL_SYNC_ROOT, '__gpp_full__/', files);
    } else {
      const routed = splitSyncPath(scope);
      if (!routed) return jsonError('Invalid scope', 400);
      if (routed.tree === 'full') {
        await walk(
          path.join(FULL_SYNC_ROOT, routed.rest),
          FULL_SYNC_ROOT,
          '__gpp_full__/',
          files
        );
      } else {
        await walk(path.join(CONTENT_ROOT, scope), CONTENT_ROOT, '', files);
      }
    }
  } catch (err: unknown) {
    const code = (err as NodeJS.ErrnoException)?.code;
    // A scope that does not exist yet is an empty manifest, not an error: it is
    // exactly what a first push looks like. A scope that names a file rather
    // than a folder is the client's mistake, not the server's — ENOTDIR would
    // otherwise surface as a 500.
    if (code === 'ENOTDIR') return jsonError('Scope is not a folder', 400);
    if (code !== 'ENOENT') throw err;
  }
  await flushHashCache();

  return new Response(JSON.stringify({ files }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};

async function walk(
  dir: string,
  root: string,
  prefix: string,
  out: Array<{ path: string; hash: string }>
) {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    // Dotfiles stay server-owned and invisible to sync — `.meta/proofing` above
    // all, which the client must never overwrite or delete.
    if (entry.name.startsWith('.')) continue;

    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      await walk(full, root, prefix, out);
    } else if (entry.isFile()) {
      const rel = prefix + path.relative(root, full).split(path.sep).join('/');
      out.push({ path: rel, hash: await cachedHash(full, rel) });
    }
  }
}
