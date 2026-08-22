/**
 * Per-album media metadata cache.
 *
 * Reading dimensions (Sharp) and EXIF (exifr) for every photo on every SSR
 * request made page load time scale with album size. This module persists
 * that metadata to `<album>/.meta/index.json`, keyed by filename and
 * invalidated per file by mtime+size, so steady-state requests do zero image
 * I/O. It also stores a tiny base64 blur preview (LQIP) per photo for the
 * grid's blur-up placeholders, and the camera model for photo search.
 */
import fs from 'fs/promises';
import path from 'path';
import sharp from 'sharp';
import * as exifr from 'exifr';
import { imageJobSemaphore } from './semaphore';

// Bump when the entry shape or computation changes — stale versions recompute.
const CACHE_VERSION = 1;

export interface MediaMeta {
  filename: string;
  size: number;
  mtimeMs: number;
  isVideo: boolean;
  width?: number;
  height?: number;
  /** ISO timestamp of EXIF DateTimeOriginal (null = none found) */
  exifDate?: string | null;
  /** Camera make+model from EXIF (null = none found) */
  camera?: string | null;
  /** Tiny base64 JPEG data URI for blur-up placeholders (photos only) */
  blur?: string | null;
}

interface CacheFile {
  version: number;
  entries: Record<string, MediaMeta>;
}

export interface MediaFileStat {
  filename: string;
  size: number;
  mtimeMs: number;
  isVideo: boolean;
}

function cachePath(albumDir: string): string {
  return path.join(albumDir, '.meta', 'index.json');
}

async function loadCache(albumDir: string): Promise<CacheFile> {
  try {
    const raw = await fs.readFile(cachePath(albumDir), 'utf-8');
    const parsed = JSON.parse(raw) as CacheFile;
    if (parsed && parsed.version === CACHE_VERSION && parsed.entries) {
      return parsed;
    }
  } catch {
    // Missing or corrupt cache — rebuild below
  }
  return { version: CACHE_VERSION, entries: {} };
}

async function saveCache(albumDir: string, cache: CacheFile): Promise<void> {
  const file = cachePath(albumDir);
  try {
    await fs.mkdir(path.dirname(file), { recursive: true });
    // Write via temp file + rename so concurrent readers never see a torn file
    const tmp = `${file}.${process.pid}.tmp`;
    await fs.writeFile(tmp, JSON.stringify(cache));
    await fs.rename(tmp, file);
  } catch (error) {
    // Cache persistence is best-effort — metadata was still computed in memory
    console.warn(`[media-cache] Could not write ${file}:`, error);
  }
}

/** Compute metadata for a single photo (dimensions, EXIF, blur preview). */
async function computePhotoMeta(albumDir: string, stat: MediaFileStat): Promise<MediaMeta> {
  const filePath = path.join(albumDir, stat.filename);
  const meta: MediaMeta = { ...stat, exifDate: null, camera: null, blur: null };

  await imageJobSemaphore.run(async () => {
    try {
      const image = sharp(filePath);
      const info = await image.metadata();
      let width = info.width;
      let height = info.height;
      // Swap dimensions for rotated images (EXIF orientation 5-8 involve 90° rotation)
      if (info.orientation && info.orientation >= 5 && width && height) {
        [width, height] = [height, width];
      }
      meta.width = width;
      meta.height = height;

      // Tiny blur preview (~20px wide) as inline data URI
      try {
        const blurBuffer = await image
          .rotate()
          .resize(20, 20, { fit: 'inside' })
          .jpeg({ quality: 40 })
          .toBuffer();
        meta.blur = `data:image/jpeg;base64,${blurBuffer.toString('base64')}`;
      } catch {
        meta.blur = null;
      }
    } catch (error) {
      console.warn(`[media-cache] Could not read dimensions for ${stat.filename}:`, error);
    }

    try {
      const exifData = await exifr.parse(filePath, { pick: ['DateTimeOriginal', 'Make', 'Model'] });
      if (exifData?.DateTimeOriginal) {
        meta.exifDate = new Date(exifData.DateTimeOriginal).toISOString();
      }
      const camera = [exifData?.Make, exifData?.Model].filter(Boolean).join(' ').trim();
      meta.camera = camera || null;
    } catch {
      // No EXIF — fine
    }
  });

  return meta;
}

/**
 * Return up-to-date metadata for the given media files of an album.
 * Cached entries are reused when filename+size+mtime match; everything else
 * is (re)computed and the cache file is rewritten.
 */
export async function getAlbumMediaMeta(
  albumDir: string,
  files: MediaFileStat[]
): Promise<Map<string, MediaMeta>> {
  const cache = await loadCache(albumDir);
  const result = new Map<string, MediaMeta>();
  let dirty = false;

  const toCompute: MediaFileStat[] = [];

  for (const stat of files) {
    const cached = cache.entries[stat.filename];
    if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size && cached.isVideo === stat.isVideo) {
      result.set(stat.filename, cached);
    } else if (stat.isVideo) {
      // Videos carry no Sharp/EXIF metadata — the stat itself is the entry
      const meta: MediaMeta = { ...stat };
      result.set(stat.filename, meta);
      cache.entries[stat.filename] = meta;
      dirty = true;
    } else {
      toCompute.push(stat);
    }
  }

  if (toCompute.length > 0) {
    const computed = await Promise.all(toCompute.map(stat => computePhotoMeta(albumDir, stat)));
    for (const meta of computed) {
      result.set(meta.filename, meta);
      cache.entries[meta.filename] = meta;
    }
    dirty = true;
  }

  // Prune entries for files that no longer exist
  const liveNames = new Set(files.map(f => f.filename));
  for (const name of Object.keys(cache.entries)) {
    if (!liveNames.has(name)) {
      delete cache.entries[name];
      dirty = true;
    }
  }

  if (dirty) {
    await saveCache(albumDir, cache);
  }

  return result;
}
