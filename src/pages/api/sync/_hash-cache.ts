/**
 * Remembers a file's hash so the server does not re-read the whole gallery on
 * every sync.
 *
 * This is the difference between a sync that costs a directory walk and one
 * that costs the entire library. BLAKE3 in JavaScript runs at about 32 MB/s
 * here while the disk reads at 2.3 GB/s, so hashing — not the network, not the
 * disk — is what a sync of a real shoot waits on: 1.1 GB took 40 seconds, on
 * every push, even when nothing had changed.
 *
 * The key is (size, mtime), the same pair every incremental build tool trusts.
 * It can be fooled by a write that preserves both, which is why it is only ever
 * a cache: a wrong hash makes the client send a file it did not need to, never
 * skip one it did.
 */
import fs from 'node:fs/promises';
import path from 'node:path';
import { blake3HexOf, CONTENT_ROOT } from './_shared';

type Entry = { size: number; mtimeMs: number; hash: string };

/** Lives beside the content, under a dotfile sync itself never looks at. */
let CACHE_FILE = path.join(CONTENT_ROOT, '.sync-hashes.json');
const VERSION = 1;

let memory: Map<string, Entry> | null = null;
let dirty = false;
/** Serialises writes so two requests cannot interleave read-modify-write. */
let writing: Promise<void> = Promise.resolve();

/** Point the cache somewhere else and start cold. Tests only. */
export function useCacheFileForTests(file: string) {
  CACHE_FILE = file;
  memory = null;
  dirty = false;
}

async function load(): Promise<Map<string, Entry>> {
  if (memory) return memory;
  memory = new Map();
  try {
    const raw = JSON.parse(await fs.readFile(CACHE_FILE, 'utf8'));
    if (raw?.version === VERSION && raw.entries) {
      for (const [key, value] of Object.entries(raw.entries as Record<string, Entry>)) {
        memory.set(key, value);
      }
    }
  } catch {
    // No cache, unreadable cache, cache from an older shape: start empty. It
    // costs one slow manifest and then it is warm again.
  }
  return memory;
}

/** Hash of `file`, from the cache when its size and mtime still match. */
export async function cachedHash(file: string, relPath: string): Promise<string> {
  const cache = await load();
  const stat = await fs.stat(file);
  const hit = cache.get(relPath);
  if (hit && hit.size === stat.size && hit.mtimeMs === stat.mtimeMs) {
    return hit.hash;
  }

  const hash = blake3HexOf(new Uint8Array(await fs.readFile(file)));
  cache.set(relPath, { size: stat.size, mtimeMs: stat.mtimeMs, hash });
  dirty = true;
  return hash;
}

/**
 * Record a hash the caller already computed — an upload verifies one anyway,
 * so the next manifest gets it for free.
 */
export async function rememberHash(file: string, relPath: string, hash: string) {
  const cache = await load();
  try {
    const stat = await fs.stat(file);
    cache.set(relPath, { size: stat.size, mtimeMs: stat.mtimeMs, hash });
    dirty = true;
  } catch {
    // The file vanished between write and stat; nothing worth remembering.
  }
}

export async function forgetHash(relPath: string) {
  const cache = await load();
  if (cache.delete(relPath)) dirty = true;
}

/** Persist, if anything changed. Safe to call on every request. */
export async function flushHashCache(): Promise<void> {
  if (!dirty) return;
  writing = writing.then(async () => {
    if (!dirty || !memory) return;
    const entries = Object.fromEntries(memory);
    dirty = false;
    const temp = `${CACHE_FILE}.${process.pid}.tmp`;
    try {
      await fs.mkdir(path.dirname(CACHE_FILE), { recursive: true });
      await fs.writeFile(temp, JSON.stringify({ version: VERSION, entries }));
      await fs.rename(temp, CACHE_FILE);
    } catch {
      // A cache we cannot persist is still a cache for this process. Losing it
      // must never fail the request that filled it.
      await fs.rm(temp, { force: true }).catch(() => {});
    }
  });
  return writing;
}
