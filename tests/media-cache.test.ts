import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtemp, rm, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import sharp from 'sharp';
import { getAlbumMediaMeta, type MediaFileStat } from '../src/lib/media-cache';

let albumDir: string;

async function makeTestImage(filename: string, width = 64, height = 40): Promise<MediaFileStat> {
  const filePath = join(albumDir, filename);
  await sharp({
    create: { width, height, channels: 3, background: { r: 200, g: 100, b: 50 } }
  }).jpeg().toFile(filePath);
  const { statSync } = await import('node:fs');
  const stat = statSync(filePath);
  return { filename, size: stat.size, mtimeMs: stat.mtimeMs, isVideo: false };
}

beforeEach(async () => {
  albumDir = await mkdtemp(join(tmpdir(), 'media-cache-test-'));
});

afterEach(async () => {
  await rm(albumDir, { recursive: true, force: true });
});

describe('media cache', () => {
  it('computes dimensions, blur preview and writes the cache file', async () => {
    const stat = await makeTestImage('photo.jpg');
    const meta = await getAlbumMediaMeta(albumDir, [stat]);

    const entry = meta.get('photo.jpg');
    expect(entry?.width).toBe(64);
    expect(entry?.height).toBe(40);
    expect(entry?.blur).toMatch(/^data:image\/jpeg;base64,/);

    const cacheRaw = await readFile(join(albumDir, '.meta', 'index.json'), 'utf-8');
    expect(JSON.parse(cacheRaw).entries['photo.jpg'].width).toBe(64);
  });

  it('reuses cached entries when size+mtime are unchanged (no recompute)', async () => {
    const stat = await makeTestImage('photo.jpg');
    await getAlbumMediaMeta(albumDir, [stat]);

    // Poison the cache — if the second call recomputed, this would be overwritten
    const cacheFile = join(albumDir, '.meta', 'index.json');
    const cache = JSON.parse(await readFile(cacheFile, 'utf-8'));
    cache.entries['photo.jpg'].width = 999;
    await writeFile(cacheFile, JSON.stringify(cache));

    const meta = await getAlbumMediaMeta(albumDir, [stat]);
    expect(meta.get('photo.jpg')?.width).toBe(999);
  });

  it('recomputes when the file changes (different mtime/size)', async () => {
    const stat = await makeTestImage('photo.jpg');
    await getAlbumMediaMeta(albumDir, [stat]);

    // Replace with a different-sized image under the same name
    const newStat = await makeTestImage('photo.jpg', 32, 32);
    const meta = await getAlbumMediaMeta(albumDir, [newStat]);
    expect(meta.get('photo.jpg')?.width).toBe(32);
    expect(meta.get('photo.jpg')?.height).toBe(32);
  });

  it('prunes entries for deleted files', async () => {
    const a = await makeTestImage('a.jpg');
    const b = await makeTestImage('b.jpg');
    await getAlbumMediaMeta(albumDir, [a, b]);

    await getAlbumMediaMeta(albumDir, [a]); // b.jpg no longer listed

    const cache = JSON.parse(await readFile(join(albumDir, '.meta', 'index.json'), 'utf-8'));
    expect(Object.keys(cache.entries)).toEqual(['a.jpg']);
  });

  it('stores videos as plain stats without image processing', async () => {
    const videoStat: MediaFileStat = { filename: 'clip.mp4', size: 123, mtimeMs: 456, isVideo: true };
    const meta = await getAlbumMediaMeta(albumDir, [videoStat]);
    const entry = meta.get('clip.mp4');
    expect(entry?.isVideo).toBe(true);
    expect(entry?.width).toBeUndefined();
    expect(entry?.blur).toBeUndefined();
  });
});
