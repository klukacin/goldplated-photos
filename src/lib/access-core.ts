/**
 * Pure access-control logic — no Astro imports, fully unit-testable.
 *
 * Model:
 * - An album is LOCKED when it has a `password` and/or a `shareToken`.
 * - A `shareToken` is a random, admin-generated secret. Presenting it via
 *   `?token=` unlocks the album (and its descendants) without a password.
 *   Setting a shareToken on an album without a password makes the album
 *   reachable only via the secret link.
 * - Unlocked albums are remembered in the `album-access` cookie as a list of
 *   internal album ids (`token` frontmatter field). The cookie value is
 *   HMAC-signed with ACCESS_SECRET so it cannot be forged.
 * - Lock state is inherited: a locked ancestor blocks its descendants until
 *   it is unlocked. An unlocked album grants its descendants.
 */
import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';

export const ACCESS_COOKIE = 'album-access';
export const ACCESS_COOKIE_MAX_AGE = 60 * 60 * 24; // 24 hours

/** Minimal album shape the access logic needs (structural, for testability). */
export interface AlbumLike {
  id: string;
  data: {
    title: string;
    token: string;
    password?: string;
    shareToken?: string;
  };
}

// ---------------------------------------------------------------------------
// Secret handling
// ---------------------------------------------------------------------------

let cachedSecret: Buffer | null = null;
let warnedAboutSecret = false;

function getSecret(): Buffer {
  if (cachedSecret) return cachedSecret;

  const fromEnv = typeof process !== 'undefined' ? process.env?.ACCESS_SECRET : undefined;

  if (fromEnv && fromEnv.length >= 16) {
    cachedSecret = Buffer.from(fromEnv, 'utf-8');
  } else {
    // Ephemeral fallback: cookies stay valid only until the server restarts.
    cachedSecret = randomBytes(32);
    if (!warnedAboutSecret) {
      warnedAboutSecret = true;
      console.warn(
        '[access] ACCESS_SECRET is not set (or shorter than 16 chars). ' +
        'Using an ephemeral secret: visitors will need to re-enter passwords after every server restart. ' +
        'Set ACCESS_SECRET in .env for stable sessions.'
      );
    }
  }
  return cachedSecret;
}

/** Test hook: reset the cached secret so tests can vary ACCESS_SECRET. */
export function _resetSecretForTests(): void {
  cachedSecret = null;
  warnedAboutSecret = false;
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/** Constant-time string comparison (length-safe). */
export function safeCompare(a: string, b: string): boolean {
  const bufA = Buffer.from(a);
  const bufB = Buffer.from(b);
  if (bufA.length !== bufB.length) return false;
  return timingSafeEqual(bufA, bufB);
}

function sign(payload: string): string {
  return createHmac('sha256', getSecret()).update(payload).digest('base64url');
}

/** Generate a new random share token (URL-safe, 22 chars). */
export function generateShareToken(): string {
  return randomBytes(16).toString('base64url');
}

// ---------------------------------------------------------------------------
// Signed cookie (list of unlocked album ids)
// ---------------------------------------------------------------------------

/** Serialize a token list into a signed cookie value: `<base64url(json)>.<hmac>`. */
export function serializeAccessCookie(tokens: string[]): string {
  const payload = Buffer.from(JSON.stringify(tokens)).toString('base64url');
  return `${payload}.${sign(payload)}`;
}

/**
 * Parse and verify a signed cookie value. Returns the token list, or [] when
 * the cookie is missing, malformed, or its signature does not verify.
 */
export function parseAccessCookie(raw: string | undefined | null): string[] {
  if (!raw) return [];
  const dot = raw.lastIndexOf('.');
  if (dot <= 0) return [];
  const payload = raw.slice(0, dot);
  const signature = raw.slice(dot + 1);
  if (!safeCompare(signature, sign(payload))) return [];
  try {
    const parsed = JSON.parse(Buffer.from(payload, 'base64url').toString('utf-8'));
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((t): t is string => typeof t === 'string');
  } catch {
    return [];
  }
}

// ---------------------------------------------------------------------------
// Access resolution
// ---------------------------------------------------------------------------

/** An album is locked when it has a password and/or a share token. */
export function isAlbumLocked(album: AlbumLike): boolean {
  return !!album.data.password || !!album.data.shareToken;
}

export interface BlockingAlbum {
  path: string;
  title: string;
  /** true → show the password form; false → link-only album, no form to show */
  hasPassword: boolean;
}

export interface AccessResult {
  hasAccess: boolean;
  /** true when the album or any ancestor is locked */
  isProtected: boolean;
  /** The nearest locked album denying access (null when access is granted). */
  blockingAlbum: BlockingAlbum | null;
  /**
   * When a provided share token granted access that the cookie didn't already
   * cover, this is the new full token list to persist in the cookie.
   * Null when no cookie update is needed.
   */
  grantedTokens: string[] | null;
}

const GRANTED: Omit<AccessResult, 'isProtected'> = {
  hasAccess: true,
  blockingAlbum: null,
  grantedTokens: null
};

function albumId(album: AlbumLike): string {
  return album.id.replace('/index.md', '');
}

/**
 * Resolve access for a bottom-up chain of albums (the album itself first,
 * then its ancestors up to the root).
 */
export function resolveChainAccess(
  chain: AlbumLike[],
  cookieValue: string | undefined | null,
  providedToken?: string | null
): AccessResult {
  const isProtected = chain.some(isAlbumLocked);
  if (!isProtected) {
    return { ...GRANTED, isProtected: false };
  }

  const unlocked = parseAccessCookie(cookieValue);

  // 1. A valid share token for the album or any ancestor grants access.
  if (providedToken) {
    for (const entry of chain) {
      const shareToken = entry.data.shareToken;
      if (shareToken && safeCompare(providedToken, shareToken)) {
        const id = entry.data.token;
        const grantedTokens = unlocked.includes(id) ? null : [...unlocked, id];
        return { hasAccess: true, isProtected, blockingAlbum: null, grantedTokens };
      }
    }
  }

  // 2. Cookie-based: walk bottom-up; the nearest unlocked album grants,
  //    the nearest locked album blocks.
  for (const entry of chain) {
    if (unlocked.includes(entry.data.token)) {
      return { ...GRANTED, isProtected };
    }
    if (isAlbumLocked(entry)) {
      return {
        hasAccess: false,
        isProtected,
        blockingAlbum: {
          path: albumId(entry),
          title: entry.data.title,
          hasPassword: !!entry.data.password
        },
        grantedTokens: null
      };
    }
  }

  // No locked album encountered on the way up (unreachable when isProtected
  // is true, but keep a safe default for genuinely unlocked chains).
  return { ...GRANTED, isProtected };
}

// ---------------------------------------------------------------------------
// Client IP (rate limiting behind a reverse proxy)
// ---------------------------------------------------------------------------

const LOOPBACK = new Set(['127.0.0.1', '::1']);

/**
 * Determine the real client IP. Behind the local reverse proxy the direct
 * peer address is loopback, so fall back to the first X-Forwarded-For hop.
 */
export function getClientIp(
  directAddress: string | undefined | null,
  forwardedFor: string | undefined | null
): string {
  const direct = (directAddress || '').trim();
  const isLoopback =
    LOOPBACK.has(direct) || direct.startsWith('::ffff:127.') || direct === '';
  if (isLoopback && forwardedFor) {
    const firstHop = forwardedFor.split(',')[0].trim();
    if (firstHop) return firstHop;
  }
  return direct || 'unknown';
}
