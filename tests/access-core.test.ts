import { describe, it, expect, beforeEach, vi } from 'vitest';
import { existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import {
  _resetSecretForTests,
  generateShareToken,
  getClientIp,
  isAlbumLocked,
  isSafeMediaPath,
  parseAccessCookie,
  resolveChainAccess,
  resolveChainVisibility,
  safeCompare,
  safeReturnUrl,
  serializeAccessCookie,
  type AlbumLike,
  type ListingAlbum
} from '../src/lib/access-core';

function album(id: string, data: Partial<AlbumLike['data']> = {}): AlbumLike {
  return {
    id,
    data: {
      title: data.title ?? id,
      token: data.token ?? `tok-${id}`,
      password: data.password,
      shareToken: data.shareToken
    }
  };
}

beforeEach(() => {
  process.env.ACCESS_SECRET = 'test-secret-at-least-16-chars';
  _resetSecretForTests();
});

describe('safeCompare', () => {
  it('matches equal strings', () => {
    expect(safeCompare('abc', 'abc')).toBe(true);
  });
  it('rejects different strings and lengths', () => {
    expect(safeCompare('abc', 'abd')).toBe(false);
    expect(safeCompare('abc', 'abcd')).toBe(false);
    expect(safeCompare('', 'a')).toBe(false);
  });
});

describe('secret handling', () => {
  it('warns when a configured ACCESS_SECRET is too short to be used', () => {
    const secretFile = join(process.cwd(), '.access-secret');
    const hadSecretFile = existsSync(secretFile);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      process.env.ACCESS_SECRET = 'too-short';
      _resetSecretForTests();
      // Force secret resolution
      serializeAccessCookie(['x']);
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining('shorter than 16 characters')
      );
    } finally {
      warn.mockRestore();
      // Do not leave behind a fallback secret file this test generated
      if (!hadSecretFile) rmSync(secretFile, { force: true });
      process.env.ACCESS_SECRET = 'test-secret-at-least-16-chars';
      _resetSecretForTests();
    }
  });
});

describe('isSafeMediaPath', () => {
  it('accepts ordinary photo and video paths', () => {
    for (const path of [
      'photo.jpg',
      '2025/wedding/ana-ivan/IMG_0001.jpg',
      'friends/trip/video.mp4',
      'a-b_c/photo (1).jpeg'
    ]) {
      expect(isSafeMediaPath(path), path).toBe(true);
    }
  });

  it('rejects traversal, absolute paths and NUL bytes', () => {
    for (const path of [
      '../secret.jpg',
      'a/../../etc/passwd',
      '/etc/passwd',
      'a/b..jpg/..',
      'a\0b.jpg'
    ]) {
      expect(isSafeMediaPath(path), path).toBe(false);
    }
  });

  it('rejects backslashes, which are separators on Windows', () => {
    // `album\photo.jpg` passes a forward-slash-only guard whole, and
    // resolveFileAccess would see it as a root-level file (dir "") — skipping
    // the album's access check — while path.join on Windows still resolves it
    // into the album. Same rule as sync-auth's safeRelPath.
    for (const path of [
      'album\\photo.jpg',
      '..\\..\\x.jpg',
      'a/b\\..\\c.jpg',
      '\\etc\\passwd',
      'locked-album\\photo.jpg'
    ]) {
      expect(isSafeMediaPath(path), path).toBe(false);
    }
  });

  it('rejects dot segments and empty segments', () => {
    for (const path of [
      '.meta/thumbnails/small/x.jpg',
      'album/.meta/proofing/sub.json',
      'album/.hidden.jpg',
      'album//photo.jpg',
      '.env'
    ]) {
      expect(isSafeMediaPath(path), path).toBe(false);
    }
  });

  it('rejects markdown (album passwords live in index.md), case-insensitively', () => {
    for (const path of ['album/index.md', 'album/INDEX.MD', 'index.md']) {
      expect(isSafeMediaPath(path), path).toBe(false);
    }
  });

  it('rejects non-strings and the empty string', () => {
    expect(isSafeMediaPath('')).toBe(false);
    expect(isSafeMediaPath(null)).toBe(false);
    expect(isSafeMediaPath(undefined)).toBe(false);
    expect(isSafeMediaPath(42)).toBe(false);
  });
});

