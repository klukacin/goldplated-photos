#!/usr/bin/env node
/**
 * Manage album share tokens (random secrets for link-sharing).
 *
 * A share token makes an album reachable via a secret link:
 *   https://your-site/photos/<album>?token=<shareToken>
 * An album with a shareToken and NO password is reachable ONLY via that link.
 *
 * Usage:
 *   node scripts/add-share-token.mjs <album-path> [...more]   Add/rotate token
 *   node scripts/add-share-token.mjs --remove <album-path>    Remove token
 *   node scripts/add-share-token.mjs --list                   List albums with tokens
 *
 * Album paths are relative to src/content/albums (e.g. "2025/weddings/john-jane").
 */
import { readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { randomBytes } from 'node:crypto';
import matter from 'gray-matter';

const ALBUMS_DIR = join(process.cwd(), 'src/content/albums');

function generateShareToken() {
  return randomBytes(16).toString('base64url');
}

function indexPath(albumPath) {
  return join(ALBUMS_DIR, albumPath, 'index.md');
}

function listAlbumsWithTokens(dir = ALBUMS_DIR, prefix = '') {
  const results = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue;
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    const idx = join(dir, entry.name, 'index.md');
    if (existsSync(idx)) {
      const { data } = matter(readFileSync(idx, 'utf-8'));
      if (data.shareToken) results.push({ path: rel, hasPassword: !!data.password });
    }
    results.push(...listAlbumsWithTokens(join(dir, entry.name), rel));
  }
  return results;
}

function updateToken(albumPath, remove) {
  const idx = indexPath(albumPath);
  if (!existsSync(idx)) {
    console.error(`✗ No index.md found for album "${albumPath}"`);
    process.exitCode = 1;
    return;
  }
  const parsed = matter(readFileSync(idx, 'utf-8'));
  if (remove) {
    delete parsed.data.shareToken;
    writeFileSync(idx, matter.stringify(parsed.content, parsed.data));
    console.log(`✓ Removed share token from "${albumPath}"`);
  } else {
    parsed.data.shareToken = generateShareToken();
    writeFileSync(idx, matter.stringify(parsed.content, parsed.data));
    console.log(`✓ ${albumPath}`);
    console.log(`  Share link: /photos/${albumPath}?token=${parsed.data.shareToken}`);
  }
}

const args = process.argv.slice(2);

if (args.length === 0 || args.includes('--help')) {
  console.log('Usage: node scripts/add-share-token.mjs <album-path> [...more] | --remove <album-path> | --list');
  process.exit(args.length === 0 ? 1 : 0);
}

if (args[0] === '--list') {
  const found = listAlbumsWithTokens();
  if (found.length === 0) {
    console.log('No albums with share tokens.');
  } else {
    for (const { path, hasPassword } of found) {
      console.log(`${path}${hasPassword ? '  (also has password)' : '  (link-only)'}`);
    }
  }
} else if (args[0] === '--remove') {
  if (!args[1]) {
    console.error('✗ --remove requires an album path');
    process.exit(1);
  }
  args.slice(1).forEach(p => updateToken(p, true));
} else {
  args.forEach(p => updateToken(p, false));
}
