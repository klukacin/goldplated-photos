import { getCollection, type CollectionEntry } from 'astro:content';
import fs from 'fs/promises';
import path from 'path';
import { marked } from 'marked';
import { getAlbumMediaMeta } from './media-cache';
import { IMAGE_EXTENSIONS } from './image-formats';

export type Album = CollectionEntry<'albums'>;

export interface Photo {
  filename: string;
  path: string;
  url: string;
  size?: number;
  mtime?: Date;
  exifDate?: Date;
  width?: number;
  height?: number;
  isVideo?: boolean;
  /** Tiny base64 JPEG data URI for blur-up placeholders */
  blur?: string | null;
  /** Camera make+model from EXIF */
  camera?: string | null;
}

// Supported file extensions. The image list lives in image-formats.ts next to
// the rule about which of them a browser can actually paint — the two have to
// move together, or adding a format quietly adds broken images. The video
// list comes from the same shared module the admin server reads.
import { VIDEO_EXTENSIONS } from '../site-features.mjs';
export { IMAGE_EXTENSIONS } from './image-formats';
export { VIDEO_EXTENSIONS };

const ARCHIVE_EXTENSIONS = ['.zip', '.rar', '.7z'];

export interface ArchiveFile {
  filename: string;
  size: number;
  url: string;
}

/**
 * Get all albums from the content collection
 */
export async function getAllAlbums(): Promise<Album[]> {
  return await getCollection('albums');
}

/**
 * Get album by path (e.g., "2025/Birthdays/Johns-Birthday")
 */
export async function getAlbumByPath(albumPath: string): Promise<Album | undefined> {
  const albums = await getAllAlbums();
  const searchPath = albumPath.toLowerCase();
  return albums.find(album => album.id.toLowerCase() === searchPath);
}

/**
 * Get album body content from body.md file (rendered as HTML)
 */
export async function getAlbumBody(albumPath: string): Promise<string> {
  const bodyPath = path.join(process.cwd(), 'src/content/albums', albumPath, 'body.md');
  try {
    const content = await fs.readFile(bodyPath, 'utf-8');
    return await marked.parse(content);
  } catch {
    return '';
  }
}

/**
 * Parse inline markdown (for short text like descriptions)
 */
export function parseInlineMarkdown(text: string): string {
  if (!text) return '';
  return marked.parseInline(text) as string;
}

/**
 * Get sub-albums for a given path
 */
export async function getSubAlbums(parentPath: string): Promise<Album[]> {
  const albums = await getAllAlbums();
  const pathPrefix = parentPath ? parentPath + '/' : '';

  const filtered = albums.filter(album => {
    const albumId = album.id.replace('/index.md', '');
    const relativePath = albumId.startsWith(pathPrefix) ? albumId.slice(pathPrefix.length) : null;

    // Check if this is a direct child (no additional slashes)
    // AND not hidden (hidden albums only accessible via direct link)
    return relativePath &&
           !relativePath.includes('/') &&
           albumId !== parentPath &&
           !album.data.hidden;
  });

  // Sort by order field (if defined), then by title
  return filtered.sort((a, b) => {
    const orderA = a.data.order ?? Infinity;
    const orderB = b.data.order ?? Infinity;
    if (orderA !== orderB) return orderA - orderB;
    return a.data.title.localeCompare(b.data.title);
  });
}

/**
 * Get media (photos and videos) from an album directory.
 *
 * Dimensions, EXIF date, camera and blur previews come from the per-album
 * metadata cache (.meta/index.json) — only new or changed files trigger
 * actual image processing (see src/lib/media-cache.ts).
 */
