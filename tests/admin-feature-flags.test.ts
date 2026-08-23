/**
 * The admin server must read the same feature flags as the gallery — that is
 * the whole point of src/site-features.mjs. With FEATURE_HEIC=0 the ingest
 * list shrinks, so the upload filter has to refuse a .heic album photo and
 * /api/config has to hand the browser code the shrunken list (admin/js are
 * classic <script>s; /api/config is how computed values reach them).
 *
 * Like tests/admin-image-formats.test.ts, this drives a real admin server
 * over HTTP with real image bytes.
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs/promises';
import path from 'node:path';
import sharp from 'sharp';
import {
  resolveFeatures,
  imageExtensionsFor,
  BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS,
} from '../src/site-features.mjs';

const PROJECT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const ALBUM_NAME = '__feature-flag-test-album__';
const ALBUM_DIR = path.join(PROJECT_ROOT, 'src/content/albums', ALBUM_NAME);

let server: ChildProcess;
let base: string;
let heic: Buffer;
let webp: Buffer;

function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const probe = createServer();
    probe.on('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const port = (probe.address() as { port: number }).port;
      probe.close(() => resolvePort(port));
    });
  });
}

async function waitForServer(url: string, timeoutMs = 20000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise(r => setTimeout(r, 100));
    }
  }
  throw new Error(`admin server did not start at ${url}`);
}

/** Same-origin headers, so the fetch-metadata guard lets the request through. */
function sameOrigin() {
  return { 'Sec-Fetch-Site': 'same-origin', 'Origin': base };
}

async function upload(endpoint: string, field: string, filename: string, bytes: Buffer) {
  const form = new FormData();
  form.append(field, new Blob([new Uint8Array(bytes)]), filename);
  return fetch(`${base}${endpoint}`, { method: 'POST', headers: sameOrigin(), body: form });
}

beforeAll(async () => {
  const canvas = { create: { width: 64, height: 48, channels: 3 as const, background: { r: 40, g: 120, b: 200 } } };
  heic = await sharp(canvas).heif({ compression: 'av1' }).toBuffer();
  webp = await sharp(canvas).webp().toBuffer();

  await fs.mkdir(ALBUM_DIR, { recursive: true });
  await fs.writeFile(path.join(ALBUM_DIR, 'index.md'), '---\ntitle: "Flag test"\nhidden: true\n---\n');

  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  server = spawn(process.execPath, ['admin/server.js'], {
    cwd: PROJECT_ROOT,
    env: { ...process.env, ADMIN_PORT: String(port), FEATURE_HEIC: '0' },
    stdio: 'ignore'
  });
  await waitForServer(`${base}/api/config`);
}, 60000);

afterAll(async () => {
  server?.kill();
  await fs.rm(ALBUM_DIR, { recursive: true, force: true });
});

describe('admin server with FEATURE_HEIC=0', () => {
  it('refuses a HEIC album photo — the ingest list shrank', async () => {
    const res = await upload(`/api/photos/${ALBUM_NAME}`, 'photos', 'iphone.heic', heic);
    expect(res.status).toBe(400);
    await expect(fs.access(path.join(ALBUM_DIR, 'iphone.heic'))).rejects.toThrow();
  });

  it('still accepts a WebP album photo — only HEIC/HEIF left the list', async () => {
    const res = await upload(`/api/photos/${ALBUM_NAME}`, 'photos', 'shot.webp', webp);
    expect(res.status).toBe(200);
    expect((await res.json()).uploaded).toContain('shot.webp');
  });

  it('serves the shrunken list to the admin frontend via /api/config', async () => {
    const res = await fetch(`${base}/api/config`, { headers: sameOrigin() });
    expect(res.status).toBe(200);
    const config = await res.json();
    expect(config.imageExtensions).toEqual(
      imageExtensionsFor(resolveFeatures({ FEATURE_HEIC: '0' }))
    );
    expect(config.imageExtensions).not.toContain('.heic');
    expect(config.imageExtensions).not.toContain('.heif');
    expect(config.browserDisplayableImageExtensions).toEqual(BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS);
  });
});
