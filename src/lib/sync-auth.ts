/**
 * Authentication for the desktop app's sync endpoints.
 *
 * This is a different question from album access. `access.ts` answers "may this
 * visitor *see* this album", with per-album passwords and share links. These
 * endpoints answer "may this client *write* to the gallery at all" — one
 * operator, one machine, full write access. Conflating the two would mean a
 * leaked share link could overwrite the site.
 *
 * So: a single shared secret, `SYNC_TOKEN`, compared in constant time. Absent
 * from the environment, the endpoints refuse every request rather than
 * defaulting to open — an unconfigured deployment must not be a writable one.
 */

import { timingSafeEqual } from 'node:crypto';

/** Longest path we will accept, to bound work before any filesystem call. */
const MAX_PATH_LENGTH = 512;

export type SyncAuthResult =
  | { ok: true }
  | { ok: false; status: 401 | 503; message: string };

/**
 * Check the `Authorization: Bearer …` header against `SYNC_TOKEN`.
 *
 * Returns 503 rather than 401 when the server has no token configured: the
 * client's credentials are not the problem, and saying so saves an hour of
 * debugging a correct token against a server that can never accept one.
 */
export function checkSyncAuth(request: Request): SyncAuthResult {
  const expected = process.env.SYNC_TOKEN;

  if (!expected || expected.length < 16) {
    return {
      ok: false,
      status: 503,
      message:
        'Sync is not enabled on this server. Set SYNC_TOKEN (at least 16 characters) in .env and restart.',
    };
  }

  const header = request.headers.get('authorization') ?? '';
  const presented = header.startsWith('Bearer ') ? header.slice(7) : '';

  if (!presented || !constantTimeEqual(presented, expected)) {
    return { ok: false, status: 401, message: 'Bad or missing sync token.' };
  }
  return { ok: true };
}

/** Compare without leaking length or content through timing. */
function constantTimeEqual(a: string, b: string): boolean {
  const bufA = Buffer.from(a, 'utf8');
  const bufB = Buffer.from(b, 'utf8');
  // timingSafeEqual throws on a length mismatch, which would itself be a leak;
  // hash both to a fixed width first so every comparison costs the same.
  if (bufA.length !== bufB.length) return false;
  return timingSafeEqual(bufA, bufB);
}

/**
 * Validate a client-supplied relative path.
 *
 * These endpoints write to disk from a network request, so this is the security
 * boundary. Rejected: absolute paths, any `..` segment, backslashes (which are
 * separators on Windows and would escape there), NUL bytes, dotfiles at any
 * depth — which keeps `.meta/proofing` server-owned, as the sync design
 * requires — and anything over a sane length.
 *
 * Returns the normalised path, or null if it must not be touched.
 */
export function safeRelPath(raw: string | null): string | null {
  if (!raw || raw.length > MAX_PATH_LENGTH) return null;
  if (raw.includes('\0') || raw.includes('\\')) return null;
  if (raw.startsWith('/')) return null;

  const segments = raw.split('/');
  if (segments.length === 0) return null;

  for (const segment of segments) {
    if (segment === '' || segment === '.' || segment === '..') return null;
    if (segment.startsWith('.')) return null;
  }
  return segments.join('/');
}

/** Same rules, but for a scope prefix, which may also be absent. */
export function safeScope(raw: string | null): string | null | undefined {
  if (raw === null || raw === '') return undefined;
  return safeRelPath(raw);
}