describe('signed access cookie', () => {
  it('round-trips a token list', () => {
    const value = serializeAccessCookie(['a', 'b']);
    expect(parseAccessCookie(value)).toEqual(['a', 'b']);
  });

  it('rejects a missing or malformed cookie', () => {
    expect(parseAccessCookie(undefined)).toEqual([]);
    expect(parseAccessCookie('')).toEqual([]);
    expect(parseAccessCookie('no-dot-here')).toEqual([]);
    expect(parseAccessCookie('.only-signature')).toEqual([]);
  });

  it('rejects a tampered payload (forged token list)', () => {
    const value = serializeAccessCookie(['a']);
    const [, signature] = [value.slice(0, value.lastIndexOf('.')), value.slice(value.lastIndexOf('.') + 1)];
    const forgedPayload = Buffer.from(JSON.stringify(['a', 'stolen-token'])).toString('base64url');
    expect(parseAccessCookie(`${forgedPayload}.${signature}`)).toEqual([]);
  });

  it('rejects a cookie signed with a different secret', () => {
    const value = serializeAccessCookie(['a']);
    process.env.ACCESS_SECRET = 'another-secret-16-chars-long!!';
    _resetSecretForTests();
    expect(parseAccessCookie(value)).toEqual([]);
  });

  it('rejects legacy unsigned JSON cookies', () => {
    expect(parseAccessCookie(JSON.stringify(['a', 'b']))).toEqual([]);
  });
});

describe('isAlbumLocked', () => {
  it('is locked with a password and/or share token, open otherwise', () => {
    expect(isAlbumLocked(album('x'))).toBe(false);
    expect(isAlbumLocked(album('x', { password: 'p' }))).toBe(true);
    expect(isAlbumLocked(album('x', { shareToken: 's' }))).toBe(true);
  });
});

