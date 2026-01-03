import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import crypto from 'crypto';
import matter from 'gray-matter';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const ALBUMS_DIR = path.join(__dirname, '../src/content/albums');

function generateToken(albumPath) {
  return crypto.createHash('sha256')
    .update(albumPath)
    .digest('hex')
    .substring(0, 12);
}

function findIndexFiles(dir, baseDir = dir) {
  const files = [];
  const items = fs.readdirSync(dir, { withFileTypes: true });

  for (const item of items) {
    const fullPath = path.join(dir, item.name);

    // Skip .meta directories
    if (item.name === '.meta') continue;

    if (item.isDirectory()) {
      files.push(...findIndexFiles(fullPath, baseDir));
    } else if (item.name === 'index.md') {
      const relativePath = path.relative(baseDir, dir);
      files.push({ fullPath, relativePath });
    }
  }

  return files;
}

function updateIndexFile(filePath, albumPath) {
  const content = fs.readFileSync(filePath, 'utf8');
  const { data, content: markdownContent } = matter(content);

  // Generate token if not present
  if (!data.token) {
    data.token = generateToken(albumPath);
    console.log(`  Generated token for ${albumPath}: ${data.token}`);
  } else {
    console.log(`  Token already exists for ${albumPath}: ${data.token}`);
  }

  // Add allowAnonymous if not present
  if (data.allowAnonymous === undefined) {
    data.allowAnonymous = true;
    console.log(`  Set allowAnonymous=true for ${albumPath}`);
  }

  // Write back to file
  const updatedContent = matter.stringify(markdownContent, data);
  fs.writeFileSync(filePath, updatedContent, 'utf8');
}

console.log('🔐 Generating tokens for all albums...\n');

const indexFiles = findIndexFiles(ALBUMS_DIR);

console.log(`Found ${indexFiles.length} index.md files\n`);

for (const { fullPath, relativePath } of indexFiles) {
  const albumPath = relativePath || 'root';
  console.log(`Processing: ${albumPath}`);
  updateIndexFile(fullPath, relativePath);
  console.log('');
}

console.log('✅ Token generation complete!');
console.log(`\nProcessed ${indexFiles.length} albums`);
console.log('\nNext steps:');
console.log('1. Review the updated index.md files');
console.log('2. Commit the changes to git');
console.log('3. Continue with Phase 2 implementation');
