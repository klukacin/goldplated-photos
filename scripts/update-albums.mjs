#!/usr/bin/env node

/**
 * Album Update Script
 *
 * This script:
 * 1. Renames folders with uppercase letters to lowercase (for case-sensitive filesystems)
 * 2. Removes all .md files that are NOT index.md from album folders
 * 3. Creates a default index.md for any folder containing images but no index.md
 * 4. Auto-selects thumbnail for albums (first image, or first image from first subfolder)
 * 5. Updates existing index.md files to add thumbnail if missing
 *
 * Usage: npm run update
 */

import fs from 'fs';
import path from 'path';
import crypto from 'crypto';

const ALBUMS_DIR = path.join(process.cwd(), 'src/content/albums');
const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif'];

// Check if folder name needs to be lowercase
function needsSanitization(name) {
  return name !== name.toLowerCase();
}

// Rename folder to lowercase (with temp rename for case-insensitive fs)
function renameToLowercase(dirPath, oldName, stats) {
  const newName = oldName.toLowerCase();
  const parentDir = path.dirname(dirPath);
  const newPath = path.join(parentDir, newName);
  const tempPath = path.join(parentDir, `__temp_${Date.now()}_${newName}`);

  try {
    // Use temp name to handle case-only renames on case-insensitive filesystems
    fs.renameSync(dirPath, tempPath);
    fs.renameSync(tempPath, newPath);
    console.log(`  Renamed: ${oldName} → ${newName}`);
    stats.renamed++;
    return newPath;
  } catch (err) {
    console.error(`  Error renaming "${oldName}": ${err.message}`);
    // Try to restore
    if (fs.existsSync(tempPath)) {
      try {
        fs.renameSync(tempPath, dirPath);
      } catch {
        console.error(`    CRITICAL: Failed to restore "${oldName}" from temp!`);
      }
    }
    return dirPath;
  }
}

// Generate a unique token based on the folder path
function generateToken(folderPath) {
  const hash = crypto.createHash('md5').update(folderPath + Date.now()).digest('hex');
  return hash.substring(0, 12);
}

// Generate title from folder name
function generateTitle(folderName) {
  // Remove leading numbers like "01.", "16.", etc.
  let title = folderName.replace(/^\d+\.?\s*/, '');
  // Replace hyphens and underscores with spaces
  title = title.replace(/[-_]/g, ' ');
  // Only capitalize first letter, preserve rest as-is (respects Croatian diacritics)
  title = title.charAt(0).toUpperCase() + title.slice(1);
  return title || folderName;
}

// Check if a directory contains image files
function containsImages(dirPath) {
  try {
    const files = fs.readdirSync(dirPath);
    return files.some(file => {
      const ext = path.extname(file).toLowerCase();
      return IMAGE_EXTENSIONS.includes(ext);
    });
  } catch {
    return false;
  }
}

// Check if a directory contains subdirectories (making it a collection)
function containsSubdirectories(dirPath) {
  try {
    const files = fs.readdirSync(dirPath, { withFileTypes: true });
    return files.some(dirent => dirent.isDirectory() && !dirent.name.startsWith('.'));
  } catch {
    return false;
  }
}

// Get first image from a directory (sorted alphabetically)
function getFirstImageInDir(dirPath) {
  try {
    const files = fs.readdirSync(dirPath)
      .filter(file => {
        const ext = path.extname(file).toLowerCase();
        return IMAGE_EXTENSIONS.includes(ext) && !file.startsWith('.');
      })
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));

    return files.length > 0 ? files[0] : null;
  } catch {
    return null;
  }
}

// Get first image from directory, or recursively from first subfolder if none
function getFirstImage(dirPath) {
  // First, try to find an image in the current directory
  const firstImage = getFirstImageInDir(dirPath);
  if (firstImage) {
    return firstImage;
  }

  // If no images, try the first subfolder
  try {
    const subdirs = fs.readdirSync(dirPath, { withFileTypes: true })
      .filter(d => d.isDirectory() && !d.name.startsWith('.'))
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));

    for (const subdir of subdirs) {
      const subImage = getFirstImageInDir(path.join(dirPath, subdir.name));
      if (subImage) {
        // Return path relative to original directory
        return path.join(subdir.name, subImage);
      }
    }
  } catch {
    // Ignore errors
  }

  return null;
}

// Check if index.md has a thumbnail field
function hasThumbnailField(indexPath) {
  try {
    const content = fs.readFileSync(indexPath, 'utf-8');
    // Check if thumbnail field exists in frontmatter
    const frontmatterMatch = content.match(/^---\n([\s\S]*?)\n---/);
    if (frontmatterMatch) {
      return /^thumbnail:/m.test(frontmatterMatch[1]);
    }
    return false;
  } catch {
    return false;
  }
}

