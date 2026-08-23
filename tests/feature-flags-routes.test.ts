/**
 * A feature flag that only hides UI is not off — the route is still there for
 * anyone who knows the URL. These tests call the real route handlers with the
 * flag disabled in the environment and require a 404, before the handler does
 * any of its normal work.
 *
 * The flags resolve from process.env when src/site-features.mjs loads, so the
 * environment is set in vi.hoisted — which runs before the import graph — and
 * the album/access modules are mocked away (they drag in astro:content, which
 * only exists inside an Astro build).
 */
import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import type { APIContext } from 'astro';
import fs from 'node:fs/promises';
import path from 'node:path';

vi.hoisted(() => {
  process.env.FEATURE_PROOFING = '0';
  process.env.FEATURE_WATERMARK = '0';
  process.env.FEATURE_HEIC = '0';
});

vi.mock('../src/lib/albums', () => ({
  getAlbumByPath: vi.fn(async () => ({
    id: 'demo/album',
    data: { title: 'Demo', proofing: true },
  })),
  getPhotosForAlbum: vi.fn(async () => [
    { filename: 'one.jpg', isVideo: false },
  ]),
  getAncestors: vi.fn(async () => []),
}));

vi.mock('../src/lib/access', () => ({
  resolveAlbumAccess: vi.fn(async () => ({ hasAccess: true, isProtected: false })),
  resolveFileAccess: vi.fn(async () => ({ hasAccess: true, isProtected: false })),
  getAccessCookieValue: vi.fn(() => undefined),
  getClientIp: vi.fn(() => '203.0.113.7'),
}));

import { POST as proofingPOST } from '../src/pages/api/proofing';
import { GET as watermarkGET } from '../src/pages/api/watermark';
import { GET as originalGET } from '../src/pages/albums/[...path]';

const cookies = { get: () => undefined } as unknown as APIContext['cookies'];

describe('FEATURE_PROOFING=0', () => {
  it('/api/proofing answers 404 even for an album with proofing enabled', async () => {
    // The mocked album has proofing: true and access is granted — everything
    // short of the kill-switch says yes. Only the flag can produce the 404.
    const request = new Request('http://localhost/api/proofing', {
      method: 'POST',
      body: JSON.stringify({
        albumPath: 'demo/album',
        selections: [{ filename: 'one.jpg' }],
      }),
    });
    const response = await proofingPOST({
      request,
      cookies,
      clientAddress: '203.0.113.7',
    } as unknown as APIContext);
    expect(response.status).toBe(404);
  });
});

describe('FEATURE_HEIC=0', () => {
  // A real file on disk, granted access — only the flag stands between it and
  // a 200. Discovery already skips HEIC when the flag is off; this pins down
  // that the file also cannot be fetched by whoever remembers the URL.
  const albumDir = path.join(process.cwd(), 'src/content/albums/__flag-route-test__');
  const heicPath = path.join(albumDir, 'photo.heic');

  beforeAll(async () => {
    await fs.mkdir(albumDir, { recursive: true });
    await fs.writeFile(heicPath, Buffer.from('not-really-heic-but-served-verbatim'));
  });

  afterAll(async () => {
    await fs.rm(albumDir, { recursive: true, force: true });
  });

  it('/albums/*.heic answers 404 even though the file exists', async () => {
    const url = new URL('http://localhost/albums/__flag-route-test__/photo.heic');
    const response = await originalGET({
      params: { path: '__flag-route-test__/photo.heic' },
      url,
      cookies,
    } as unknown as APIContext);
    expect(response.status).toBe(404);
  });
});

describe('FEATURE_WATERMARK=0', () => {
  it('/api/watermark answers 404 before touching the file', async () => {
    const url = 'http://localhost/api/watermark?path=demo/album/one.jpg';
    const response = await watermarkGET({
      request: new Request(url),
      cookies,
    } as unknown as APIContext);
    expect(response.status).toBe(404);
  });
});
