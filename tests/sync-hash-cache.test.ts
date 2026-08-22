import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtemp, rm, writeFile, utimes, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  cachedHash,
  flushHashCache,
  forgetHash,
  rememberHash,
  useCacheFileForTests,
} from '../src/pages/api/sync/_hash-cache';
import { blake3HexOf } from '../src/pages/api/sync/_shared';

/**
 * The manifest is the whole cost of a sync — hashing 1.1 GB in JavaScript took
 * 40 s on every push. Caching it by (size, mtime) makes that a directory walk,
 * but only if the cache is never wrong in the dangerous direction: a stale hash
 * means the client is told the server already has a photo it does not, and the
 * photo silently never arrives.
 */
describe('sync hash cache', () => {
  let dir: string;
  let file: string;

  const hashOf = (text: string) => blake3HexOf(new TextEncoder().encode(text));

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'gpp-hash-'));
    file = join(dir, 'photo.jpg');
    useCacheFileForTests(join(dir, '.sync-hashes.json'));
    await writeFile(file, 'first');
  });

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  it('returns the real hash the first time', async () => {
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(hashOf('first'));
  });

  it('serves the same answer from cache when nothing changed', async () => {
    const once = await cachedHash(file, 'a/photo.jpg');
    // Corrupt the file's bytes without touching size or mtime is not possible
    // through the public API, so assert the cheaper property: a second call
    // agrees, and the entry is on disk to be reused by the next process.
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(once);
    await flushHashCache();
    const saved = JSON.parse(await readFile(join(dir, '.sync-hashes.json'), 'utf8'));
    expect(saved.entries['a/photo.jpg'].hash).toBe(once);
  });

  it('re-hashes when the content changes', async () => {
    await cachedHash(file, 'a/photo.jpg');
    await writeFile(file, 'second changed');
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(hashOf('second changed'));
  });

  it('re-hashes when the size is the same but the mtime moved', async () => {
    // Same length, different bytes: only the timestamp betrays the edit, which
    // is exactly the case a size-only check would get wrong.
    await cachedHash(file, 'a/photo.jpg');
    await writeFile(file, 'FIRST');
    const later = new Date(Date.now() + 5000);
    await utimes(file, later, later);
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(hashOf('FIRST'));
  });

  it('takes a hash the uploader already computed', async () => {
    await rememberHash(file, 'a/photo.jpg', hashOf('first'));
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(hashOf('first'));
  });

  it('forgets a deleted file', async () => {
    await cachedHash(file, 'a/photo.jpg');
    await forgetHash('a/photo.jpg');
    await flushHashCache();
    const saved = JSON.parse(await readFile(join(dir, '.sync-hashes.json'), 'utf8'));
    expect(saved.entries['a/photo.jpg']).toBeUndefined();
  });

  it('survives a corrupt cache file by starting cold', async () => {
    await writeFile(join(dir, '.sync-hashes.json'), '{ not json');
    expect(await cachedHash(file, 'a/photo.jpg')).toBe(hashOf('first'));
  });
});