// Update existing index.md to add thumbnail field
function updateExistingIndexMd(indexPath, thumbnail, stats) {
  if (!thumbnail) return;
  if (hasThumbnailField(indexPath)) return;

  try {
    const content = fs.readFileSync(indexPath, 'utf-8');
    const frontmatterMatch = content.match(/^(---\n)([\s\S]*?)(\n---)/);

    if (frontmatterMatch) {
      const [fullMatch, start, frontmatter, end] = frontmatterMatch;
      const newFrontmatter = frontmatter.trimEnd() + `\nthumbnail: "${thumbnail}"`;
      const newContent = content.replace(fullMatch, start + newFrontmatter + end);

      fs.writeFileSync(indexPath, newContent, 'utf-8');
      const relativePath = path.relative(ALBUMS_DIR, indexPath);
      console.log(`  Updated: ${relativePath} (added thumbnail: ${thumbnail})`);
      stats.updated++;
    }
  } catch (err) {
    console.error(`  Error updating ${indexPath}: ${err.message}`);
  }
}

// Create default index.md content
function createDefaultIndexContent(folderName, folderPath, isCollection, thumbnail) {
  const title = generateTitle(folderName);
  const token = generateToken(folderPath);

  let content = `---
title: "${title}"
token: "${token}"
style: single-column
isCollection: ${isCollection}`;

  if (thumbnail) {
    content += `\nthumbnail: "${thumbnail}"`;
  }

  content += `\n---\n`;

  return content;
}

// Process a single directory
function processDirectory(dirPath, stats) {
  let currentPath = dirPath;
  let folderName = path.basename(currentPath);

  // Skip hidden directories
  if (folderName.startsWith('.')) {
    return;
  }

  // First, rename folder to lowercase if needed
  if (needsSanitization(folderName)) {
    currentPath = renameToLowercase(currentPath, folderName, stats);
    folderName = path.basename(currentPath);
  }

  const relativePath = path.relative(ALBUMS_DIR, currentPath);

  let files;
  try {
    files = fs.readdirSync(currentPath, { withFileTypes: true });
  } catch (err) {
    console.error(`  Error reading ${relativePath}: ${err.message}`);
    return;
  }

  // Find and remove non-index.md markdown files
  const mdFiles = files.filter(f =>
    f.isFile() &&
    f.name.endsWith('.md') &&
    f.name !== 'index.md'
  );

  for (const mdFile of mdFiles) {
    const mdPath = path.join(currentPath, mdFile.name);
    try {
      fs.unlinkSync(mdPath);
      console.log(`  Removed: ${path.join(relativePath, mdFile.name)}`);
      stats.removed++;
    } catch (err) {
      console.error(`  Error removing ${mdFile.name}: ${err.message}`);
    }
  }

  // Check if index.md exists
  const indexPath = path.join(currentPath, 'index.md');
  const hasIndex = fs.existsSync(indexPath);

  // Determine if this is a collection or album
  const hasSubdirs = containsSubdirectories(currentPath);
  const hasImages = containsImages(currentPath);

  // Get first image for thumbnail (from this folder or first subfolder)
  const thumbnail = getFirstImage(currentPath);

  // Only create index.md if the folder has content (images or subdirectories)
  if (!hasIndex && (hasImages || hasSubdirs)) {
    const isCollection = hasSubdirs && !hasImages;
    const content = createDefaultIndexContent(folderName, currentPath, isCollection, thumbnail);

    try {
      fs.writeFileSync(indexPath, content, 'utf-8');
      console.log(`  Created: ${path.join(relativePath, 'index.md')} (${isCollection ? 'collection' : 'album'}${thumbnail ? ', thumbnail: ' + thumbnail : ''})`);
      stats.created++;
    } catch (err) {
      console.error(`  Error creating index.md: ${err.message}`);
    }
  } else if (hasIndex) {
    // Update existing index.md to add thumbnail if missing
    updateExistingIndexMd(indexPath, thumbnail, stats);
  }

  // Process subdirectories
  const subdirs = files.filter(f => f.isDirectory() && !f.name.startsWith('.'));
  for (const subdir of subdirs) {
    processDirectory(path.join(currentPath, subdir.name), stats);
  }
}

// Main function
function main() {
  console.log('Album Update Script');
  console.log('===================\n');

  if (!fs.existsSync(ALBUMS_DIR)) {
    console.error(`Error: Albums directory not found: ${ALBUMS_DIR}`);
    process.exit(1);
  }

  const stats = { renamed: 0, removed: 0, created: 0, updated: 0 };

  console.log('Processing albums...\n');

  // Get top-level directories in albums folder
  const topLevelDirs = fs.readdirSync(ALBUMS_DIR, { withFileTypes: true })
    .filter(d => d.isDirectory() && !d.name.startsWith('.'));

  for (const dir of topLevelDirs) {
    processDirectory(path.join(ALBUMS_DIR, dir.name), stats);
  }

  console.log('\n===================');
  const parts = [];
  if (stats.renamed > 0) parts.push(`renamed ${stats.renamed} folders to lowercase`);
  if (stats.removed > 0) parts.push(`removed ${stats.removed} files`);
  if (stats.created > 0) parts.push(`created ${stats.created} index.md files`);
  if (stats.updated > 0) parts.push(`updated ${stats.updated} with thumbnails`);
  console.log(`Done! ${parts.length > 0 ? parts.join(', ') : 'No changes needed'}.`);
}

main();
