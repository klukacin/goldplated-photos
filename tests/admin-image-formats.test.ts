/**
 * The admin panel is where a HEIC enters the site, and the two destinations do
 * not have the same rules.
 *
 * An album photo may be HEIC: every URL the gallery builds for it goes through
 * /api/thumbnail, which converts to JPEG or WebP. A public asset — hero slide,
 * home card, landing background — may not: those files are served raw out of
 * public/, no conversion exists anywhere in the chain, and Chrome and Firefox
 * cannot decode HEIC. Accepting one there means a permanently broken hero
 * slider that looks perfect on the photographer's Safari.
 *
 * WebP is fine everywhere and must not be rejected by an over-tight filter.
 *
 * These tests drive a real admin server over HTTP with real image bytes.
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs/promises';
import path from 'node:path';
import sharp from 'sharp';

const PROJECT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const ALBUM_NAME = '__format-test-album__';
const ALBUM_DIR = path.join(PROJECT_ROOT, 'src/content/albums', ALBUM_NAME);
const HERO_DIR = path.join(PROJECT_ROOT, 'public/home/hero');
const LANDING_BG = path.join(PROJECT_ROOT, 'public/images/landing-bg.jpg');

let server: ChildProcess;
let base: string;
let heic: Buffer;
let webp: Buffer;
/** Hero files the tests created, removed again in afterAll. */
const heroLeftovers: string[] = [];
/**
 * The landing endpoint renames whatever it accepts over public/images/landing-bg.jpg.
 * Held here and put back in afterAll, so a regression that lets a HEIC through
 * does not also destroy the photographer's landing background.
 */
let landingBackup: Buffer | null = null;

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
  // Real files, not stubs — an extension check that happened to pass on empty
  // bytes would prove nothing about the pipeline behind it.
  const canvas = { create: { width: 64, height: 48, channels: 3 as const, background: { r: 200, g: 120, b: 40 } } };
  heic = await sharp(canvas).heif({ compression: 'av1' }).toBuffer();
  webp = await sharp(canvas).webp().toBuffer();

  landingBackup = await fs.readFile(LANDING_BG).catch(() => null);

  // A crashed earlier run skips afterAll and leaves its uploads behind — and
  // *.heic is gitignored, so git status shows a clean tree over a dirty one.
  // The refusal tests assert the file does NOT exist, so a stale one fails
  // them forever until someone thinks to look on disk. Start clean instead.
  await fs.rm(path.join(HERO_DIR, 'sunset.heic'), { force: true });
  await fs.rm(path.join(PROJECT_ROOT, 'public/home/cards/card.heic'), { force: true });

  await fs.mkdir(ALBUM_DIR, { recursive: true });
  await fs.writeFile(path.join(ALBUM_DIR, 'index.md'), '---\ntitle: "Format test"\nhidden: true\n---\n');

  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  server = spawn(process.execPath, ['admin/server.js'], {
    cwd: PROJECT_ROOT,
    env: { ...process.env, ADMIN_PORT: String(port) },
    stdio: 'ignore'
  });
  await waitForServer(`${base}/api/config`);
}, 60000);

afterAll(async () => {
  server?.kill();
  await fs.rm(ALBUM_DIR, { recursive: true, force: true });
  for (const name of heroLeftovers) {
    await fs.rm(path.join(HERO_DIR, name), { force: true });
  }
  if (landingBackup) {
    await fs.writeFile(LANDING_BG, landingBackup);
  } else {
    await fs.rm(LANDING_BG, { force: true });
  }
});

describe('album photo uploads', () => {
  it('accepts a HEIC — the gallery only ever shows it through /api/thumbnail', async () => {
    const res = await upload(`/api/photos/${ALBUM_NAME}`, 'photos', 'iphone.heic', heic);
    expect(res.status).toBe(200);
    expect((await res.json()).uploaded).toContain('iphone.heic');
  });

  it('accepts a WebP', async () => {
    const res = await upload(`/api/photos/${ALBUM_NAME}`, 'photos', 'shot.webp', webp);
    expect(res.status).toBe(200);
    expect((await res.json()).uploaded).toContain('shot.webp');
  });

  it('lists both back, so neither is invisible to the album editor', async () => {
    const res = await fetch(`${base}/api/photos/${ALBUM_NAME}`, { headers: sameOrigin() });
    expect(res.status).toBe(200);
    const names = (await res.json()).map((p: { filename: string }) => p.filename);
    expect(names).toContain('iphone.heic');
    expect(names).toContain('shot.webp');
  });
});

describe('public asset uploads', () => {
  it('refuses a HEIC hero slide — public/ is served raw, so it could never render', async () => {
    const res = await upload('/api/assets/hero', 'image', 'sunset.heic', heic);
    expect(res.status).toBe(400);
    expect((await res.json()).error).toMatch(/heic/i);
    await expect(fs.access(path.join(HERO_DIR, 'sunset.heic'))).rejects.toThrow();
  });

  it('refuses a HEIC home card image', async () => {
    const res = await upload('/api/assets/cards', 'image', 'card.heic', heic);
    expect(res.status).toBe(400);
  });

  it('refuses a HEIC landing background', async () => {
    // Worse than the others: this endpoint renames whatever arrives to
    // landing-bg.jpg, so a HEIC would sit there mislabelled as a JPEG.
    const res = await upload('/api/assets/landing', 'image', 'bg.heic', heic);
    expect(res.status).toBe(400);
  });

  it('accepts a WebP hero slide', async () => {
    heroLeftovers.push('__format-test__.webp');
    const res = await upload('/api/assets/hero', 'image', '__format-test__.webp', webp);
    expect(res.status).toBe(200);
  });
});
