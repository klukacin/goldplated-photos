#!/usr/bin/env node

/**
 * Folder Sanitization Script
 *
 * This script renames all album folders to lowercase to ensure:
 * 1. URLs work correctly on case-sensitive filesystems (Linux)
 * 2. Consistent path handling between local and production
 *
 * Usage: npm run sanitize
 *        node scripts/sanitize-folders.mjs [--dry-run]
 *
 * Options:
 *   --dry-run   Show what would be renamed without making changes
 */

import fs from 'fs';
import path from 'path';

const ALBUMS_DIR = path.join(process.cwd(), 'src/content/albums');
const DRY_RUN = process.argv.includes('--dry-run');

// Stats tracking
const stats = {
  scanned: 0,
  renamed: 0,
  conflicts: 0,
  errors: 0
};

/**
 * Sanitize a folder name to lowercase
 * Preserves numbers, hyphens, and underscores
 */
function sanitizeName(name) {
  return name.toLowerCase();
}

/**
 * Check if a name needs sanitization
 */
function needsSanitization(name) {
  return name !== sanitizeName(name);
}

/**
 * Rename a folder to lowercase
 * Uses a temp name to handle case-only renames on case-insensitive filesystems
 */
function renameToLowercase(dirPath, oldName) {
  const newName = sanitizeName(oldName);
  const parentDir = path.dirname(dirPath);
  const oldPath = dirPath;
  const newPath = path.join(parentDir, newName);
  const tempPath = path.join(parentDir, `__temp_${Date.now()}_${newName}`);

  // Check for conflicts (different folder with same lowercase name)
  if (fs.existsSync(newPath) && oldPath !== newPath) {
    // On case-insensitive filesystems, these might be the same folder
    // Check by comparing inodes or using a temp rename
    try {
      const oldStat = fs.statSync(oldPath);
      const newStat = fs.statSync(newPath);

      // If different inodes, it's a real conflict
      if (oldStat.ino !== newStat.ino) {
        console.log(`  ⚠️  CONFLICT: Cannot rename "${oldName}" → "${newName}" (target exists)`);
        stats.conflicts++;
        return false;
      }
    } catch {
      // If we can't stat, assume conflict
      console.log(`  ⚠️  CONFLICT: Cannot rename "${oldName}" → "${newName}" (target exists)`);
      stats.conflicts++;
      return false;
    }
  }

  if (DRY_RUN) {
    console.log(`  Would rename: ${oldName} → ${newName}`);
    stats.renamed++;
    return true;
  }

  try {
    // Use temp name to handle case-only renames on case-insensitive filesystems
    fs.renameSync(oldPath, tempPath);
    fs.renameSync(tempPath, newPath);
    console.log(`  ✓ Renamed: ${oldName} → ${newName}`);
    stats.renamed++;
    return true;
  } catch (err) {
    console.error(`  ✗ Error renaming "${oldName}": ${err.message}`);
    stats.errors++;

    // Try to restore original name if temp rename succeeded
    if (fs.existsSync(tempPath)) {
      try {
        fs.renameSync(tempPath, oldPath);
      } catch {
        console.error(`    CRITICAL: Failed to restore "${oldName}" from temp!`);
      }
    }
    return false;
  }
}

/**
 * Process a directory recursively (bottom-up to handle nested renames)
 */
function processDirectory(dirPath) {
  const folderName = path.basename(dirPath);

  // Skip hidden directories
  if (folderName.startsWith('.')) {
    return;
  }

  stats.scanned++;

  let entries;
  try {
    entries = fs.readdirSync(dirPath, { withFileTypes: true });
  } catch (err) {
    console.error(`  Error reading ${dirPath}: ${err.message}`);
    stats.errors++;
    return;
  }

  // Process subdirectories first (bottom-up)
  const subdirs = entries.filter(e => e.isDirectory() && !e.name.startsWith('.'));
  for (const subdir of subdirs) {
    processDirectory(path.join(dirPath, subdir.name));
  }

  // After processing children, rename this folder if needed
  if (needsSanitization(folderName)) {
    // Re-read the current path in case parent was renamed
    const parentDir = path.dirname(dirPath);
    const currentPath = path.join(parentDir, folderName);

    if (fs.existsSync(currentPath)) {
      renameToLowercase(currentPath, folderName);
    }
  }
}

/**
 * Main function
 */
function main() {
  console.log('Folder Sanitization Script');
  console.log('==========================');

  if (DRY_RUN) {
    console.log('🔍 DRY RUN MODE - No changes will be made\n');
  } else {
    console.log('');
  }

  if (!fs.existsSync(ALBUMS_DIR)) {
    console.error(`Error: Albums directory not found: ${ALBUMS_DIR}`);
    process.exit(1);
  }

  console.log(`Scanning: ${ALBUMS_DIR}\n`);

  // Get top-level directories
  const topLevelDirs = fs.readdirSync(ALBUMS_DIR, { withFileTypes: true })
    .filter(d => d.isDirectory() && !d.name.startsWith('.'));

  // Process each top-level directory
  for (const dir of topLevelDirs) {
    const dirPath = path.join(ALBUMS_DIR, dir.name);

    // Check if top-level needs renaming
    if (needsSanitization(dir.name)) {
      console.log(`📁 ${dir.name}`);
    }

    processDirectory(dirPath);

    // Rename top-level directory if needed (after processing children)
    if (needsSanitization(dir.name)) {
      renameToLowercase(dirPath, dir.name);
    }
  }

  // Summary
  console.log('\n==========================');
  console.log('Summary:');
  console.log(`  Folders scanned: ${stats.scanned}`);
  console.log(`  Folders renamed: ${stats.renamed}`);
  if (stats.conflicts > 0) {
    console.log(`  Conflicts: ${stats.conflicts}`);
  }
  if (stats.errors > 0) {
    console.log(`  Errors: ${stats.errors}`);
  }

  if (DRY_RUN && stats.renamed > 0) {
    console.log('\nRun without --dry-run to apply changes.');
  }

  if (stats.conflicts > 0 || stats.errors > 0) {
    process.exit(1);
  }
}

main();