describe('resolveChainAccess', () => {
  const cookieFor = (...tokens: string[]) => serializeAccessCookie(tokens);

  it('grants access to fully public chains', () => {
    const result = resolveChainAccess([album('2025/pub'), album('2025')], undefined);
    expect(result).toMatchObject({ hasAccess: true, isProtected: false, blockingAlbum: null });
  });

  it('blocks a password-protected album without a cookie', () => {
    const result = resolveChainAccess([album('2025/priv', { password: 'p' })], undefined);
    expect(result.hasAccess).toBe(false);
    expect(result.blockingAlbum).toMatchObject({ path: '2025/priv', hasPassword: true });
  });

  it('grants access with the album token in a valid cookie', () => {
    const a = album('2025/priv', { password: 'p' });
    const result = resolveChainAccess([a], cookieFor(a.data.token));
    expect(result.hasAccess).toBe(true);
  });

  it('inherits an ancestor unlock down to children', () => {
    const parent = album('2025', { password: 'p' });
    const child = album('2025/child');
    const result = resolveChainAccess([child, parent], cookieFor(parent.data.token));
    expect(result.hasAccess).toBe(true);
  });

  it('blocks a child when a locked ancestor is not unlocked', () => {
    const parent = album('2025', { password: 'p' });
    const child = album('2025/child');
    const result = resolveChainAccess([child, parent], undefined);
    expect(result.hasAccess).toBe(false);
    expect(result.blockingAlbum?.path).toBe('2025');
  });

  it('grants access via a matching share token and reports the cookie update', () => {
    const shareToken = generateShareToken();
    const a = album('2025/client', { password: 'p', shareToken });
    const result = resolveChainAccess([a], undefined, shareToken);
    expect(result.hasAccess).toBe(true);
    expect(result.grantedTokens).toEqual([a.data.token]);
  });

  it('does not report a cookie update when the token is already unlocked', () => {
    const shareToken = generateShareToken();
    const a = album('2025/client', { password: 'p', shareToken });
    const result = resolveChainAccess([a], cookieFor(a.data.token), shareToken);
    expect(result.hasAccess).toBe(true);
    expect(result.grantedTokens).toBeNull();
  });

  it('an ancestor share token grants descendants', () => {
    const shareToken = generateShareToken();
    const parent = album('2025', { password: 'p', shareToken });
    const child = album('2025/child');
    const result = resolveChainAccess([child, parent], undefined, shareToken);
    expect(result.hasAccess).toBe(true);
  });

  it('rejects a wrong share token (and internal tokens are never accepted)', () => {
    const a = album('2025/client', { password: 'p', shareToken: generateShareToken() });
    expect(resolveChainAccess([a], undefined, 'wrong').hasAccess).toBe(false);
    // The internal cookie id must not work as a URL token
    expect(resolveChainAccess([a], undefined, a.data.token).hasAccess).toBe(false);
  });

  it('rejects tokens on albums without a shareToken (opt-in only)', () => {
    const a = album('2025/priv', { password: 'p' });
    const result = resolveChainAccess([a], undefined, 'anything');
    expect(result.hasAccess).toBe(false);
  });

  it('link-only albums (shareToken, no password) block direct access', () => {
    const shareToken = generateShareToken();
    const a = album('2025/link-only', { shareToken });
    const blocked = resolveChainAccess([a], undefined);
    expect(blocked.hasAccess).toBe(false);
    expect(blocked.blockingAlbum).toMatchObject({ path: '2025/link-only', hasPassword: false });
    expect(resolveChainAccess([a], undefined, shareToken).hasAccess).toBe(true);
  });

  it('a child with its own lock stays locked even when the parent is unlocked', () => {
    const parent = album('2025', { password: 'p' });
    const linkOnlyChild = album('2025/secret', { shareToken: generateShareToken() });
    const passwordChild = album('2025/vip', { password: 'q' });
    const parentCookie = cookieFor(parent.data.token);
    expect(resolveChainAccess([linkOnlyChild, parent], parentCookie).hasAccess).toBe(false);
    expect(resolveChainAccess([passwordChild, parent], parentCookie).hasAccess).toBe(false);
  });

  it("a child with its own lock stays locked against the parent's share link too", () => {
    // The cookie path stops at the nearest lock (test above). The share-token
    // path must stop at the same place, or handing a client the collection's
    // secret link also hands them every separately-locked album inside it.
    const parentShareToken = generateShareToken();
    const parent = album('2025', { shareToken: parentShareToken });
    const passwordChild = album('2025/vip', { password: 'q' });
    const linkOnlyChild = album('2025/secret', { shareToken: generateShareToken() });

    expect(resolveChainAccess([passwordChild, parent], undefined, parentShareToken).hasAccess).toBe(false);
    expect(resolveChainAccess([linkOnlyChild, parent], undefined, parentShareToken).hasAccess).toBe(false);
  });

  it("a child's own share token still opens it from under a locked parent", () => {
    const childShareToken = generateShareToken();
    const parent = album('2025', { password: 'p' });
    const child = album('2025/secret', { shareToken: childShareToken });
    const result = resolveChainAccess([child, parent], undefined, childShareToken);
    expect(result.hasAccess).toBe(true);
    expect(result.grantedTokens).toEqual([child.data.token]);
  });

  it('ignores forged (unsigned) cookies', () => {
    const a = album('2025/priv', { password: 'p' });
    const forged = JSON.stringify([a.data.token]);
    expect(resolveChainAccess([a], forged).hasAccess).toBe(false);
  });
});

