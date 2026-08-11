import { describe, it, expect, beforeEach } from 'vitest';
import {
  _resetSecretForTests,
  generateShareToken,
  getClientIp,
  isAlbumLocked,
  parseAccessCookie,
  resolveChainAccess,
  safeCompare,
  serializeAccessCookie,
  type AlbumLike
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

  it('ignores forged (unsigned) cookies', () => {
    const a = album('2025/priv', { password: 'p' });
    const forged = JSON.stringify([a.data.token]);
    expect(resolveChainAccess([a], forged).hasAccess).toBe(false);
  });
});

describe('getClientIp', () => {
  it('uses the direct address when it is a real client', () => {
    expect(getClientIp('203.0.113.7', '198.51.100.1')).toBe('203.0.113.7');
  });
  it('uses the first X-Forwarded-For hop behind a local proxy', () => {
    expect(getClientIp('127.0.0.1', '198.51.100.1, 10.0.0.1')).toBe('198.51.100.1');
    expect(getClientIp('::1', '198.51.100.1')).toBe('198.51.100.1');
    expect(getClientIp('::ffff:127.0.0.1', '198.51.100.1')).toBe('198.51.100.1');
  });
  it('falls back to "unknown" when nothing is available', () => {
    expect(getClientIp(undefined, null)).toBe('unknown');
    expect(getClientIp('', '')).toBe('unknown');
  });
});
