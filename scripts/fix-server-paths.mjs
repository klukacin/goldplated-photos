import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const ENTRY_FILE = path.join(__dirname, '../dist/server/entry.mjs');
const REMOTE_ROOT = '/home/klukacincom/public_html';

console.log(`Fixing paths in ${ENTRY_FILE}...`);

if (!fs.existsSync(ENTRY_FILE)) {
  console.error('Error: dist/server/entry.mjs not found. Run build first.');
  process.exit(1);
}

let content = fs.readFileSync(ENTRY_FILE, 'utf8');

// Find the lines with "client": "file://..." and "server": "file://..."
// We use a regex to replace the value
const clientRegex = /"client":\s*"file:\/\/[^"]+"/g;
const serverRegex = /"server":\s*"file:\/\/[^"]+"/g;

const newClient = `"client": "file://${REMOTE_ROOT}/client/"`;
const newServer = `"server": "file://${REMOTE_ROOT}/server/"`;

let changed = false;

if (clientRegex.test(content)) {
  content = content.replace(clientRegex, newClient);
  changed = true;
  console.log('Updated client path.');
} else {
  console.warn('Warning: Could not find client path pattern.');
}

if (serverRegex.test(content)) {
  content = content.replace(serverRegex, newServer);
  changed = true;
  console.log('Updated server path.');
} else {
  console.warn('Warning: Could not find server path pattern.');
}

if (changed) {
  fs.writeFileSync(ENTRY_FILE, content, 'utf8');
  console.log('Successfully updated entry.mjs');
} else {
  console.log('No changes made to entry.mjs');
}
