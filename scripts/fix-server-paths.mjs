import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const ENTRY_FILE = path.join(__dirname, '../dist/server/entry.mjs');

// Remote root comes from the environment (deploy.sh exports it from .env).
const REMOTE_ROOT = process.env.DEPLOY_REMOTE_ROOT;
if (!REMOTE_ROOT) {
  console.error('Error: DEPLOY_REMOTE_ROOT is not set. Configure it in .env (see .env.example).');
  process.exit(1);
}

console.log(`Fixing paths in ${ENTRY_FILE} (remote root: ${REMOTE_ROOT})...`);

if (!fs.existsSync(ENTRY_FILE)) {
  console.error('Error: dist/server/entry.mjs not found. Run build first.');
  process.exit(1);
}

let content = fs.readFileSync(ENTRY_FILE, 'utf8');

// Find the lines with "client": "file://..." and "server": "file://..."
// We use a regex to replace the value
const newClient = `"client": "file://${REMOTE_ROOT}/client/"`;
const newServer = `"server": "file://${REMOTE_ROOT}/server/"`;

let changed = false;

const afterClient = content.replace(/"client":\s*"file:\/\/[^"]+"/g, newClient);
if (afterClient !== content) {
  content = afterClient;
  changed = true;
  console.log('Updated client path.');
} else {
  console.warn('Warning: Could not find client path pattern.');
}

const afterServer = content.replace(/"server":\s*"file:\/\/[^"]+"/g, newServer);
if (afterServer !== content) {
  content = afterServer;
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
