import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { APIContext, APIRoute } from 'astro';
import { mkdir, readdir, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

/**
 * The sync endpoints are the one door through which a network client writes
 * into the published gallery. Everything here holds a rule that keeps that door
 * from becoming a way to write anywhere on the server, or to publish bytes that
 * are not the bytes the photographer sent.
 *
 * `CONTENT_ROOT` is resolved from `process.cwd()` when `_shared` loads, which
 * would aim these tests at the repo's real albums. Redirecting that one module
 * is the smallest available seam and leaves the endpoints untouched: both
 * endpoints and the hash cache read the root from here, so redirecting it moves
 * the whole subsystem into a temp directory.
 */
const { CONTENT_ROOT, FULL_SYNC_ROOT } = await vi.hoisted(async () => {
  const { mkdtemp } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join: joinPath } = await import('node:path');
  const base = await mkdtemp(joinPath(tmpdir(), 'gpp-sync-endpoints-'));
  return {
    CONTENT_ROOT: joinPath(base, 'albums'),
    FULL_SYNC_ROOT: joinPath(base, '.sync-full'),
  };
});

vi.mock('../src/pages/api/sync/_shared', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/pages/api/sync/_shared')>();
  return { ...actual, CONTENT_ROOT, FULL_SYNC_ROOT };
});

import { checkSyncAuth, safeRelPath, safeScope, splitSyncPath } from '../src/lib/sync-auth';
import { useCacheFileForTests } from '../src/pages/api/sync/_hash-cache';
import { blake3HexOf } from '../src/pages/api/sync/_shared';
import { GET as fileGET, PUT as filePUT, DELETE as fileDELETE } from '../src/pages/api/sync/file';
import { GET as manifestGET } from '../src/pages/api/sync/manifest';

const TOKEN = 'sync-token-at-least-16-chars';

/** Astro hands routes a full APIContext; the sync endpoints read only these two. */
function call(route: APIRoute, request: unknown, url: URL): Promise<Response> {
  return Promise.resolve(route({ request, url } as unknown as APIContext)) as Promise<Response>;
}

function authorized(token: string | null = TOKEN, extra: Record<string, string> = {}): Headers {
  const headers = new Headers(extra);
  if (token !== null) headers.set('authorization', `Bearer ${token}`);
  return headers;
}

const fileUrl = (path: string) =>
  new URL(`http://localhost/api/sync/file?path=${encodeURIComponent(path)}`);

const manifestUrl = (scope?: string) =>
  new URL(
    scope === undefined
      ? 'http://localhost/api/sync/manifest'
      : `http://localhost/api/sync/manifest?scope=${encodeURIComponent(scope)}`
  );

const bytesOf = (text: string) => new TextEncoder().encode(text);
const hashOf = (text: string) => blake3HexOf(bytesOf(text));

/** PUT `text` to `path`, declaring `hash` (by default the honest one). */
function upload(path: string, text: string, hash: string | null = hashOf(text)) {
  const headers = authorized();
  if (hash !== null) headers.set('x-content-blake3', hash);
  const request = new Request(fileUrl(path), { method: 'PUT', headers, body: bytesOf(text) });
  return call(filePUT, request, fileUrl(path));
}

async function manifest(scope?: string): Promise<Array<{ path: string; hash: string }>> {
  const url = manifestUrl(scope);
  const response = await call(manifestGET, new Request(url, { headers: authorized() }), url);
  expect(response.status).toBe(200);
  return (await response.json()).files;
}

async function exists(path: string): Promise<boolean> {
  return stat(path).then(
    () => true,
    () => false
  );
}

async function listing(dir: string): Promise<string[]> {
  return readdir(dir).catch(() => [] as string[]);
}

beforeEach(async () => {
  process.env.SYNC_TOKEN = TOKEN;
  await rm(CONTENT_ROOT, { recursive: true, force: true });
  await rm(FULL_SYNC_ROOT, { recursive: true, force: true });
  await mkdir(CONTENT_ROOT, { recursive: true });
  // The cache keeps its map in module state; this points it at the fresh root
  // and starts it cold, so no test inherits another's hashes.
  useCacheFileForTests(join(CONTENT_ROOT, '.sync-hashes.json'));
});

