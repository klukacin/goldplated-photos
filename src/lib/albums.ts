import { getCollection, type CollectionEntry } from 'astro:content';
import fs from 'fs/promises';
import path from 'path';
import { createHash } from 'crypto';
import sharp from 'sharp';
import * as exifr from 'exifr';
import { marked } from 'marked';

export type Album = CollectionEntry<'albums'>;

export interface AlbumWithPhotos extends Album {
  photos: Photo[];
  subAlbums: Album[];
}

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
}

// Supported file extensions
export const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif'];
export const VIDEO_EXTENSIONS = ['.mp4', '.webm', '.mov', '.avi', '.mkv', '.m4v'];
export const ARCHIVE_EXTENSIONS = ['.zip', '.rar', '.7z'];

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
 * Get media (photos and videos) from an album directory
 */
export async function getPhotosForAlbum(albumPath: string): Promise<Photo[]> {
  const albumDir = path.join(process.cwd(), 'src/content/albums', albumPath);

  try {
    const files = await fs.readdir(albumDir);
    const allMediaExtensions = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];

    const mediaPromises = files
      .filter(file => {
        // Skip hidden files (e.g., macOS resource forks like ._filename.jpg)
        if (file.startsWith('.')) return false;
        const ext = path.extname(file).toLowerCase();
        return allMediaExtensions.includes(ext);
      })
      .map(async filename => {
        const filePath = path.join(albumDir, filename);
        const ext = path.extname(filename).toLowerCase();
        const isVideo = VIDEO_EXTENSIONS.includes(ext);

        try {
          const stats = await fs.stat(filePath);
          let width: number | undefined;
          let height: number | undefined;

          // Only read dimensions and EXIF for images (not videos)
          let exifDate: Date | undefined;
          if (!isVideo) {
            try {
              const metadata = await sharp(filePath).metadata();
              width = metadata.width;
              height = metadata.height;
              // Swap dimensions for rotated images (EXIF orientation 5-8 involve 90° rotation)
              if (metadata.orientation && metadata.orientation >= 5 && width && height) {
                [width, height] = [height, width];
              }
            } catch (metaError) {
              console.warn(`Could not read dimensions for ${filename}:`, metaError);
            }

            // Extract EXIF date
            try {
              const exifData = await exifr.parse(filePath, { pick: ['DateTimeOriginal'] });
              if (exifData?.DateTimeOriginal) {
                exifDate = new Date(exifData.DateTimeOriginal);
              }
            } catch (exifError) {
              // Silently ignore EXIF errors
            }
          }

          return {
            filename,
            path: filePath,
            url: `/albums/${albumPath}/${filename}`,
            size: stats.size,
            mtime: stats.mtime,
            exifDate,
            width,
            height,
            isVideo
          };
        } catch (error) {
          console.error(`Error reading stats for ${filename}:`, error);
          return {
            filename,
            path: filePath,
            url: `/albums/${albumPath}/${filename}`,
            isVideo
          };
        }
      });

    return await Promise.all(mediaPromises);
  } catch (error) {
    console.error(`Error reading album directory: ${albumDir}`, error);
    return [];
  }
}

/**
 * Get counts of photos and videos in an album
 */
export async function getMediaCounts(albumPath: string): Promise<{ photos: number; videos: number }> {
  const media = await getPhotosForAlbum(albumPath);
  return {
    photos: media.filter(m => !m.isVideo).length,
    videos: media.filter(m => m.isVideo).length
  };
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
 * Check if password is correct for album
 */
export function checkPassword(album: Album, password: string): boolean {
  if (!album.data.password) return true;
  return album.data.password === password;
}

/**
 * Sort photos based on album settings
 */
export async function sortPhotos(photos: Photo[], sortOrder: string): Promise<Photo[]> {
  switch (sortOrder) {
    case 'name':
      return photos.sort((a, b) => a.filename.localeCompare(b.filename));
    case 'date-asc':
    case 'date-desc':
      // Will be implemented with EXIF data
      return photos;
    case 'custom':
    default:
      return photos;
  }
}

/**
 * Generate stable token for album path
 */
export function generateAlbumToken(albumPath: string): string {
  return createHash('sha256')
    .update(albumPath)
    .digest('hex')
    .substring(0, 12);
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

/**
 * Get all descendants (recursive, all levels)
 */
export async function getAllDescendants(parentPath: string): Promise<Album[]> {
  const albums = await getAllAlbums();
  const pathPrefix = parentPath ? parentPath + '/' : '';

  return albums.filter(album => {
    const albumId = album.id.replace('/index.md', '');
    return albumId.startsWith(pathPrefix) && albumId !== parentPath;
  });
}

/**
 * Check if user has access to album
 */
export function checkAccess(
  album: Album,
  ancestors: Album[],
  unlockedTokens: Set<string>,
  providedToken?: string
): {
  hasAccess: boolean;
  requiresPassword: boolean;
  blockingAncestor?: Album;
} {
  // 1. Valid token + allowAnonymous = bypass parent passwords
  if (providedToken === album.data.token && album.data.allowAnonymous) {
    return { hasAccess: true, requiresPassword: false };
  }

  // 2. Check album password
  if (album.data.password && !unlockedTokens.has(album.data.token)) {
    return { hasAccess: false, requiresPassword: true };
  }

  // 3. Check ancestor passwords (inheritance)
  for (const ancestor of ancestors) {
    if (ancestor.data.password && !unlockedTokens.has(ancestor.data.token)) {
      return {
        hasAccess: false,
        requiresPassword: true,
        blockingAncestor: ancestor
      };
    }
  }

  return { hasAccess: true, requiresPassword: false };
}
