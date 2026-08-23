/**
 * The photo listing behind /photos/search — read-only and cheap on purpose.
 *
 * Search used to call getPhotosForAlbum() per public album per query, which
 * stat()s every photo to revalidate the media cache and computes metadata
 * (Sharp + exifr) for anything missing. Measured on a 300-album / 3600-photo
 * library that was ~120ms of filesystem work per query, ~75ms of it the
 * per-file stats — validating a cache whose values search then displayed
 * unchanged. Worse, the first search after a big import would be the request
 * that ran Sharp over every new photo.
 *
 * This module does neither. The directory listing is the authority on which
 * files exist; everything else comes from `.meta/index.json` exactly as the
 * last album-page or thumbnail request left it. A photo the cache has not
 * met yet is still searchable by filename — it simply has no camera or
 * capture date to match on until something else populates the cache. Search
 * results are links; the page behind the link is always fresh.
 *
 * Cache files are kept in memory keyed by (path, mtime, size), so a
 * steady-state query costs one readdir and one stat per album.
 */
import fs from 'fs/promises';
import path from 'path';
import { IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from '../site-features.mjs';
import type { MediaMeta } from './media-cache';

/** What search matching and result rendering need — nothing stat-derived. */
export interface SearchablePhoto {
  filename: string;
  isVideo: boolean;
  camera: string | null;
  exifDate: Date | null;
  width?: number;
  height?: number;
  blur?: string | null;
}

interface CachedFile {
  mtimeMs: number;
  size: number;
  entries: Record<string, MediaMeta>;
}

// Per-process memo of parsed cache files. Bounded by the number of albums,
// and each parsed file is small (a few KB per album).
const memo = new Map<string, CachedFile>();

/** The album's cache entries as of the file's current mtime; {} without one. */
async function readCacheEntries(albumDir: string): Promise<Record<string, MediaMeta>> {
  const file = path.join(albumDir, '.meta', 'index.json');
  let stat;
  try {
    stat = await fs.stat(file);
  } catch {
    memo.delete(albumDir);
    return {};
  }

  const held = memo.get(albumDir);
  if (held && held.mtimeMs === stat.mtimeMs && held.size === stat.size) {
    return held.entries;
  }

  try {
    const parsed = JSON.parse(await fs.readFile(file, 'utf-8'));
    const entries: Record<string, MediaMeta> =
      parsed && typeof parsed.entries === 'object' && parsed.entries ? parsed.entries : {};
    memo.set(albumDir, { mtimeMs: stat.mtimeMs, size: stat.size, entries });
    return entries;
  } catch {
    // Torn or corrupt cache — treat as absent rather than failing the search
    memo.delete(albumDir);
    return {};
  }
}

/**
 * List an album directory's media for search: one readdir, one stat (of the
 * cache file), zero metadata computation.
 */
export async function getSearchablePhotos(albumDir: string): Promise<SearchablePhoto[]> {
  let files: string[];
  try {
    files = await fs.readdir(albumDir);
  } catch {
    return [];
  }

  const entries = await readCacheEntries(albumDir);

  const photos: SearchablePhoto[] = [];
  for (const filename of files) {
    if (filename.startsWith('.')) continue;
    const ext = path.extname(filename).toLowerCase();
    const isVideo = VIDEO_EXTENSIONS.includes(ext);
    if (!isVideo && !IMAGE_EXTENSIONS.includes(ext)) continue;

    const meta = entries[filename];
    photos.push({
      filename,
      isVideo,
      camera: meta?.camera ?? null,
      exifDate: meta?.exifDate ? new Date(meta.exifDate) : null,
      width: meta?.width,
      height: meta?.height,
      blur: meta?.blur ?? null
    });
  }
  return photos;
}
