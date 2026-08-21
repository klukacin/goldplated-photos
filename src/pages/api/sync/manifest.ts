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
import { checkSyncAuth, safeScope } from '../../../lib/sync-auth';
import { blake3Hex, CONTENT_ROOT, jsonError } from './_shared';

export const prerender = false;

export const GET: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const scope = safeScope(url.searchParams.get('scope'));
  if (scope === null) return jsonError('Invalid scope', 400);

  const root = scope ? path.join(CONTENT_ROOT, scope) : CONTENT_ROOT;
  const files: Array<{ path: string; hash: string }> = [];

  try {
    await walk(root, files);
  } catch (err: unknown) {
    // A scope that does not exist yet is an empty manifest, not an error: it is
    // exactly what a first push looks like.
    if ((err as NodeJS.ErrnoException)?.code !== 'ENOENT') throw err;
  }

  return new Response(JSON.stringify({ files }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};

async function walk(dir: string, out: Array<{ path: string; hash: string }>) {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    // Dotfiles stay server-owned and invisible to sync — `.meta/proofing` above
    // all, which the client must never overwrite or delete.
    if (entry.name.startsWith('.')) continue;

    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      await walk(full, out);
    } else if (entry.isFile()) {
      out.push({
        path: path.relative(CONTENT_ROOT, full).split(path.sep).join('/'),
        hash: await blake3Hex(full),
      });
    }
  }
}
