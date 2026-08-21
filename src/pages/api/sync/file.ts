/**
 * Read, write or remove one published file.
 *
 * The client has already diffed against `/api/sync/manifest`, so every request
 * here is one it decided was necessary. Writes are verified against the hash
 * the client declares and land atomically, so a dropped connection cannot leave
 * a half-written photo in the gallery.
 */
import type { APIRoute } from 'astro';
import fs from 'node:fs/promises';
import path from 'node:path';
import { checkSyncAuth, safeRelPath } from '../../../lib/sync-auth';
import { blake3HexOf, CONTENT_ROOT, jsonError } from './_shared';

export const prerender = false;

/** 512 MB: far above any photo, far below anything that could exhaust the box. */
const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;

function resolve(url: URL): string | null {
  const rel = safeRelPath(url.searchParams.get('path'));
  if (!rel) return null;
  const full = path.join(CONTENT_ROOT, rel);
  // Belt and braces: even with the path rules above, never act outside the root.
  if (!full.startsWith(CONTENT_ROOT + path.sep)) return null;
  return full;
}

export const GET: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const full = resolve(url);
  if (!full) return jsonError('Invalid path', 400);

  try {
    const bytes = await fs.readFile(full);
    return new Response(new Uint8Array(bytes), {
      status: 200,
      headers: { 'Content-Type': 'application/octet-stream' },
    });
  } catch {
    return jsonError('Not found', 404);
  }
};

export const PUT: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const full = resolve(url);
  if (!full) return jsonError('Invalid path', 400);

  const body = new Uint8Array(await request.arrayBuffer());
  if (body.byteLength > MAX_UPLOAD_BYTES) {
    return jsonError('Too large', 413);
  }

  // Reject rather than publish a file that did not survive the wire intact.
  const declared = request.headers.get('x-content-blake3');
  if (declared) {
    const actual = blake3HexOf(body);
    if (actual !== declared) {
      return jsonError(`Hash mismatch: declared ${declared}, received ${actual}`, 422);
    }
  }

  await fs.mkdir(path.dirname(full), { recursive: true });
  // Write beside the target and rename: rename is atomic within a filesystem,
  // so a reader never sees a partial photo and a dropped upload leaves nothing.
  const temp = `${full}.upload-${process.pid}-${Date.now()}`;
  try {
    await fs.writeFile(temp, body);
    await fs.rename(temp, full);
  } catch (err) {
    await fs.rm(temp, { force: true });
    throw err;
  }

  return new Response(null, { status: 204 });
};

export const DELETE: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const full = resolve(url);
  if (!full) return jsonError('Invalid path', 400);

  await fs.rm(full, { force: true });
  // Tidy up a directory the deletion emptied, but never complain if it is not
  // empty — another album's files may share the parent.
  await fs.rmdir(path.dirname(full)).catch(() => {});
  return new Response(null, { status: 204 });
};