afterAll(async () => {
  delete process.env.SYNC_TOKEN;
  await rm(CONTENT_ROOT, { recursive: true, force: true });
  await rm(FULL_SYNC_ROOT, { recursive: true, force: true });
});

describe('sync authentication', () => {
  it('refuses every request when SYNC_TOKEN is unset, instead of defaulting to open', () => {
    // The property that matters most: a server that was never configured for
    // sync must be closed. If this ever returns ok, every deployment that has
    // not set SYNC_TOKEN is world-writable.
    delete process.env.SYNC_TOKEN;
    expect(checkSyncAuth(new Request('http://localhost/', { headers: authorized() }))).toMatchObject({
      ok: false,
      status: 503,
    });
  });

  it('refuses when SYNC_TOKEN is too short to be a real secret', () => {
    // 15 characters is guessable; refusing is safer than accepting a token a
    // brute-force run would reach.
    process.env.SYNC_TOKEN = 'x'.repeat(15);
    const request = new Request('http://localhost/', {
      headers: authorized('x'.repeat(15)),
    });
    expect(checkSyncAuth(request)).toMatchObject({ ok: false, status: 503 });
  });

  it('accepts exactly the configured token', () => {
    expect(checkSyncAuth(new Request('http://localhost/', { headers: authorized() }))).toEqual({
      ok: true,
    });
  });

  it('rejects a wrong token of the same length with 401', () => {
    const wrong = `${TOKEN.slice(0, -1)}X`;
    expect(wrong).toHaveLength(TOKEN.length);
    expect(checkSyncAuth(new Request('http://localhost/', { headers: authorized(wrong) }))).toMatchObject({
      ok: false,
      status: 401,
    });
  });

  it('rejects a wrong token of a different length without throwing', () => {
    // node's timingSafeEqual throws on a length mismatch. A comparison that
    // forgets to guard that turns a short token into a 500 — and a 500 that
    // differs from a 401 is itself an oracle for the secret's length.
    for (const wrong of ['short', `${TOKEN}-and-then-some-more`, 'x'.repeat(4096)]) {
      expect(() =>
        checkSyncAuth(new Request('http://localhost/', { headers: authorized(wrong) }))
      ).not.toThrow();
      expect(checkSyncAuth(new Request('http://localhost/', { headers: authorized(wrong) }))).toMatchObject({
        ok: false,
        status: 401,
      });
    }
  });

  it('rejects a request with no Authorization header at all', () => {
    expect(checkSyncAuth(new Request('http://localhost/'))).toMatchObject({ ok: false, status: 401 });
  });

  it('rejects an Authorization header that is not a Bearer token', () => {
    for (const header of [TOKEN, `Basic ${TOKEN}`, `bearer ${TOKEN}`, 'Bearer ']) {
      const request = new Request('http://localhost/', { headers: { authorization: header } });
      expect(checkSyncAuth(request)).toMatchObject({ ok: false, status: 401 });
    }
  });

  it('closes every sync endpoint to an unauthenticated caller', async () => {
    const cases: Array<[string, APIRoute, string]> = [
      ['manifest GET', manifestGET, 'GET'],
      ['file GET', fileGET, 'GET'],
      ['file PUT', filePUT, 'PUT'],
      ['file DELETE', fileDELETE, 'DELETE'],
    ];
    for (const [name, route, method] of cases) {
      const url = fileUrl('a/b.jpg');
      const response = await call(route, new Request(url, { method }), url);
      expect(response.status, name).toBe(401);
    }
  });

  it('answers a misconfigured server before it looks at the path', async () => {
    // Ordering matters: a 400 here would tell an unauthenticated caller which
    // paths the server considers valid.
    delete process.env.SYNC_TOKEN;
    const url = manifestUrl('../escape');
    const response = await call(manifestGET, new Request(url, { headers: authorized() }), url);
    expect(response.status).toBe(503);
  });
});

