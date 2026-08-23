/**
 * The share URL builder, extracted from PhotoGrid's client script. The trap
 * it guards: the share token is introduced by '?' when it is the only
 * parameter and by '&' when a photo deep link comes first — the wrong one
 * hands a client a link that opens the password form instead of the photo.
 */
import { describe, it, expect } from 'vitest';
import { buildPhotoShareUrl, type ShareUrlConfig } from '../src/lib/share-url';

const publicAlbum: ShareUrlConfig = {
  siteUrl: 'https://example.com',
  albumPath: '2025/weddings/ana-ivan',
  isProtected: false,
  albumToken: '',
};

const protectedAlbum: ShareUrlConfig = {
  ...publicAlbum,
  isProtected: true,
  albumToken: 'sekrit-token',
};

describe('buildPhotoShareUrl', () => {
  it('is the bare album URL for a public album with no photo', () => {
    expect(buildPhotoShareUrl(publicAlbum, ''))
      .toBe('https://example.com/photos/2025/weddings/ana-ivan');
  });

  it('adds ?photo= for a deep link, URL-encoding the filename', () => {
    expect(buildPhotoShareUrl(publicAlbum, 'DSC 001+.jpg'))
      .toBe('https://example.com/photos/2025/weddings/ana-ivan?photo=DSC%20001%2B.jpg');
  });

  it('introduces the token with ? when it is the only parameter', () => {
    expect(buildPhotoShareUrl(protectedAlbum, ''))
      .toBe('https://example.com/photos/2025/weddings/ana-ivan?token=sekrit-token');
  });

  it('introduces the token with & after a photo deep link', () => {
    expect(buildPhotoShareUrl(protectedAlbum, 'one.jpg'))
      .toBe('https://example.com/photos/2025/weddings/ana-ivan?photo=one.jpg&token=sekrit-token');
  });

  it('never appends a token for a protected album that has none', () => {
    // Password-only albums have no shareToken; the link must not leak an
    // empty token parameter that would look like one.
    const passwordOnly = { ...publicAlbum, isProtected: true };
    expect(buildPhotoShareUrl(passwordOnly, 'one.jpg'))
      .toBe('https://example.com/photos/2025/weddings/ana-ivan?photo=one.jpg');
  });
});
