import type { APIRoute } from 'astro';
import fs from 'fs/promises';
import path from 'path';
import { getAlbumByPath, getPhotosForAlbum } from '../../lib/albums';
import { resolveAlbumAccess, getAccessCookieValue, getClientIp } from '../../lib/access';
import { validateProofingPayload, submissionFilename, type ProofingSubmission } from '../../lib/proofing';

export const prerender = false;

// Dedicated rate limit for submissions: 5 per 15 minutes per client IP
// (separate from the password limiter so the two can't starve each other)
const SUBMIT_LIMIT = 5;
const SUBMIT_WINDOW_MS = 15 * 60 * 1000;
const submissions = new Map<string, { count: number; windowStart: number }>();

function isSubmitLimited(ip: string): boolean {
  const now = Date.now();
  const entry = submissions.get(ip);
  if (!entry || now - entry.windowStart > SUBMIT_WINDOW_MS) {
    submissions.set(ip, { count: 1, windowStart: now });
    return false;
  }
  entry.count++;
  return entry.count > SUBMIT_LIMIT;
}

const MAX_BODY_BYTES = 256 * 1024;

function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' }
  });
}

export const POST: APIRoute = async ({ request, cookies, clientAddress }) => {
  try {
    // Cap the payload before parsing
    const raw = await request.text();
    if (raw.length > MAX_BODY_BYTES) {
      return jsonError('Payload too large', 413);
    }
    let body: unknown;
    try {
      body = JSON.parse(raw);
    } catch {
      return jsonError('Invalid JSON', 400);
    }

    const albumPath = (body as { albumPath?: unknown })?.albumPath;
    if (typeof albumPath !== 'string' || !albumPath) {
      return jsonError('Album path is required', 400);
    }
    if (albumPath.includes('..') || albumPath.startsWith('/') || albumPath.includes('\0')) {
      return jsonError('Invalid path', 400);
    }

    const album = await getAlbumByPath(albumPath);
    if (!album) {
      return jsonError('Album not found', 404);
    }
    if (!album.data.proofing) {
      return jsonError('Proofing is not enabled for this album', 403);
    }

    // Same access rules as viewing the album
    const access = await resolveAlbumAccess(
      albumPath,
      getAccessCookieValue(cookies),
      request.headers.get('X-Album-Token')
    );
    if (!access.hasAccess) {
      return jsonError('Unauthorized', 401);
    }

    // Rate limit AFTER auth so an attacker can't exhaust a shared bucket
    const ip = getClientIp(clientAddress, request.headers.get('x-forwarded-for'));
    if (isSubmitLimited(ip)) {
      return jsonError('Too many submissions — please try again later', 429);
    }

    // Validate against the album's real photo list
    const photos = await getPhotosForAlbum(albumPath);
    const knownFilenames = photos.filter(p => !p.isVideo).map(p => p.filename);
    const result = validateProofingPayload(body, knownFilenames);
    if (!result.ok) {
      return jsonError(result.error, 400);
    }

    const submittedAt = new Date();
    const submission: ProofingSubmission = {
      submittedAt: submittedAt.toISOString(),
      name: result.name,
      selections: result.selections
    };

    const proofingDir = path.join(
      process.cwd(), 'src/content/albums', albumPath, '.meta', 'proofing'
    );
    await fs.mkdir(proofingDir, { recursive: true });
    const filePath = path.join(proofingDir, submissionFilename(submittedAt, result.name));
    await fs.writeFile(filePath, JSON.stringify(submission, null, 2));

    return new Response(JSON.stringify({ success: true, count: result.selections.length }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    });
  } catch (error) {
    console.error('[proofing] Error storing submission:', error);
    return jsonError('Failed to store submission', 500);
  }
};
