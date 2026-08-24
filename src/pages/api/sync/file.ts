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
import { checkSyncAuth, safeRelPath, splitSyncPath } from '../../../lib/sync-auth';
import { flushHashCache, forgetHash, rememberHash } from './_hash-cache';
import { blake3HexOf, CONTENT_ROOT, FULL_SYNC_ROOT, jsonError } from './_shared';

export const prerender = false;

/** 512 MB: far above any photo, far below anything that could exhaust the box. */
const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;

function resolve(url: URL): { full: string; rel: string; root: string } | null {
  const rel = safeRelPath(url.searchParams.get('path'));
  if (!rel) return null;
  // A `__gpp_full__/` path is full-scope sync payload — originals, metadata —
  // and is stored under a private root the gallery never serves. Nothing with
  // that prefix may ever land under src/content/albums.
  const routed = splitSyncPath(rel);
  if (!routed) return null;
  const root = routed.tree === 'full' ? FULL_SYNC_ROOT : CONTENT_ROOT;
  const full = path.join(root, routed.rest);
  // Belt and braces: even with the path rules above, never act outside the root.
  if (!full.startsWith(root + path.sep)) return null;
  return { full, rel, root };
}

export const GET: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const target = resolve(url);
  if (!target) return jsonError('Invalid path', 400);
  const { full } = target;

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

  const target = resolve(url);
  if (!target) return jsonError('Invalid path', 400);
  const { full, rel } = target;

  // Integrity is not optional. Every client we have computes this hash anyway,
  // and without it a truncated upload is published as a photo.
  const declared = request.headers.get('x-content-blake3')?.toLowerCase();
  if (!declared) {
    return jsonError('Missing X-Content-Blake3', 400);
  }

  // Refuse an oversized upload from its declared length, before reading it.
  // `arrayBuffer()` below buffers the whole body in memory, so a body measured
  // only after the fact is a body already held — this turns the honest case
  // into a cheap rejection. A client that lies about its length is still
  // caught underneath, just not cheaply.
  const declaredLength = Number(request.headers.get('content-length'));
  if (Number.isFinite(declaredLength) && declaredLength > MAX_UPLOAD_BYTES) {
    return jsonError('Too large', 413);
  }

  const body = new Uint8Array(await request.arrayBuffer());
  if (body.byteLength > MAX_UPLOAD_BYTES) {
    return jsonError('Too large', 413);
  }

  // Reject rather than publish a file that did not survive the wire intact.
  const actual = blake3HexOf(body);
  if (actual !== declared) {
    return jsonError(`Hash mismatch: declared ${declared}, received ${actual}`, 422);
  }
  const verified = actual;

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

  // The hash was computed to verify the upload; keeping it means the next
  // manifest does not read this file again.
  await rememberHash(full, rel, verified);
  await flushHashCache();

  return new Response(null, { status: 204 });
};

export const DELETE: APIRoute = async ({ request, url }) => {
  const auth = checkSyncAuth(request);
  if (!auth.ok) return jsonError(auth.message, auth.status);

  const target = resolve(url);
  if (!target) return jsonError('Invalid path', 400);
  const { full, rel, root } = target;

  // A path may name a directory — `2026/weddings` is a perfectly well-formed
  // request — and `rm` without `recursive` raises EISDIR. Sync deletes files;
  // a directory disappears when the last file in it does.
  try {
    await fs.rm(full, { force: true });
  } catch (err) {
    if ((err as NodeJS.ErrnoException)?.code === 'EISDIR' ||
        (err as NodeJS.ErrnoException)?.code === 'ERR_FS_EISDIR') {
      return jsonError('Path is a directory', 400);
    }
    throw err;
  }

  await forgetHash(rel);
  await flushHashCache();

  // Tidy up a directory the deletion emptied, but never complain if it is not
  // empty — another album's files may share the parent. Neither root is a
  // directory to tidy: deleting the last top-level file would otherwise
  // remove the tree the gallery's content collection reads from (or the
  // private full-sync store).
  const parent = path.dirname(full);
  if (parent !== root) {
    await fs.rmdir(parent).catch(() => {});
  }
  return new Response(null, { status: 204 });
};
