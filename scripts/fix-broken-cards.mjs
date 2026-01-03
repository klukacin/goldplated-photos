#!/usr/bin/env node
/**
 * Fix broken album/folder card thumbnails
 *
 * Scans all albums for invalid thumbnail references and auto-fixes them
 * by selecting the first available image.
 *
 * Usage:
 *   npm run fixbrokencards         - Fix all broken thumbnails
 *   npm run fixbrokencards --dry   - Preview fixes without applying
 */

import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';
import matter from 'gray-matter';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const albumsDir = path.join(__dirname, '..', 'src/content/albums');

const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif'];
const isDryRun = process.argv.includes('--dry');

// Colors for console output
const GREEN = '\x1b[32m';
const YELLOW = '\x1b[33m';
const RED = '\x1b[31m';
const DIM = '\x1b[2m';
const NC = '\x1b[0m';

/**
 * Find first image in a directory (searches up to maxDepth levels)
 */
async function findFirstImage(dir, maxDepth = 3, currentPath = '') {
  try {
    const entries = await fs.readdir(dir, { withFileTypes: true });

    // First, look for images directly in this folder
    const images = entries
      .filter(e => e.isFile() && !e.name.startsWith('.'))
      .filter(e => IMAGE_EXTENSIONS.includes(path.extname(e.name).toLowerCase()))
      .sort((a, b) => a.name.localeCompare(b.name));

    if (images.length > 0) {
      return currentPath ? `${currentPath}/${images[0].name}` : images[0].name;
    }

    // If no direct images and we haven't reached max depth, look in subfolders
    if (maxDepth > 0) {
      const subdirs = entries
        .filter(e => e.isDirectory() && !e.name.startsWith('.'))
        .sort((a, b) => a.name.localeCompare(b.name));

      for (const subdir of subdirs) {
        const subPath = path.join(dir, subdir.name);
        const newPath = currentPath ? `${currentPath}/${subdir.name}` : subdir.name;
        const result = await findFirstImage(subPath, maxDepth - 1, newPath);
        if (result) {
          return result;
        }
      }
    }

    return null;
  } catch {
    return null;
  }
}

/**
 * Check if thumbnail path is valid
 */
async function thumbnailExists(albumDir, thumbnailPath) {
  if (!thumbnailPath) return false;

  const fullPath = path.join(albumDir, thumbnailPath);
  try {
    await fs.access(fullPath);
    return true;
  } catch {
    return false;
  }
}

/**
 * Process a single album
 */
async function processAlbum(albumPath, relativePath) {
  const indexPath = path.join(albumPath, 'index.md');

  try {
    await fs.access(indexPath);
  } catch {
    return null; // No index.md
  }

  const content = await fs.readFile(indexPath, 'utf-8');
  const parsed = matter(content);
  const currentThumbnail = parsed.data.thumbnail;

  // Check if current thumbnail is valid
  const isValid = await thumbnailExists(albumPath, currentThumbnail);

  if (isValid) {
    return { status: 'ok', path: relativePath, thumbnail: currentThumbnail };
  }

  // Find a replacement
  const newThumbnail = await findFirstImage(albumPath);

  if (!newThumbnail) {
    return {
      status: 'empty',
      path: relativePath,
      oldThumbnail: currentThumbnail,
      message: 'No images found'
    };
  }

  if (!isDryRun) {
    // Update the index.md
    parsed.data.thumbnail = newThumbnail;
    const newContent = matter.stringify(parsed.content, parsed.data);
    await fs.writeFile(indexPath, newContent);
  }

  return {
    status: 'fixed',
    path: relativePath,
    oldThumbnail: currentThumbnail || '(none)',
    newThumbnail
  };
}

/**
 * Recursively find all albums
 */
async function findAllAlbums(dir, relativePath = '') {
  const albums = [];
  const entries = await fs.readdir(dir, { withFileTypes: true });

  // Check if this directory has an index.md (is an album)
  const hasIndex = entries.some(e => e.isFile() && e.name === 'index.md');
  if (hasIndex) {
    albums.push({ path: dir, relativePath: relativePath || '(root)' });
  }

  // Recurse into subdirectories
  for (const entry of entries) {
    if (entry.isDirectory() && !entry.name.startsWith('.')) {
      const subPath = path.join(dir, entry.name);
      const subRelative = relativePath ? `${relativePath}/${entry.name}` : entry.name;
      const subAlbums = await findAllAlbums(subPath, subRelative);
      albums.push(...subAlbums);
    }
  }

  return albums;
}

// Main
async function main() {
  console.log('');
  if (isDryRun) {
    console.log(`${YELLOW}DRY RUN${NC} - No changes will be made\n`);
  }
  console.log('Scanning albums for broken thumbnails...\n');

  const albums = await findAllAlbums(albumsDir);

  let okCount = 0;
  let fixedCount = 0;
  let emptyCount = 0;

  for (const album of albums) {
    const result = await processAlbum(album.path, album.relativePath);

    if (!result) continue;

    switch (result.status) {
      case 'ok':
        okCount++;
        break;

      case 'fixed':
        fixedCount++;
        console.log(`${GREEN}✓${NC} ${result.path}`);
        console.log(`  ${DIM}${result.oldThumbnail}${NC}`);
        console.log(`  ${GREEN}→ ${result.newThumbnail}${NC}`);
        console.log('');
        break;

      case 'empty':
        emptyCount++;
        console.log(`${YELLOW}⚠${NC} ${result.path}`);
        console.log(`  ${DIM}${result.oldThumbnail || '(no thumbnail set)'}${NC}`);
        console.log(`  ${YELLOW}No images found to use as thumbnail${NC}`);
        console.log('');
        break;
    }
  }

  // Summary
  console.log('─'.repeat(50));
  console.log(`${GREEN}✓ ${okCount} albums OK${NC}`);

  if (fixedCount > 0) {
    const action = isDryRun ? 'would be fixed' : 'fixed';
    console.log(`${GREEN}✓ ${fixedCount} albums ${action}${NC}`);
  }

  if (emptyCount > 0) {
    console.log(`${YELLOW}⚠ ${emptyCount} albums have no images${NC}`);
  }

  if (isDryRun && fixedCount > 0) {
    console.log(`\n${DIM}Run without --dry to apply fixes${NC}`);
  }
}

main().catch(console.error);