describe('safeRelPath', () => {
  it('accepts ordinary nested album paths', () => {
    for (const path of [
      'photo.jpg',
      '2025/wedding/photo.jpg',
      '2025/wedding/index.md',
      'a/b/c/d/e/f.txt',
      '2025/wedding/photo.final.v2.jpg',
      'ime i prezime/foto.jpg',
      'x'.repeat(512),
    ]) {
      expect(safeRelPath(path), path).toBe(path);
    }
  });

  it('rejects anything that could reach outside the album tree', () => {
    // Each of these, if accepted, is an arbitrary-file-write on the server:
    // path.join(CONTENT_ROOT, rel) would land somewhere else entirely.
    for (const path of [
      '/etc/passwd',
      '/',
      '..',
      '../secret',
      '../../etc/passwd',
      '2025/../../etc/passwd',
      '2025/wedding/..',
      'a/../b',
      '.',
      'a/./b',
    ]) {
      expect(safeRelPath(path), path).toBeNull();
    }
  });

  it('rejects backslashes, which are separators on Windows', () => {
    // On a Windows host `..\..\x` escapes the root exactly as `../../x` does,
    // and the segment split below would never see it.
    for (const path of ['a\\b', '..\\..\\x', 'a/b\\..\\c', '\\etc\\passwd']) {
      expect(safeRelPath(path), path).toBeNull();
    }
  });

  it('rejects NUL bytes, which truncate the path inside the C library', () => {
    // "a.jpg\0/../../etc/passwd" reads as harmless to JavaScript and as
    // "a.jpg" to a syscall — the classic poison-null-byte bypass.
    for (const path of ['a\0b', 'photo.jpg\0.txt', '\0']) {
      expect(safeRelPath(path), path).toBeNull();
    }
  });

  it('rejects dotfiles at any depth, keeping .meta server-owned', () => {
    // `.meta/proofing/` holds client selections the server writes and the
    // desktop app must never overwrite or delete; `.sync-hashes.json` is the
    // server's own cache. Both are dotfiles, and that is the whole defence.
    for (const path of [
      '.env',
      '.sync-hashes.json',
      'a/.meta/x.json',
      '2025/wedding/.meta/proofing/sub.json',
      'a/b/.hidden',
      '.meta',
      'a/.meta',
    ]) {
      expect(safeRelPath(path), path).toBeNull();
    }
  });

  it('rejects empty paths and empty segments', () => {
    for (const path of ['', 'a//b', 'a/', '/a', '//']) {
      expect(safeRelPath(path), path).toBeNull();
    }
    expect(safeRelPath(null)).toBeNull();
  });

  it('rejects a path over the length cap, bounding work before any syscall', () => {
    expect(safeRelPath('x'.repeat(513))).toBeNull();
    expect(safeRelPath(`${'a/'.repeat(300)}b.jpg`)).toBeNull();
  });
});

describe('safeScope', () => {
  it('treats an absent or empty scope as "the whole tree"', () => {
    expect(safeScope(null)).toBeUndefined();
    expect(safeScope('')).toBeUndefined();
  });

  it('accepts a nested scope and applies the same rules as a file path', () => {
    expect(safeScope('2025/wedding')).toBe('2025/wedding');
    for (const scope of ['../..', '/etc', '.meta', 'a/../b', 'a\\b', 'a\0b']) {
      expect(safeScope(scope), scope).toBeNull();
    }
  });
});

