#!/usr/bin/env node
/**
 * Clear thumbnail cache
 * Usage:
 *   npm run clear <album-path>  - Clear specific album (e.g., ws/parties)
 *   npm run clear:all           - Clear all thumbnails
 */

import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const albumsDir = path.join(__dirname, '..', 'src/content/albums');

async function clearAlbum(albumPath) {
  const thumbnailDir = path.join(albumsDir, albumPath, '.meta', 'thumbnails');

  try {
    await fs.access(thumbnailDir);
    await fs.rm(thumbnailDir, { recursive: true });
    console.log(`✓ Cleared thumbnails for: ${albumPath}`);
  } catch (err) {
    if (err.code === 'ENOENT') {
      console.log(`  No thumbnails found for: ${albumPath}`);
    } else {
      console.error(`✗ Error clearing ${albumPath}:`, err.message);
    }
  }
}

async function clearAll() {
  console.log('Clearing all thumbnails...\n');

  let cleared = 0;

  async function findAndClear(dir, relativePath = '') {
    const entries = await fs.readdir(dir, { withFileTypes: true });

    for (const entry of entries) {
      if (entry.isDirectory()) {
        if (entry.name === '.meta') {
          const thumbnailDir = path.join(dir, entry.name, 'thumbnails');
          try {
            await fs.access(thumbnailDir);
            await fs.rm(thumbnailDir, { recursive: true });
            console.log(`✓ ${relativePath || 'root'}`);
            cleared++;
          } catch {
            // No thumbnails directory
          }
        } else if (!entry.name.startsWith('.')) {
          await findAndClear(
            path.join(dir, entry.name),
            relativePath ? `${relativePath}/${entry.name}` : entry.name
          );
        }
      }
    }
  }

  await findAndClear(albumsDir);

  console.log(`\n${cleared} album(s) cleared.`);
}

// Main
const args = process.argv.slice(2);

if (args[0] === '--all') {
  await clearAll();
} else if (args.length > 0) {
  for (const albumPath of args) {
    await clearAlbum(albumPath);
  }
} else {
  console.log(`
Usage:
  npm run clear <album-path>   Clear specific album (e.g., ws/parties)
  npm run clear:all            Clear all thumbnails

Examples:
  npm run clear ws/parties
  npm run clear friends/vacation weddings/john-jane
  npm run clear:all
`);
}
