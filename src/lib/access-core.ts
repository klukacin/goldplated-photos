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
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

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
    return cachedSecret;
  }

  if (fromEnv && fromEnv.length < 16 && !warnedAboutSecret) {
    // Do not fall through in silence: the operator set a secret and it is not
    // being used, which otherwise only shows up as sessions that mysteriously
    // reset when the fallback file rotates.
    warnedAboutSecret = true;
    console.warn(
      '[access] ACCESS_SECRET is set but shorter than 16 characters — it was IGNORED. ' +
      'A generated secret is used instead. Set ACCESS_SECRET to at least 16 characters in .env.'
    );
  }

  // Fallback: persist a generated secret to .access-secret so sessions
  // survive restarts and are shared across PM2 cluster workers. Only if the
  // file cannot be written do we degrade to an ephemeral per-process secret.
  const secretFile = join(process.cwd(), '.access-secret');
  try {
    cachedSecret = Buffer.from(readFileSync(secretFile, 'utf-8').trim(), 'base64url');
    if (cachedSecret.length >= 16) return cachedSecret;
  } catch { /* not there yet */ }

  const generated = randomBytes(32);
  try {
    // 'wx' fails when the file appeared meanwhile (concurrent worker) — re-read
    writeFileSync(secretFile, generated.toString('base64url'), { flag: 'wx', mode: 0o600 });
    cachedSecret = generated;
    if (!warnedAboutSecret) {
      warnedAboutSecret = true;
      console.warn(
        '[access] ACCESS_SECRET is not set — generated one and stored it in .access-secret. ' +
        'Set ACCESS_SECRET in .env to manage it explicitly.'
      );
    }
  } catch {
    try {
      cachedSecret = Buffer.from(readFileSync(secretFile, 'utf-8').trim(), 'base64url');
      if (cachedSecret.length >= 16) return cachedSecret;
    } catch { /* unreadable too */ }
    cachedSecret = generated;
    if (!warnedAboutSecret) {
      warnedAboutSecret = true;
      console.warn(
        '[access] ACCESS_SECRET is not set and .access-secret could not be written. ' +
        'Using an ephemeral secret: visitors must re-enter passwords after every restart. ' +
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
// Media path validation
// ---------------------------------------------------------------------------

/**
 * Validate a client-supplied path into the albums tree — the one rule set for
 * every media route (`/albums/*`, thumbnail, exif, video-info, watermark).
 *
 * Rejected:
 * - `..` anywhere (path traversal)
 * - a leading `/` (absolute paths)
 * - NUL bytes
 * - backslashes — a separator on Windows, where `album\photo.jpg` would both
 *   escape `path.join` guards and make `resolveFileAccess` see a root-level
 *   file (dir `""`), skipping the album access check entirely
 * - empty segments and any segment starting with a dot (`.meta` cache,
 *   dotfiles)
 * - markdown files (`index.md` holds album passwords) — matched
 *   case-insensitively, since the file system serving them may not be
 *   case-sensitive
 *
 * Routes with extra rules of their own (disabled formats, feature flags) apply
 * those on top of this.
 */
export function isSafeMediaPath(raw: unknown): raw is string {
  if (typeof raw !== 'string' || raw === '') return false;
  if (raw.includes('\0') || raw.includes('\\')) return false;
  if (raw.startsWith('/') || raw.includes('..')) return false;
  if (raw.toLowerCase().endsWith('.md')) return false;
  for (const segment of raw.split('/')) {
    if (segment === '' || segment.startsWith('.')) return false;
  }
  return true;
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

  // Walk bottom-up and stop at the nearest album that settles the question:
  // an unlocked one (cookie) or a matching share token grants, a locked one
  // blocks. Both grants are tested at the same level so they agree — an
  // ancestor's share link must no more open a separately locked child than
  // an ancestor's cookie does, or handing a client the collection link hands
  // them every album inside it that was deliberately locked on its own.
  for (const entry of chain) {
    if (unlocked.includes(entry.data.token)) {
      return { ...GRANTED, isProtected };
    }

    const shareToken = entry.data.shareToken;
    if (providedToken && shareToken && safeCompare(providedToken, shareToken)) {
      const id = entry.data.token;
      const grantedTokens = unlocked.includes(id) ? null : [...unlocked, id];
      return { hasAccess: true, isProtected, blockingAlbum: null, grantedTokens };
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
// Listing visibility
// ---------------------------------------------------------------------------

/** Minimal album shape for deciding how a listing should present an album. */
export interface ListingAlbum {
  password?: string;
  shareToken?: string;
  hidden?: boolean;
}

export interface ChainVisibility {
  /** The album or one of its ancestors is locked. */
  locked: boolean;
  /** The album or one of its ancestors is hidden from listings. */
  hidden: boolean;
}

/**
 * Apply the same inheritance rule as `resolveChainAccess`, but for pages that
 * render a listing rather than answering a request: tag pages, public search.
 *
 * Those pages know nothing about the visitor — a tag page is prerendered — so
 * they can only ask what the tree says. Checking an album's own frontmatter is
 * not enough: an album sitting inside a locked collection is protected content
 * too, and printing its title and cover filename on a public page hands out
 * what the lock on the collection exists to withhold.
 *
 * `lookup` resolves an album path to its frontmatter, or undefined when that
 * path is not an album (an intermediate folder with no index.md).
 */
export function resolveChainVisibility(
  albumPath: string,
  lookup: (path: string) => ListingAlbum | undefined
): ChainVisibility {
  const parts = albumPath.split('/');
  const result: ChainVisibility = { locked: false, hidden: false };

  for (let i = parts.length; i > 0; i--) {
    const entry = lookup(parts.slice(0, i).join('/'));
    if (!entry) continue;
    if (entry.password || entry.shareToken) result.locked = true;
    if (entry.hidden) result.hidden = true;
  }
  return result;
}

// ---------------------------------------------------------------------------
// Redirect targets
// ---------------------------------------------------------------------------

/**
 * Reduce a caller-supplied redirect target to somewhere on this site, falling
 * back to `fallback` when it is anything else.
 *
 * The unlock form carries where to go back to, and a form can be posted from
 * any page on the internet — so left alone this hands an attacker a redirect
 * that departs from the photographer's own domain. That is what makes the
 * phishing link convincing: the visitor sees the gallery they know in the
 * address bar before they are sent somewhere else.
 *
 * A path is kept only if it starts with a single `/`. `//evil.example` is
 * protocol-relative and leaves the site despite looking local, and a backslash
 * takes its place in enough browsers to be worth refusing too.
 */
export function safeReturnUrl(raw: unknown, fallback: string): string {
  if (typeof raw !== 'string' || raw === '') return fallback;
  if (!raw.startsWith('/')) return fallback;
  if (raw.startsWith('//') || raw.startsWith('/\\')) return fallback;
  // A control character can truncate or split the header a redirect is
  // written into.
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i);
    if (code <= 0x1f || code === 0x7f) return fallback;
  }
  return raw;
}

// ---------------------------------------------------------------------------
// Client IP (rate limiting behind a reverse proxy)
// ---------------------------------------------------------------------------

const LOOPBACK = new Set(['127.0.0.1', '::1']);

/**
 * Determine the real client IP. Behind the local reverse proxy the direct
 * peer address is loopback, so fall back to the LAST X-Forwarded-For hop —
 * that's the one appended by our own proxy. Earlier hops are client-supplied
 * and trivially spoofable (an attacker could rotate them to reset rate
 * limits or frame another IP).
 */
export function getClientIp(
  directAddress: string | undefined | null,
  forwardedFor: string | undefined | null
): string {
  const direct = (directAddress || '').trim();
  const isLoopback =
    LOOPBACK.has(direct) || direct.startsWith('::ffff:127.') || direct === '';
  if (isLoopback && forwardedFor) {
    const hops = forwardedFor.split(',').map(h => h.trim()).filter(Boolean);
    const lastHop = hops[hops.length - 1];
    if (lastHop) return lastHop;
  }
  return direct || 'unknown';
}