describe('sync manifest', () => {
  it('reports an empty manifest for a scope that does not exist yet', async () => {
    // This is what a first push looks like. An error here would make the very
    // first sync of a new album impossible.
    expect(await manifest('2025/not-created-yet')).toEqual([]);
  });

  it('lists every file with its real BLAKE3 hash, relative to the album root', async () => {
    await mkdir(join(CONTENT_ROOT, '2025/wedding'), { recursive: true });
    await writeFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'alpha');
    await writeFile(join(CONTENT_ROOT, '2025/wedding/index.md'), 'title');
    await writeFile(join(CONTENT_ROOT, 'top.txt'), 'top level');

    const files = await manifest();
    expect(new Map(files.map((f) => [f.path, f.hash]))).toEqual(
      new Map([
        ['2025/wedding/a.jpg', hashOf('alpha')],
        ['2025/wedding/index.md', hashOf('title')],
        ['top.txt', hashOf('top level')],
      ])
    );
  });

  it('narrows the listing to the requested scope', async () => {
    await mkdir(join(CONTENT_ROOT, '2025/wedding'), { recursive: true });
    await mkdir(join(CONTENT_ROOT, '2024/portraits'), { recursive: true });
    await writeFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'alpha');
    await writeFile(join(CONTENT_ROOT, '2024/portraits/b.jpg'), 'beta');

    expect((await manifest('2025')).map((f) => f.path)).toEqual(['2025/wedding/a.jpg']);
  });

  it('never lists dotfiles or dot-directories, including its own hash cache', async () => {
    // Anything the manifest lists is something the client believes it owns and
    // may delete. `.meta/proofing/` is client selections the gallery wrote and
    // the desktop app has never seen — listing it would sync them away.
    await mkdir(join(CONTENT_ROOT, '2025/wedding/.meta/proofing'), { recursive: true });
    await writeFile(join(CONTENT_ROOT, '2025/wedding/.meta/proofing/sub.json'), '{}');
    await writeFile(join(CONTENT_ROOT, '2025/wedding/.DS_Store'), 'junk');
    await writeFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'alpha');

    // Run once to make the cache write itself into the root, then look again:
    // `.sync-hashes.json` must not show up in the second listing.
    expect((await manifest()).map((f) => f.path)).toEqual(['2025/wedding/a.jpg']);
    expect(await exists(join(CONTENT_ROOT, '.sync-hashes.json'))).toBe(true);
    expect((await manifest()).map((f) => f.path)).toEqual(['2025/wedding/a.jpg']);
  });

  it('rejects an unsafe scope with 400 rather than walking it', async () => {
    const url = manifestUrl('../../etc');
    const response = await call(manifestGET, new Request(url, { headers: authorized() }), url);
    expect(response.status).toBe(400);
  });

  it('never follows a symlink out of the album tree', async () => {
    // readdir reports a symlink as neither file nor directory, so the walk
    // skips it. If it ever started following them, a single link dropped in an
    // album would publish the hash of anything on the box.
    await mkdir(join(CONTENT_ROOT, 'real'), { recursive: true });
    await writeFile(join(CONTENT_ROOT, 'real/a.jpg'), 'alpha');
    await symlink('/etc/passwd', join(CONTENT_ROOT, 'leak.txt'));
    await symlink('/etc', join(CONTENT_ROOT, 'leakdir'));

    expect((await manifest()).map((f) => f.path)).toEqual(['real/a.jpg']);
  });

  it('answers a scope that names a file with 400, not a 500', async () => {
    // readdir raises ENOTDIR here, and only ENOENT means "empty manifest". A
    // client mistake must read as a client mistake.
    await writeFile(join(CONTENT_ROOT, 'a.jpg'), 'alpha');
    const url = manifestUrl('a.jpg');
    const response = await call(manifestGET, new Request(url, { headers: authorized() }), url);
    expect(response.status).toBe(400);
    expect((await response.json()).error).toBe('Scope is not a folder');
  });
});