describe('resolveChainVisibility', () => {
  const tree: Record<string, ListingAlbum> = {
    '2026': {},
    '2026/clients': { shareToken: 'secret-link' },
    '2026/clients/ana-ivan': {},
    '2026/drafts': { hidden: true },
    '2026/drafts/maybe': {},
    '2026/public': {},
    '2026/public/street': {},
    '2026/public/vip': { password: 'p' }
  };
  const lookup = (p: string) => tree[p];

  it('reports a fully public chain as neither locked nor hidden', () => {
    expect(resolveChainVisibility('2026/public/street', lookup)).toEqual({ locked: false, hidden: false });
  });

  it("reports an album's own lock", () => {
    expect(resolveChainVisibility('2026/public/vip', lookup).locked).toBe(true);
  });

  it('reports a lock inherited from an ancestor', () => {
    // The pixels of this album are behind the access check either way, but its
    // title and cover filename are not — a public listing must treat it as
    // locked, exactly as it treats the collection above it.
    expect(resolveChainVisibility('2026/clients/ana-ivan', lookup).locked).toBe(true);
  });

  it('reports hidden inherited from an ancestor', () => {
    expect(resolveChainVisibility('2026/drafts/maybe', lookup).hidden).toBe(true);
  });

  it('ignores path segments that are not albums', () => {
    expect(resolveChainVisibility('2026/nothing/here', lookup)).toEqual({ locked: false, hidden: false });
  });
});

describe('getClientIp', () => {
  it('uses the direct address when it is a real client', () => {
    expect(getClientIp('203.0.113.7', '198.51.100.1')).toBe('203.0.113.7');
  });
  it('uses the LAST X-Forwarded-For hop behind a local proxy (appended by our proxy)', () => {
    expect(getClientIp('127.0.0.1', '198.51.100.1')).toBe('198.51.100.1');
    expect(getClientIp('::1', '198.51.100.1')).toBe('198.51.100.1');
    expect(getClientIp('::ffff:127.0.0.1', '198.51.100.1')).toBe('198.51.100.1');
  });
  it('ignores client-spoofed leading X-Forwarded-For entries', () => {
    // The proxy APPENDS the real peer: "spoofed, realIP" — trust the last hop
    expect(getClientIp('127.0.0.1', '1.2.3.4, 198.51.100.1')).toBe('198.51.100.1');
    expect(getClientIp('127.0.0.1', 'a, b, 198.51.100.1')).toBe('198.51.100.1');
  });
  it('falls back to "unknown" when nothing is available', () => {
    expect(getClientIp(undefined, null)).toBe('unknown');
    expect(getClientIp('', '')).toBe('unknown');
  });
});

describe('safeReturnUrl', () => {
  // The unlock form carries where to go back to, and a form can be posted from
  // any page on the internet. Left alone, that is a redirect off the
  // photographer's own domain — which is exactly what makes a phishing link
  // convincing: the visitor sees the gallery they know before they are sent
  // somewhere else.
  const FALLBACK = '/photos/2026/weddings';

  it('keeps an ordinary path on this site', () => {
    expect(safeReturnUrl('/photos/2026/weddings/ana', FALLBACK))
      .toBe('/photos/2026/weddings/ana');
  });

  it('refuses an absolute URL somewhere else', () => {
    expect(safeReturnUrl('https://evil.example/login', FALLBACK)).toBe(FALLBACK);
  });

  it('refuses a protocol-relative URL that only looks local', () => {
    // `//evil.example` starts with a slash and leaves the site anyway.
    expect(safeReturnUrl('//evil.example/login', FALLBACK)).toBe(FALLBACK);
  });

  it('refuses a backslash-escaped host', () => {
    expect(safeReturnUrl('/\\evil.example', FALLBACK)).toBe(FALLBACK);
  });

  it('refuses a control character that could split the header', () => {
    expect(safeReturnUrl('/photos\r\nSet-Cookie: a=b', FALLBACK)).toBe(FALLBACK);
  });

  it('falls back when nothing was supplied', () => {
    expect(safeReturnUrl(undefined, FALLBACK)).toBe(FALLBACK);
    expect(safeReturnUrl('', FALLBACK)).toBe(FALLBACK);
  });
});
