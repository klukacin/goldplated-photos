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

/**
 * The desktop app's full-scope sync namespace. Everything under it — camera
 * originals (RAW included) and per-album metadata — is device-to-device
 * payload, NOT gallery content: it must never land under `src/content/albums`
 * where the site would build and serve it. The endpoints route it to a
 * private root instead (`.sync-full/`, outside the content tree).
 */
export const FULL_SYNC_PREFIX = '__gpp_full__';

/**
 * Decide which tree a **validated** rel path belongs to.
 *
 * Pure routing, factored out so the security-relevant part is unit-testable:
 * call it only with the output of {@link safeRelPath} (or a validated scope).
 * A path beginning with the reserved prefix maps into the private full-sync
 * tree, with the prefix stripped; anything else is ordinary gallery content.
 * The bare prefix itself names no file and maps to nothing.
 *
 * The prefix segments are ordinary names (no dots), so `safeRelPath` has
 * already refused every escape shape — `__gpp_full__/../x`, backslashes, NUL,
 * dotfiles — before this ever runs; only a *leading* prefix is reserved, so an
 * album someone really named `2026/__gpp_full__` stays ordinary content.
 */
export function splitSyncPath(
  rel: string
): { tree: 'content' | 'full'; rest: string } | null {
  if (rel === FULL_SYNC_PREFIX) return null;
  if (rel.startsWith(`${FULL_SYNC_PREFIX}/`)) {
    const rest = rel.slice(FULL_SYNC_PREFIX.length + 1);
    if (rest === '') return null;
    return { tree: 'full', rest };
  }
  return { tree: 'content', rest: rel };
}