describe('sync file upload', () => {
  it('writes the exact bytes sent when the declared hash matches', async () => {
    const response = await upload('2025/wedding/a.jpg', 'alpha');
    expect(response.status).toBe(204);
    expect(await readFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'utf8')).toBe('alpha');
  });

  it('creates the album directory on the way, so a first push needs no setup', async () => {
    expect((await upload('2025/new-album/a.jpg', 'alpha')).status).toBe(204);
    expect(await exists(join(CONTENT_ROOT, '2025/new-album/a.jpg'))).toBe(true);
  });

  it('refuses a body whose hash does not match, and leaves nothing behind', async () => {
    // A published file that is not the file the photographer sent is worse than
    // a failed sync: the client believes it succeeded and never retries.
    const response = await upload('2025/wedding/a.jpg', 'alpha', hashOf('something else'));
    expect(response.status).toBe(422);
    expect(await exists(join(CONTENT_ROOT, '2025/wedding/a.jpg'))).toBe(false);
  });

  it('leaves a previously published file untouched when an upload is rejected', async () => {
    await upload('2025/wedding/a.jpg', 'good');
    expect((await upload('2025/wedding/a.jpg', 'corrupt', hashOf('good'))).status).toBe(422);
    expect(await readFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'utf8')).toBe('good');
  });

  it('leaves no .upload- temp file behind, on either the accepted or rejected path', async () => {
    // The write goes to a sibling and is renamed into place. A temp file that
    // survives is a half-written photo sitting in a published album.
    await upload('2025/wedding/a.jpg', 'alpha');
    await upload('2025/wedding/b.jpg', 'beta', hashOf('not beta'));
    const left = (await listing(join(CONTENT_ROOT, '2025/wedding'))).filter((n) =>
      n.includes('.upload-')
    );
    expect(left).toEqual([]);
  });

  it('refuses an oversized Content-Length with 413 without reading the body', async () => {
    // The honest case, and the cheap one: the client says how big it is and is
    // turned away before a byte is buffered. arrayBuffer() throws here so that
    // a regression to measuring-after-reading shows up as a failure rather
    // than as a test that quietly got slower.
    const request = {
      headers: authorized(undefined, {
        'x-content-blake3': hashOf('irrelevant'),
        'content-length': String(512 * 1024 * 1024 + 1),
      }),
      arrayBuffer: async () => {
        throw new Error('body was read despite an oversized Content-Length');
      },
    };
    const response = await call(filePUT, request, fileUrl('big.bin'));
    expect(response.status).toBe(413);
    expect(await exists(join(CONTENT_ROOT, 'big.bin'))).toBe(false);
  });

  it('refuses an oversized body that declared no length, with 413', async () => {
    // The buffer is allocated but never written to, so the pages are never
    // faulted in: this costs address space, not 512 MB of memory.
    //
    // Note what this does and does not prove, because the Content-Length check
    // above does not make it obsolete. A client that lies about its length, or
    // omits it, still has its whole body buffered by arrayBuffer() before this
    // second check can measure it. The cap keeps oversized files out of the
    // gallery; it does not keep them out of memory.
    const oversized = new ArrayBuffer(512 * 1024 * 1024 + 1);
    const request = {
      headers: authorized(undefined, { 'x-content-blake3': hashOf('irrelevant') }),
      arrayBuffer: async () => oversized,
    };
    const response = await call(filePUT, request, fileUrl('big.bin'));
    expect(response.status).toBe(413);
    expect(await exists(join(CONTENT_ROOT, 'big.bin'))).toBe(false);
  });

  it('refuses to write through an unsafe path, and writes nothing anywhere', async () => {
    for (const path of ['../escaped.txt', '/etc/passwd', 'a/.meta/proofing/forged.json', '']) {
      const url = fileUrl(path);
      const request = new Request(url, {
        method: 'PUT',
        headers: authorized(undefined, { 'x-content-blake3': hashOf('x') }),
        body: bytesOf('x'),
      });
      expect((await call(filePUT, request, url)).status, path).toBe(400);
    }
    expect(await listing(CONTENT_ROOT)).toEqual([]);
  });

  it('serves back a published file and 404s one that was never pushed', async () => {
    await upload('2025/wedding/a.jpg', 'alpha');
    const found = await call(
      fileGET,
      new Request(fileUrl('2025/wedding/a.jpg'), { headers: authorized() }),
      fileUrl('2025/wedding/a.jpg')
    );
    expect(found.status).toBe(200);
    expect(await found.text()).toBe('alpha');

    const missing = await call(
      fileGET,
      new Request(fileUrl('2025/wedding/nope.jpg'), { headers: authorized() }),
      fileUrl('2025/wedding/nope.jpg')
    );
    expect(missing.status).toBe(404);
  });

  it('replaces a symlink rather than writing through it', async () => {
    // The write lands on a sibling and is renamed over the target, and rename
    // does not follow symlinks. If it ever became a plain write to `full`, a
    // symlink planted in an album would turn every upload into a write to
    // wherever that link points.
    const outside = join(CONTENT_ROOT, '..', `gpp-sync-outside-${process.pid}.txt`);
    await rm(outside, { force: true });
    await writeFile(outside, 'untouched');
    await symlink(outside, join(CONTENT_ROOT, 'sneaky.txt'));

    expect((await upload('sneaky.txt', 'payload')).status).toBe(204);
    expect(await readFile(outside, 'utf8')).toBe('untouched');
    expect(await readFile(join(CONTENT_ROOT, 'sneaky.txt'), 'utf8')).toBe('payload');
    await rm(outside, { force: true });
  });

  it('removes a published file on DELETE', async () => {
    await upload('2025/wedding/a.jpg', 'alpha');
    const url = fileUrl('2025/wedding/a.jpg');
    const response = await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url);
    expect(response.status).toBe(204);
    expect(await exists(join(CONTENT_ROOT, '2025/wedding/a.jpg'))).toBe(false);
  });

  it('prunes the album directory a delete emptied, but not one still in use', async () => {
    await upload('2025/wedding/a.jpg', 'alpha');
    await upload('2025/portraits/b.jpg', 'beta');
    const url = fileUrl('2025/wedding/a.jpg');
    await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url);

    expect(await exists(join(CONTENT_ROOT, '2025/wedding'))).toBe(false);
    expect(await exists(join(CONTENT_ROOT, '2025/portraits/b.jpg'))).toBe(true);
  });

  it('treats a delete of something already gone as done', async () => {
    const url = fileUrl('2025/wedding/never-existed.jpg');
    const response = await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url);
    expect(response.status).toBe(204);
  });

  it('answers a DELETE of a directory path with 400, not a 500', async () => {
    // `2025/wedding` is a well-formed path that safeRelPath accepts, and `rm`
    // without `recursive` raises EISDIR on it. Sync deletes files; a directory
    // goes when its last file does.
    await upload('2025/wedding/a.jpg', 'alpha');
    const url = fileUrl('2025/wedding');
    const response = await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url);
    expect(response.status).toBe(400);
    expect((await response.json()).error).toBe('Path is a directory');
    expect(await exists(join(CONTENT_ROOT, '2025/wedding/a.jpg'))).toBe(true);
  });

  it('never removes the album root when the last top-level file goes', async () => {
    // The tidy-up rmdir would otherwise be handed CONTENT_ROOT itself, and the
    // gallery's content collection has no directory left to read.
    await upload('a.jpg', 'alpha');
    const url = fileUrl('a.jpg');
    expect((await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url)).status).toBe(204);
    expect(await exists(CONTENT_ROOT)).toBe(true);
  });

  it('never removes the album root even when nothing is left in it at all', async () => {
    // The reliable trigger, and the one a delete-only sync actually takes:
    // forgetHash on a key that was never cached leaves `dirty` false, so the
    // flush writes no .sync-hashes.json and the root is genuinely empty at
    // rmdir time. Anything that relied on the cache file keeping the root
    // non-empty would pass the test above and still lose the tree here.
    await writeFile(join(CONTENT_ROOT, 'a.jpg'), 'alpha');
    const url = fileUrl('a.jpg');
    expect((await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url)).status).toBe(204);
    expect(await exists(CONTENT_ROOT)).toBe(true);
    expect(await listing(CONTENT_ROOT)).toEqual([]);
  });

  it('refuses an upload that declares no hash, and writes nothing', async () => {
    // Verification is not opt-in. If a missing header meant "write it
    // unverified", the 422 rule above would guard only the clients that chose
    // to be guarded, and a truncated upload would publish as a photo.
    const response = await upload('2025/wedding/a.jpg', 'alpha', null);
    expect(response.status).toBe(400);
    expect((await response.json()).error).toBe('Missing X-Content-Blake3');
    expect(await exists(join(CONTENT_ROOT, '2025/wedding/a.jpg'))).toBe(false);
  });

  it('accepts a declared hash in upper-case hex', async () => {
    // Hex has no case, and a client that spells it in capitals is not sending
    // corrupt bytes. Rejecting it would be a 422 that says "corruption".
    expect((await upload('2025/wedding/a.jpg', 'alpha', hashOf('alpha').toUpperCase())).status).toBe(204);
    expect(await readFile(join(CONTENT_ROOT, '2025/wedding/a.jpg'), 'utf8')).toBe('alpha');
  });
});

