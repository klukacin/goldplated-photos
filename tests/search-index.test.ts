/**
 * The search page's photo listing. Measured on a 300-album / 3600-photo
 * library, /photos/search spent ~120ms per query on filesystem work, ~75ms
 * of it stat()ing every photo just to revalidate the media cache — whose
 * values search then trusts anyway. getSearchablePhotos reads the directory
 * (the authority on which files exist) and takes everything else from the
 * cache file as-is, never computing metadata: a search must never be the
 * request that runs Sharp over a whole album.
 */
import { describe, it, expect, beforeEach, afterAll } from 'vitest';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { getSearchablePhotos } from '../src/lib/search-index';

let albumDir: string;

async function writeCache(entries: Record<string, unknown>): Promise<void> {
  await fs.mkdir(path.join(albumDir, '.meta'), { recursive: true });
  await fs.writeFile(
    path.join(albumDir, '.meta', 'index.json'),
    JSON.stringify({ version: 1, entries })
  );
}

beforeEach(async () => {
  albumDir = await fs.mkdtemp(path.join(os.tmpdir(), 'gpp-search-index-'));
  await fs.writeFile(path.join(albumDir, 'one.jpg'), 'x');
  await fs.writeFile(path.join(albumDir, 'clip.mp4'), 'x');
  await fs.writeFile(path.join(albumDir, 'index.md'), '---\ntitle: t\n---\n');
  await fs.writeFile(path.join(albumDir, '.hidden.jpg'), 'x');
});

afterAll(async () => {
  // beforeEach makes a fresh dir per test; each is tiny, remove the last one
  await fs.rm(albumDir, { recursive: true, force: true });
});

describe('getSearchablePhotos', () => {
  it('lists media from the directory even when no cache exists — filename-only', async () => {
    const photos = await getSearchablePhotos(albumDir);
    const names = photos.map(p => p.filename).sort();
    expect(names).toEqual(['clip.mp4', 'one.jpg']);
    const one = photos.find(p => p.filename === 'one.jpg')!;
    expect(one.isVideo).toBe(false);
    expect(one.camera).toBeNull();
    expect(one.exifDate).toBeNull();
    expect(photos.find(p => p.filename === 'clip.mp4')!.isVideo).toBe(true);
  });

  it('skips dotfiles and non-media files', async () => {
    const names = (await getSearchablePhotos(albumDir)).map(p => p.filename);
    expect(names).not.toContain('index.md');
    expect(names).not.toContain('.hidden.jpg');
  });

  it('joins cached metadata onto the directory listing', async () => {
    await writeCache({
      'one.jpg': {
        filename: 'one.jpg', size: 1, mtimeMs: 1, isVideo: false,
        width: 640, height: 480,
        exifDate: '2025-06-14T10:00:00.000Z',
        camera: 'Canon EOS R5', blur: 'data:image/jpeg;base64,xx'
      }
    });
    const one = (await getSearchablePhotos(albumDir)).find(p => p.filename === 'one.jpg')!;
    expect(one.camera).toBe('Canon EOS R5');
    expect(one.exifDate?.toISOString()).toBe('2025-06-14T10:00:00.000Z');
    expect(one.width).toBe(640);
    expect(one.height).toBe(480);
    expect(one.blur).toBe('data:image/jpeg;base64,xx');
  });

  it('the directory is the authority — cache entries for deleted files do not appear', async () => {
    await writeCache({
      'gone.jpg': { filename: 'gone.jpg', size: 1, mtimeMs: 1, isVideo: false, camera: 'Leica' }
    });
    const names = (await getSearchablePhotos(albumDir)).map(p => p.filename);
    expect(names).not.toContain('gone.jpg');
  });

  it('picks up a rewritten cache file on the next call', async () => {
    await writeCache({
      'one.jpg': { filename: 'one.jpg', size: 1, mtimeMs: 1, isVideo: false, camera: 'Nikon Z6' }
    });
    expect((await getSearchablePhotos(albumDir))[0]).toBeTruthy();
    // The in-memory reuse must be validated by the cache file's identity,
    // not assumed forever — a new write must surface new values.
    await new Promise(r => setTimeout(r, 20));
    await writeCache({
      'one.jpg': { filename: 'one.jpg', size: 2, mtimeMs: 2, isVideo: false, camera: 'Fuji X-T5' }
    });
    const one = (await getSearchablePhotos(albumDir)).find(p => p.filename === 'one.jpg')!;
    expect(one.camera).toBe('Fuji X-T5');
  });

  it('returns an empty list for a directory that does not exist', async () => {
    expect(await getSearchablePhotos(path.join(albumDir, 'nope'))).toEqual([]);
  });
});