export async function getPhotosForAlbum(albumPath: string): Promise<Photo[]> {
  const albumDir = path.join(process.cwd(), 'src/content/albums', albumPath);

  try {
    const files = await fs.readdir(albumDir);
    const allMediaExtensions = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];

    const mediaFiles = files.filter(file => {
      // Skip hidden files (e.g., macOS resource forks like ._filename.jpg)
      if (file.startsWith('.')) return false;
      const ext = path.extname(file).toLowerCase();
      return allMediaExtensions.includes(ext);
    });

    // Cheap stat pass — the expensive metadata comes from the cache
    const stats = await Promise.all(
      mediaFiles.map(async filename => {
        const ext = path.extname(filename).toLowerCase();
        const isVideo = VIDEO_EXTENSIONS.includes(ext);
        try {
          const stat = await fs.stat(path.join(albumDir, filename));
          return { filename, size: stat.size, mtimeMs: stat.mtimeMs, isVideo };
        } catch (error) {
          console.error(`Error reading stats for ${filename}:`, error);
          return null;
        }
      })
    );
    const liveStats = stats.filter((s): s is NonNullable<typeof s> => s !== null);

    const metaByName = await getAlbumMediaMeta(albumDir, liveStats);

    return liveStats.map(stat => {
      const meta = metaByName.get(stat.filename);
      return {
        filename: stat.filename,
        path: path.join(albumDir, stat.filename),
        url: `/albums/${albumPath}/${stat.filename}`,
        size: stat.size,
        mtime: new Date(stat.mtimeMs),
        exifDate: meta?.exifDate ? new Date(meta.exifDate) : undefined,
        width: meta?.width,
        height: meta?.height,
        isVideo: stat.isVideo,
        blur: meta?.blur ?? null,
        camera: meta?.camera ?? null
      };
    });
  } catch (error) {
    console.error(`Error reading album directory: ${albumDir}`, error);
    return [];
  }
}

/**
 * Get archive files (ZIP, RAR, 7z) from an album directory
 */
export async function getArchiveFiles(albumPath: string): Promise<ArchiveFile[]> {
  const albumDir = path.join(process.cwd(), 'src/content/albums', albumPath);

  try {
    const files = await fs.readdir(albumDir);

    const archivePromises = files
      .filter(file => {
        if (file.startsWith('.')) return false;
        const ext = path.extname(file).toLowerCase();
        return ARCHIVE_EXTENSIONS.includes(ext);
      })
      .map(async filename => {
        const filePath = path.join(albumDir, filename);
        try {
          const stats = await fs.stat(filePath);
          return {
            filename,
            size: stats.size,
            url: `/albums/${albumPath}/${filename}`
          };
        } catch {
          return null;
        }
      });

    const results = await Promise.all(archivePromises);
    return results.filter((f): f is ArchiveFile => f !== null);
  } catch {
    return [];
  }
}

/**
 * Get cover photo URL for an album
 * Returns the specified thumbnail, first photo, or null
 */
export async function getAlbumCoverPhoto(albumPath: string, specifiedThumbnail?: string): Promise<string | null> {
  // If thumbnail is specified in metadata, use it
  if (specifiedThumbnail) {
    return `/albums/${albumPath}/${specifiedThumbnail}`;
  }

  // Otherwise, try to get the first photo from the album
  const photos = await getPhotosForAlbum(albumPath);
  if (photos.length > 0) {
    return photos[0].url;
  }

  // No cover photo available
  return null;
}

/**
 * Build breadcrumb navigation from path
 */
export function buildBreadcrumbs(albumPath: string): Array<{ label: string; path: string }> {
  if (!albumPath) return [];

  const parts = albumPath.split('/');
  const breadcrumbs = [
    { label: 'Home', path: '/home' },
    { label: 'Photos', path: '/photos' }
  ];

  parts.forEach((part, index) => {
    const path = '/photos/' + parts.slice(0, index + 1).join('/');
    breadcrumbs.push({
      label: part.replace(/-/g, ' '),
      path
    });
  });

  return breadcrumbs;
}

/**
 * Get all ancestor albums (bottom-up)
 */
export async function getAncestors(albumPath: string): Promise<Album[]> {
  const ancestors: Album[] = [];
  const parts = albumPath.split('/');

  for (let i = parts.length - 1; i > 0; i--) {
    const ancestorPath = parts.slice(0, i).join('/');
    const ancestor = await getAlbumByPath(ancestorPath);
    if (ancestor) ancestors.push(ancestor);
  }

  return ancestors;
}