describe('sync upload and manifest agree', () => {
  it('reports an uploaded file with the hash the upload verified', async () => {
    await upload('2025/wedding/a.jpg', 'alpha');
    expect(await manifest()).toEqual([{ path: '2025/wedding/a.jpg', hash: hashOf('alpha') }]);
  });

  it('reports the new hash after a delete and a re-upload, never the stale one', async () => {
    // The cache is keyed on (size, mtime), so a same-length rewrite is the case
    // that would go wrong. A stale hash here tells the client the server
    // already holds a photo it does not, and that photo silently never arrives.
    await upload('2025/wedding/a.jpg', 'alpha');
    expect(await manifest()).toEqual([{ path: '2025/wedding/a.jpg', hash: hashOf('alpha') }]);

    const url = fileUrl('2025/wedding/a.jpg');
    await call(fileDELETE, new Request(url, { method: 'DELETE', headers: authorized() }), url);
    expect(await manifest()).toEqual([]);

    await upload('2025/wedding/a.jpg', 'ALPHA');
    expect(await manifest()).toEqual([{ path: '2025/wedding/a.jpg', hash: hashOf('ALPHA') }]);
  });

  it('reports the new hash when a file is overwritten in place', async () => {
    await upload('2025/wedding/a.jpg', 'alpha');
    await manifest();
    await upload('2025/wedding/a.jpg', 'omega');
    expect(await manifest()).toEqual([{ path: '2025/wedding/a.jpg', hash: hashOf('omega') }]);
  });
});

