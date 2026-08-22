/**
 * The admin panel binds to loopback, so the internet cannot reach it — but any
 * web page the photographer happens to be visiting can. `cors()` does not stop
 * that: it only decides who may *read* a response, and the request has already
 * run by then. Requests a browser sends without a preflight (a GET of any kind,
 * a multipart POST) therefore land on the admin API with full authority.
 *
 * These tests drive a real server over HTTP with the headers a browser would
 * attach, and hold it to rejecting anything a page on another origin sends.
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const PROJECT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

let server: ChildProcess;
let base: string;

/** Ask the OS for a port nobody is using, so a running admin panel is unharmed. */
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

beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  server = spawn(process.execPath, ['admin/server.js'], {
    cwd: PROJECT_ROOT,
    env: { ...process.env, ADMIN_PORT: String(port) },
    stdio: 'ignore'
  });
  await waitForServer(`${base}/api/config`);
}, 30000);

afterAll(() => {
  server?.kill();
});

/** The headers a browser attaches to a request made from a page on evil.example. */
const CROSS_SITE = {
  'Sec-Fetch-Site': 'cross-site',
  'Sec-Fetch-Mode': 'no-cors',
  'Origin': 'https://evil.example'
};

describe('admin API cross-site requests', () => {
  it('refuses to reach the script runner from another origin', async () => {
    // An <img src="http://localhost:4444/api/tools/run/deploy"> on any page is
    // a plain GET: no preflight, no CORS check before the handler runs. A 404
    // here would mean the handler was entered and a real id would have spawned
    // `npm run deploy` against the photographer's production server.
    const res = await fetch(`${base}/api/tools/run/__no-such-script__`, { headers: CROSS_SITE });
    expect(res.status).toBe(403);
  });

  it('refuses to list albums to another origin', async () => {
    const res = await fetch(`${base}/api/albums`, { headers: CROSS_SITE });
    expect(res.status).toBe(403);
  });

  it('refuses a cross-site upload', async () => {
    // Multipart is a "simple request" — the browser sends it without asking.
    const form = new FormData();
    form.append('photos', new Blob(['not a photo']), 'evil.txt');
    const res = await fetch(`${base}/api/photos/anywhere`, {
      method: 'POST',
      headers: { 'Sec-Fetch-Site': 'cross-site', 'Origin': 'https://evil.example' },
      body: form
    });
    expect(res.status).toBe(403);
  });

  it('refuses a request that only carries a foreign Origin', async () => {
    // Browsers too old for fetch metadata still send Origin on CORS requests.
    const res = await fetch(`${base}/api/albums`, {
      headers: { 'Origin': 'https://evil.example' }
    });
    expect(res.status).toBe(403);
  });
});

describe('admin API same-origin requests', () => {
  it('serves the admin panel its own API', async () => {
    const res = await fetch(`${base}/api/albums`, {
      headers: { 'Sec-Fetch-Site': 'same-origin', 'Origin': base }
    });
    expect(res.status).toBe(200);
  });

  it('serves a request the user typed into the address bar', async () => {
    const res = await fetch(`${base}/api/config`, {
      headers: { 'Sec-Fetch-Site': 'none' }
    });
    expect(res.status).toBe(200);
  });

  it('serves a client that is not a browser at all', async () => {
    // curl and the like send neither header; they are not a CSRF vector.
    const res = await fetch(`${base}/api/config`);
    expect(res.status).toBe(200);
  });
});