describe('splitSyncPath', () => {
  // Pure routing over already-validated paths: this is the function that
  // decides whether bytes land in the served gallery tree or the private
  // full-scope store, so its whole table is stated here.
  it('routes ordinary paths to the content tree, untouched', () => {
    for (const rel of ['2025/wedding/a.jpg', 'index.md', 'a/b/c.txt']) {
      expect(splitSyncPath(rel)).toEqual({ tree: 'content', rest: rel });
    }
  });

  it('routes the reserved prefix to the full tree, stripped', () => {
    expect(splitSyncPath('__gpp_full__/2025/w/a.nef')).toEqual({
      tree: 'full',
      rest: '2025/w/a.nef',
    });
    expect(splitSyncPath('__gpp_full__/2025/w/album.gpp.json')).toEqual({
      tree: 'full',
      rest: '2025/w/album.gpp.json',
    });
  });

  it('refuses the bare prefix — it names no file', () => {
    expect(splitSyncPath('__gpp_full__')).toBeNull();
  });

  it('reserves only a LEADING prefix; elsewhere it is an ordinary name', () => {
    expect(splitSyncPath('2025/__gpp_full__/a.jpg')).toEqual({
      tree: 'content',
      rest: '2025/__gpp_full__/a.jpg',
    });
    expect(splitSyncPath('__gpp_full__extra/a.jpg')).toEqual({
      tree: 'content',
      rest: '__gpp_full__extra/a.jpg',
    });
  });

  it('never sees an escape: safeRelPath refuses them first', () => {
    // The prefix grants nothing — a hostile manifest cannot ride it out of
    // either tree, because validation runs before routing.
    for (const hostile of [
      '__gpp_full__/../escape.txt',
      '__gpp_full__/2025/../../etc/passwd',
      '__gpp_full__/.meta/x.json',
      '__gpp_full__\\..\\x',
      '__gpp_full__/a\0b',
      '__gpp_full__/',
    ]) {
      expect(safeRelPath(hostile), hostile).toBeNull();
    }
  });
});

describe('full-scope sync storage', () => {
  const fullUpload = (rel: string, text: string) => upload(rel, text);

  it('stores a __gpp_full__ upload under the private root, never the content tree', async () => {
    const response = await fullUpload('__gpp_full__/2025/w/a.nef', 'raw bytes');
    expect(response.status).toBe(204);

    // In the private store, with the prefix stripped…
    expect(await readFile(join(FULL_SYNC_ROOT, '2025/w/a.nef'), 'utf8')).toBe('raw bytes');
    // …and nowhere under the tree the gallery builds and the deploy rsyncs.
    expect(await exists(join(CONTENT_ROOT, '__gpp_full__'))).toBe(false);
    expect(await exists(join(CONTENT_ROOT, '2025/w/a.nef'))).toBe(false);
  });

  it('serves a stored full-scope file back, and deletes it, by its prefixed name', async () => {
    await fullUpload('__gpp_full__/2025/w/a.nef', 'raw bytes');

    const url = fileUrl('__gpp_full__/2025/w/a.nef');
    const found = await call(fileGET, new Request(url, { headers: authorized() }), url);
    expect(found.status).toBe(200);
    expect(await found.text()).toBe('raw bytes');

    const gone = await call(
      fileDELETE,
      new Request(url, { method: 'DELETE', headers: authorized() }),
      url
    );
    expect(gone.status).toBe(204);
    expect(await exists(join(FULL_SYNC_ROOT, '2025/w/a.nef'))).toBe(false);
    // The private root itself survives an emptying delete.
    expect(await exists(FULL_SYNC_ROOT)).toBe(true);
  });

  it('verifies full-scope uploads against the declared hash like any other', async () => {
    const response = await upload('__gpp_full__/2025/w/a.nef', 'corrupt', hashOf('intact'));
    expect(response.status).toBe(422);
    expect(await exists(join(FULL_SYNC_ROOT, '2025/w/a.nef'))).toBe(false);
  });

  it('lists both trees in one manifest, full entries under their prefix', async () => {
    await upload('2025/w/a.jpg', 'web pixels');
    await fullUpload('__gpp_full__/2025/w/a.nef', 'raw bytes');
    await fullUpload('__gpp_full__/2025/w/album.gpp.json', '{"version":1}');

    const files = await manifest();
    expect(new Map(files.map((f) => [f.path, f.hash]))).toEqual(
      new Map([
        ['2025/w/a.jpg', hashOf('web pixels')],
        ['__gpp_full__/2025/w/a.nef', hashOf('raw bytes')],
        ['__gpp_full__/2025/w/album.gpp.json', hashOf('{"version":1}')],
      ])
    );
  });

  it('scopes a manifest to one tree at a time', async () => {
    await upload('2025/w/a.jpg', 'web pixels');
    await fullUpload('__gpp_full__/2025/w/a.nef', 'raw bytes');

    expect((await manifest('2025')).map((f) => f.path)).toEqual(['2025/w/a.jpg']);
    expect((await manifest('__gpp_full__/2025')).map((f) => f.path)).toEqual([
      '__gpp_full__/2025/w/a.nef',
    ]);
    // A full store that does not exist yet is an empty manifest — the first
    // full-scope push of a new deployment must not error.
    await rm(FULL_SYNC_ROOT, { recursive: true, force: true });
    expect(await manifest('__gpp_full__/2025')).toEqual([]);
  });

  it('refuses every escape and dotfile shape through the prefix, and writes nothing', async () => {
    for (const path of [
      '__gpp_full__',
      '__gpp_full__/../escape.txt',
      '__gpp_full__/2025/../../etc/passwd',
      '__gpp_full__/.meta/proofing/x.json',
    ]) {
      const url = fileUrl(path);
      const request = new Request(url, {
        method: 'PUT',
        headers: authorized(undefined, { 'x-content-blake3': hashOf('x') }),
        body: bytesOf('x'),
      });
      expect((await call(filePUT, request, url)).status, path).toBe(400);
    }
    expect(await exists(FULL_SYNC_ROOT)).toBe(false);
    expect(await listing(CONTENT_ROOT)).toEqual([]);
  });
});
