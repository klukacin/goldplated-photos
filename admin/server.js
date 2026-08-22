import express from 'express';
import cors from 'cors';
import multer from 'multer';
import matter from 'gray-matter';
import exifr from 'exifr';
import { spawn } from 'child_process';
import { fileURLToPath } from 'url';
import { dirname, join, extname, basename, resolve, sep } from 'path';
import { existsSync, readdirSync, statSync, readFileSync, writeFileSync, mkdirSync, unlinkSync, renameSync, rmSync } from 'fs';
import crypto from 'crypto';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const PROJECT_ROOT = dirname(__dirname);
const ALBUMS_DIR = join(PROJECT_ROOT, 'src/content/albums');
const HOME_DIR = join(PROJECT_ROOT, 'src/content/home');
const PUBLIC_DIR = join(PROJECT_ROOT, 'public');
// Note: Thumbnails are stored per-album in src/content/albums/{path}/.meta/thumbnails/

// Load .env for SITE_URL etc. (best effort — file may not exist)
try {
  process.loadEnvFile(join(PROJECT_ROOT, '.env'));
} catch { /* no .env — fine */ }

const app = express();
// 4444 unless told otherwise. Overridable so a test can take a free port
// instead of fighting an admin panel the photographer already has running.
const PORT = Number(process.env.ADMIN_PORT) || 4444;

// Middleware
const ALLOWED_ORIGINS = [`http://localhost:${PORT}`, `http://127.0.0.1:${PORT}`];

// SECURITY: reject anything a page on another origin sent us.
//
// `cors()` below does NOT do this. CORS decides who may *read* a response;
// the request has already run by the time the browser applies it. Requests a
// browser sends without a preflight — any GET, a multipart POST — therefore
// reach the handlers with full authority, and the server is bound to loopback
// but every page the photographer visits is too. An <img> tag pointing at
// /api/tools/run/deploy is enough to push the site live from someone else's
// website; a multipart POST is enough to write files into an album.
//
// Fetch metadata is the check that works here, because browsers send it on the
// no-preflight requests where Origin is absent. `same-origin` is the admin's
// own UI, `none` is the user typing the address; everything else is somebody
// else's page. Clients that send neither header are not browsers and cannot be
// driven by a hostile web page, so they pass — the panel stays scriptable.
app.use((req, res, next) => {
  const site = req.get('Sec-Fetch-Site');
  if (site && site !== 'same-origin' && site !== 'none') {
    return res.status(403).json({ error: 'Cross-site requests are not allowed' });
  }
  const origin = req.get('Origin');
  if (origin && !ALLOWED_ORIGINS.includes(origin)) {
    return res.status(403).json({ error: 'Cross-site requests are not allowed' });
  }
  next();
});

app.use(cors({ origin: ALLOWED_ORIGINS }));
app.use(express.json({ limit: '5mb' }));
app.use(express.static(join(__dirname)));

// Error type for clean status-code handling in routes/middleware
class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

// SECURITY: Resolve a user-supplied relative path against a base directory and
// assert the result stays inside it. Throws 400 on traversal attempts.
function resolveSafe(baseDir, relPath) {
  const rel = String(relPath ?? '');
  if (rel.includes('\0')) throw new HttpError(400, 'Invalid path');
  const full = resolve(baseDir, rel);
  if (full !== baseDir && !full.startsWith(baseDir + sep)) {
    throw new HttpError(400, 'Invalid path');
  }
  return full;
}

// SECURITY: Reduce an uploaded/user-supplied filename to a safe basename
function safeFilename(name) {
  const cleaned = basename(String(name ?? '').replace(/\\/g, '/')).trim();
  if (!cleaned || cleaned.startsWith('.') || cleaned.includes('\0')) {
    throw new HttpError(400, 'Invalid filename');
  }
  return cleaned;
}

// Helper: Generate random token (internal album id for the access cookie)
function generateToken() {
  return crypto.randomBytes(6).toString('hex');
}

// Helper: Generate random share token (secret link)
function generateShareToken() {
  return crypto.randomBytes(16).toString('base64url');
}

// Helper: Sanitize path to lowercase
// Converts folder names to lowercase to ensure URLs work on case-sensitive filesystems
function sanitizePath(inputPath) {
  if (!inputPath) return inputPath;
  // Split path, lowercase each segment, rejoin
  return inputPath.split('/').map(segment => segment.toLowerCase()).join('/');
}

// Helper: Get file extensions
const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif'];
const VIDEO_EXTENSIONS = ['.mp4', '.webm', '.mov', '.avi', '.mkv', '.m4v'];

// Multer setup for file uploads
const storage = multer.diskStorage({
  destination: (req, file, cb) => {
    try {
      const uploadPath = req.uploadPath;
      if (!uploadPath) throw new HttpError(400, 'Upload destination not set');
      if (!existsSync(uploadPath)) {
        mkdirSync(uploadPath, { recursive: true });
      }
      cb(null, uploadPath);
    } catch (err) {
      cb(err);
    }
  },
  filename: (req, file, cb) => {
    try {
      let name = safeFilename(file.originalname);
      // Collision handling: never silently overwrite — suffix -1, -2, ...
      // unless the client explicitly asked to overwrite (?overwrite=1).
      if (req.query.overwrite !== '1') {
        const ext = extname(name);
        const stem = basename(name, ext);
        let candidate = name;
        let counter = 1;
        while (existsSync(join(req.uploadPath, candidate))) {
          candidate = `${stem}-${counter}${ext}`;
          counter++;
        }
        if (candidate !== name) {
          req.renamedFiles = req.renamedFiles || [];
          req.renamedFiles.push({ from: name, to: candidate });
          name = candidate;
        }
      }
      cb(null, name);
    } catch (err) {
      cb(err);
    }
  }
});

const MAX_IMAGE_SIZE = 100 * 1024 * 1024;  // 100 MB per image
const MAX_VIDEO_SIZE = 2 * 1024 * 1024 * 1024; // 2 GB per video

function extFilter(allowedExtensions) {
  return (req, file, cb) => {
    const ext = extname(file.originalname || '').toLowerCase();
    if (allowedExtensions.includes(ext)) {
      cb(null, true);
    } else {
      cb(new HttpError(400, `File type not allowed: ${file.originalname}`));
    }
  };
}

const uploadImages = multer({
  storage,
  limits: { fileSize: MAX_IMAGE_SIZE, files: 50 },
  fileFilter: extFilter(IMAGE_EXTENSIONS)
});
const uploadVideos = multer({
  storage,
  limits: { fileSize: MAX_VIDEO_SIZE, files: 20 },
  fileFilter: extFilter(VIDEO_EXTENSIONS)
});

// Helper: Check if file is an image
function isImage(filename) {
  return IMAGE_EXTENSIONS.includes(extname(filename).toLowerCase());
}

// Helper: Check if file is a video
function isVideo(filename) {
  return VIDEO_EXTENSIONS.includes(extname(filename).toLowerCase());
}

// Helper: Read album metadata
function readAlbumMeta(albumPath) {
  const albumDir = resolveSafe(ALBUMS_DIR, albumPath);
  const indexPath = join(albumDir, 'index.md');
  if (!existsSync(indexPath)) return null;

  const fileContent = readFileSync(indexPath, 'utf-8');
  const { data, content: indexBody } = matter(fileContent);

  // Read body from separate body.md file, fall back to index.md content for backwards compatibility
  const bodyPath = join(albumDir, 'body.md');
  let body = '';
  if (existsSync(bodyPath)) {
    body = readFileSync(bodyPath, 'utf-8');
  } else if (indexBody && indexBody.trim()) {
    // Backwards compatibility: use body from index.md if body.md doesn't exist
    body = indexBody.trim();
  }

  return { ...data, body, path: albumPath };
}

// Helper: Write album metadata
function writeAlbumMeta(albumPath, metadata, body = '') {
  const albumDir = resolveSafe(ALBUMS_DIR, albumPath);
  const indexPath = join(albumDir, 'index.md');
  const bodyPath = join(albumDir, 'body.md');
  if (!existsSync(albumDir)) {
    mkdirSync(albumDir, { recursive: true });
  }

  // Write metadata to index.md (no body content)
  const content = matter.stringify('', metadata);
  writeFileSync(indexPath, content);

  // Write body to separate body.md file
  if (body && body.trim()) {
    writeFileSync(bodyPath, body);
  } else if (existsSync(bodyPath)) {
    unlinkSync(bodyPath); // Remove if empty
  }
}

// Helper: Merge a metadata patch into existing frontmatter.
// Keys present with value null are removed; keys absent from the patch keep
// their existing value — so a partial form never wipes fields it doesn't know.
function mergeAlbumMeta(existing, patch) {
  // `body` and `path` are handled separately by callers; never persist them.
  const { body: _b, path: _p, isNew: _n, ...rest } = patch;
  const merged = { ...existing };
  delete merged.body;
  delete merged.path;
  delete merged.isNew;
  for (const [key, value] of Object.entries(rest)) {
    if (value === null) {
      delete merged[key];
    } else if (value !== undefined) {
      merged[key] = value;
    }
  }
  return merged;
}

// Helper: Build album tree
function buildAlbumTree(dir = ALBUMS_DIR, relativePath = '') {
  const items = [];

  if (!existsSync(dir)) return items;

  const entries = readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    // Skip hidden folders like .meta
    if (entry.isDirectory() && !entry.name.startsWith('.')) {
      const entryPath = relativePath ? `${relativePath}/${entry.name}` : entry.name;
      const fullPath = join(dir, entry.name);
      const indexPath = join(fullPath, 'index.md');

      let meta = null;
      if (existsSync(indexPath)) {
        meta = readAlbumMeta(entryPath);
      }

      const children = buildAlbumTree(fullPath, entryPath);
      const photos = getPhotosInDir(fullPath);
      const videos = getVideosInDir(fullPath);

      // Calculate cumulative counts (direct + all children)
      const childPhotoCount = children.reduce((sum, child) => sum + child.photoCount, 0);
      const childVideoCount = children.reduce((sum, child) => sum + child.videoCount, 0);

      items.push({
        name: entry.name,
        path: entryPath,
        meta,
        children,
        photoCount: photos.length + childPhotoCount,
        videoCount: videos.length + childVideoCount,
        proofingCount: countProofingSubmissions(fullPath),
        isCollection: meta?.isCollection || children.length > 0
      });
    }
  }

  // Sort like the gallery: explicit `order` first, then alphabetically
  return items.sort((a, b) => {
    const orderA = a.meta?.order ?? Infinity;
    const orderB = b.meta?.order ?? Infinity;
    if (orderA !== orderB) return orderA - orderB;
    return a.name.localeCompare(b.name);
  });
}

// Helper: Get photos in directory
function getPhotosInDir(dir) {
  if (!existsSync(dir)) return [];

  return readdirSync(dir)
    .filter(f => isImage(f) && !f.startsWith('.'))
    .map(f => {
      const filePath = join(dir, f);
      const stats = statSync(filePath);
      return {
        filename: f,
        size: stats.size,
        mtime: stats.mtime.toISOString()
      };
    })
    .sort((a, b) => a.filename.localeCompare(b.filename));
}

// Helper: Get videos in directory
function getVideosInDir(dir) {
  if (!existsSync(dir)) return [];

  return readdirSync(dir)
    .filter(f => isVideo(f) && !f.startsWith('.'))
    .map(f => {
      const filePath = join(dir, f);
      const stats = statSync(filePath);
      return {
        filename: f,
        size: stats.size,
        mtime: stats.mtime.toISOString()
      };
    })
    .sort((a, b) => a.filename.localeCompare(b.filename));
}

// ============ ALBUMS API ============

// GET /api/albums - List all albums as tree
app.get('/api/albums', (req, res, next) => {
  try {
    const tree = buildAlbumTree();
    res.json(tree);
  } catch (error) {
    next(error);
  }
});

// Helper: Get path from wildcard param (Express 5 returns array)
function getPathParam(param) {
  if (Array.isArray(param)) {
    return param.join('/');
  }
  return param || '';
}

// GET /api/albums/:path - Get single album
app.get('/api/albums/*path', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const fullPath = resolveSafe(ALBUMS_DIR, albumPath);

    // Check if folder exists
    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const meta = readAlbumMeta(albumPath);
    if (meta) {
      res.json(meta);
    } else {
      // Folder exists but no index.md - return default structure
      const hasChildren = readdirSync(fullPath, { withFileTypes: true })
        .some(entry => entry.isDirectory() && !entry.name.startsWith('.'));

      res.json({
        path: albumPath,
        title: basename(albumPath).replace(/-/g, ' '),
        isCollection: hasChildren,
        isNew: true, // Flag to indicate no index.md exists yet
        token: generateToken(),
        sort: 'date-desc',
        style: 'grid',
        body: ''
      });
    }
  } catch (error) {
    next(error);
  }
});

// POST /api/albums - Create new album
app.post('/api/albums', (req, res, next) => {
  try {
    const { path: rawPath, body, ...metadata } = req.body;

    if (!rawPath) {
      return res.status(400).json({ error: 'Path is required' });
    }

    // Sanitize path to lowercase for case-sensitive filesystem compatibility
    const albumPath = sanitizePath(rawPath);
    const fullPath = resolveSafe(ALBUMS_DIR, albumPath);
    if (existsSync(join(fullPath, 'index.md'))) {
      return res.status(409).json({ error: 'Album already exists' });
    }

    // Ensure internal token exists
    if (!metadata.token) {
      metadata.token = generateToken();
    }

    // Set defaults, then drop explicit nulls (cleared optional fields)
    const defaultMeta = mergeAlbumMeta({
      title: basename(albumPath).replace(/-/g, ' '),
      sort: 'date-desc',
      style: 'grid'
    }, metadata);

    writeAlbumMeta(albumPath, defaultMeta, body || '');
    res.json({ success: true, path: albumPath, token: defaultMeta.token });
  } catch (error) {
    next(error);
  }
});

// PUT /api/albums/:path - Update album (merges into existing frontmatter —
// fields the form doesn't send are preserved, null values are removed)
app.put('/api/albums/*path', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const { body, ...patch } = req.body;

    const existing = readAlbumMeta(albumPath);
    if (!existing) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const merged = mergeAlbumMeta(existing, patch);
    // body: undefined → keep existing; string (may be '') → replace
    const newBody = body === undefined ? existing.body : body;
    writeAlbumMeta(albumPath, merged, newBody || '');
    res.json({ success: true, meta: { ...merged, path: albumPath } });
  } catch (error) {
    next(error);
  }
});

// DELETE /api/albums/:path - Delete album
app.delete('/api/albums/*path', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const fullPath = resolveSafe(ALBUMS_DIR, albumPath);

    if (albumPath === '' || fullPath === ALBUMS_DIR) {
      return res.status(400).json({ error: 'Refusing to delete the albums root' });
    }
    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    rmSync(fullPath, { recursive: true });
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ PHOTOS API ============

// GET /api/photos/:albumPath - List photos in album
app.get('/api/photos/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const fullPath = resolveSafe(ALBUMS_DIR, albumPath);

    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const photos = getPhotosInDir(fullPath);
    res.json(photos);
  } catch (error) {
    next(error);
  }
});

// POST /api/photos/:albumPath - Upload photos
app.post('/api/photos/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    req.uploadPath = resolveSafe(ALBUMS_DIR, albumPath);
    next();
  } catch (error) {
    next(error);
  }
}, uploadImages.array('photos', 50), (req, res) => {
  const files = req.files || [];
  res.json({
    success: true,
    uploaded: files.map(f => f.filename),
    renamed: req.renamedFiles || []
  });
});

// DELETE /api/photos/:albumPath/file/:filename - Delete photo
app.delete('/api/photos/:albumPath/file/:filename', (req, res, next) => {
  try {
    const filename = safeFilename(req.params.filename);
    const filePath = resolveSafe(ALBUMS_DIR, join(req.params.albumPath, filename));

    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Photo not found' });
    }

    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ VIDEOS API ============

// GET /api/videos/:albumPath - List videos in album
app.get('/api/videos/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const fullPath = resolveSafe(ALBUMS_DIR, albumPath);

    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const videos = getVideosInDir(fullPath);
    res.json(videos);
  } catch (error) {
    next(error);
  }
});

// POST /api/videos/:albumPath - Upload videos
app.post('/api/videos/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    req.uploadPath = resolveSafe(ALBUMS_DIR, albumPath);
    next();
  } catch (error) {
    next(error);
  }
}, uploadVideos.array('videos', 20), (req, res) => {
  const files = req.files || [];
  res.json({
    success: true,
    uploaded: files.map(f => f.filename),
    renamed: req.renamedFiles || []
  });
});

// DELETE /api/videos/:albumPath/file/:filename - Delete video
app.delete('/api/videos/:albumPath/file/:filename', (req, res, next) => {
  try {
    const filename = safeFilename(req.params.filename);
    const filePath = resolveSafe(ALBUMS_DIR, join(req.params.albumPath, filename));

    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Video not found' });
    }

    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ FOLDERS API ============

// POST /api/folders - Create folder
app.post('/api/folders', (req, res, next) => {
  try {
    const { path: rawPath } = req.body;

    if (!rawPath) {
      return res.status(400).json({ error: 'Path is required' });
    }

    // Sanitize path to lowercase for case-sensitive filesystem compatibility
    const folderPath = sanitizePath(rawPath);
    const fullPath = resolveSafe(ALBUMS_DIR, folderPath);
    if (!existsSync(fullPath)) {
      mkdirSync(fullPath, { recursive: true });
    }

    res.json({ success: true, path: folderPath });
  } catch (error) {
    next(error);
  }
});

// ============ HOME API ============

// GET /api/home/intro - Get intro content
app.get('/api/home/intro', (req, res, next) => {
  try {
    const introPath = join(HOME_DIR, 'intro.md');
    if (!existsSync(introPath)) {
      return res.json({ type: 'intro', body: '' });
    }

    const content = readFileSync(introPath, 'utf-8');
    const { data, content: body } = matter(content);
    res.json({ ...data, body });
  } catch (error) {
    next(error);
  }
});

// PUT /api/home/intro - Update intro
app.put('/api/home/intro', (req, res, next) => {
  try {
    const { body, ...metadata } = req.body;
    const introPath = join(HOME_DIR, 'intro.md');

    if (!existsSync(HOME_DIR)) {
      mkdirSync(HOME_DIR, { recursive: true });
    }

    const content = matter.stringify(body || '', { type: 'intro', ...metadata });
    writeFileSync(introPath, content);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// GET /api/home/cards - List all cards
app.get('/api/home/cards', (req, res, next) => {
  try {
    const cardsDir = join(HOME_DIR, 'cards');
    if (!existsSync(cardsDir)) {
      return res.json([]);
    }

    const cards = readdirSync(cardsDir)
      .filter(f => f.endsWith('.md') && !f.startsWith('.'))
      .map(f => {
        const filePath = join(cardsDir, f);
        const content = readFileSync(filePath, 'utf-8');
        const { data, content: body } = matter(content);
        return { id: f.replace('.md', ''), ...data, body };
      })
      .sort((a, b) => (a.order || 0) - (b.order || 0));

    res.json(cards);
  } catch (error) {
    next(error);
  }
});

// POST /api/home/cards - Create card
app.post('/api/home/cards', (req, res, next) => {
  try {
    const { id, body, ...metadata } = req.body;
    const rawId = id || metadata.title?.toLowerCase().replace(/\s+/g, '-') || `card-${Date.now()}`;
    const cardId = safeFilename(rawId).replace(/\.md$/i, '');
    const cardsDir = join(HOME_DIR, 'cards');

    if (!existsSync(cardsDir)) {
      mkdirSync(cardsDir, { recursive: true });
    }

    const cardPath = resolveSafe(cardsDir, `${cardId}.md`);
    const content = matter.stringify(body || '', { type: 'card', ...metadata });
    writeFileSync(cardPath, content);

    res.json({ success: true, id: cardId });
  } catch (error) {
    next(error);
  }
});

// PUT /api/home/cards/:id - Update card
app.put('/api/home/cards/:id', (req, res, next) => {
  try {
    const id = safeFilename(req.params.id).replace(/\.md$/i, '');
    const { body, ...metadata } = req.body;
    const cardPath = resolveSafe(join(HOME_DIR, 'cards'), `${id}.md`);

    if (!existsSync(cardPath)) {
      return res.status(404).json({ error: 'Card not found' });
    }

    const content = matter.stringify(body || '', { type: 'card', ...metadata });
    writeFileSync(cardPath, content);

    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// DELETE /api/home/cards/:id - Delete card
app.delete('/api/home/cards/:id', (req, res, next) => {
  try {
    const id = safeFilename(req.params.id).replace(/\.md$/i, '');
    const cardPath = resolveSafe(join(HOME_DIR, 'cards'), `${id}.md`);

    if (!existsSync(cardPath)) {
      return res.status(404).json({ error: 'Card not found' });
    }

    unlinkSync(cardPath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// POST /api/home/cards/reorder - Reorder cards
app.post('/api/home/cards/reorder', (req, res, next) => {
  try {
    const { order } = req.body; // Array of card IDs in new order
    if (!Array.isArray(order)) {
      return res.status(400).json({ error: 'order must be an array of card ids' });
    }
    const cardsDir = join(HOME_DIR, 'cards');

    order.forEach((cardId, index) => {
      const cardPath = resolveSafe(cardsDir, `${safeFilename(cardId).replace(/\.md$/i, '')}.md`);
      if (existsSync(cardPath)) {
        const content = readFileSync(cardPath, 'utf-8');
        const { data, content: body } = matter(content);
        data.order = index + 1;
        writeFileSync(cardPath, matter.stringify(body, data));
      }
    });

    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ ASSETS API ============

// GET /api/assets/hero - List hero images
app.get('/api/assets/hero', (req, res, next) => {
  try {
    const heroDir = join(PUBLIC_DIR, 'home/hero');
    if (!existsSync(heroDir)) {
      return res.json([]);
    }

    const images = readdirSync(heroDir)
      .filter(f => isImage(f))
      .map(f => ({
        filename: f,
        url: `/home/hero/${f}`
      }));

    res.json(images);
  } catch (error) {
    next(error);
  }
});

// POST /api/assets/hero - Upload hero image
app.post('/api/assets/hero', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'home/hero');
  next();
}, uploadImages.single('image'), (req, res, next) => {
  try {
    if (!req.file) return res.status(400).json({ error: 'No image uploaded' });
    res.json({ success: true, filename: req.file.filename });
  } catch (error) {
    next(error);
  }
});

// DELETE /api/assets/hero/:name - Delete hero image
app.delete('/api/assets/hero/:name', (req, res, next) => {
  try {
    const filePath = join(PUBLIC_DIR, 'home/hero', safeFilename(req.params.name));
    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Image not found' });
    }
    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// GET /api/assets/cards - List card images
app.get('/api/assets/cards', (req, res, next) => {
  try {
    const cardsDir = join(PUBLIC_DIR, 'home/cards');
    if (!existsSync(cardsDir)) {
      return res.json([]);
    }

    const images = readdirSync(cardsDir)
      .filter(f => isImage(f))
      .map(f => ({
        filename: f,
        url: `/home/cards/${f}`
      }));

    res.json(images);
  } catch (error) {
    next(error);
  }
});

// POST /api/assets/cards - Upload card image
app.post('/api/assets/cards', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'home/cards');
  next();
}, uploadImages.single('image'), (req, res, next) => {
  try {
    if (!req.file) return res.status(400).json({ error: 'No image uploaded' });
    const filename = req.file.filename;
    const url = `/home/cards/${filename}`;
    res.json({ success: true, filename, url });
  } catch (error) {
    next(error);
  }
});

// DELETE /api/assets/cards/:name - Delete card image
app.delete('/api/assets/cards/:name', (req, res, next) => {
  try {
    const filePath = join(PUBLIC_DIR, 'home/cards', safeFilename(req.params.name));
    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Image not found' });
    }
    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// POST /api/assets/landing - Update landing background
app.post('/api/assets/landing', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'images');
  next();
}, uploadImages.single('image'), (req, res, next) => {
  try {
    if (!req.file) return res.status(400).json({ error: 'No image uploaded' });
    // Rename to landing-bg.jpg (this endpoint always replaces the background)
    const oldPath = req.file.path;
    const newPath = join(PUBLIC_DIR, 'images', 'landing-bg.jpg');
    if (oldPath !== newPath) {
      renameSync(oldPath, newPath);
    }
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ CACHE API ============

// Helper: Count files in directory
function countFilesInDir(dir) {
  if (!existsSync(dir)) return 0;
  return readdirSync(dir).filter(f => !f.startsWith('.')).length;
}

// Helper: Get thumbnail dir for an album
function getAlbumThumbnailDir(albumPath) {
  return join(resolveSafe(ALBUMS_DIR, albumPath), '.meta/thumbnails');
}

// Helper: Count thumbnails for a specific album
function countAlbumThumbnails(albumPath) {
  const thumbDir = getAlbumThumbnailDir(albumPath);
  const stats = { small: 0, medium: 0, large: 0, total: 0 };

  if (!existsSync(thumbDir)) return stats;

  stats.small = countFilesInDir(join(thumbDir, 'small'));
  stats.medium = countFilesInDir(join(thumbDir, 'medium'));
  stats.large = countFilesInDir(join(thumbDir, 'large'));
  stats.total = stats.small + stats.medium + stats.large;

  return stats;
}

// Helper: Count all thumbnails across all albums recursively
function countAllThumbnails(dir = ALBUMS_DIR) {
  const stats = { small: 0, medium: 0, large: 0, total: 0 };

  if (!existsSync(dir)) return stats;

  const entries = readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (entry.name === '.meta') {
        // Found a .meta folder, check for thumbnails
        const thumbDir = join(dir, entry.name, 'thumbnails');
        if (existsSync(thumbDir)) {
          stats.small += countFilesInDir(join(thumbDir, 'small'));
          stats.medium += countFilesInDir(join(thumbDir, 'medium'));
          stats.large += countFilesInDir(join(thumbDir, 'large'));
        }
      } else if (!entry.name.startsWith('.')) {
        // Recurse into album subdirectories
        const subStats = countAllThumbnails(join(dir, entry.name));
        stats.small += subStats.small;
        stats.medium += subStats.medium;
        stats.large += subStats.large;
      }
    }
  }

  stats.total = stats.small + stats.medium + stats.large;
  return stats;
}

// GET /api/cache/stats - Get thumbnail statistics for ALL albums
app.get('/api/cache/stats', (req, res, next) => {
  try {
    const stats = countAllThumbnails();
    res.json(stats);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/cache/thumbnails - Clear all thumbnails across all albums
app.delete('/api/cache/thumbnails', (req, res, next) => {
  try {
    let deleted = 0;

    function clearThumbnailsRecursive(dir) {
      if (!existsSync(dir)) return;

      const entries = readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        if (entry.isDirectory()) {
          if (entry.name === '.meta') {
            const thumbDir = join(dir, entry.name, 'thumbnails');
            if (existsSync(thumbDir)) {
              const before = countFilesInDir(join(thumbDir, 'small')) +
                           countFilesInDir(join(thumbDir, 'medium')) +
                           countFilesInDir(join(thumbDir, 'large'));
              rmSync(thumbDir, { recursive: true });
              deleted += before;
            }
          } else if (!entry.name.startsWith('.')) {
            clearThumbnailsRecursive(join(dir, entry.name));
          }
        }
      }
    }

    clearThumbnailsRecursive(ALBUMS_DIR);
    res.json({ success: true, deleted });
  } catch (error) {
    next(error);
  }
});

// GET /api/cache/album/:path - Get thumbnail stats for specific album
app.get('/api/cache/album/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const stats = countAlbumThumbnails(albumPath);
    res.json(stats);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/cache/album/:path - Clear thumbnails for specific album
app.delete('/api/cache/album/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const thumbDir = getAlbumThumbnailDir(albumPath);

    if (!existsSync(thumbDir)) {
      return res.json({ success: true, deleted: 0 });
    }

    const before = countFilesInDir(join(thumbDir, 'small')) +
                   countFilesInDir(join(thumbDir, 'medium')) +
                   countFilesInDir(join(thumbDir, 'large'));

    rmSync(thumbDir, { recursive: true });

    res.json({ success: true, deleted: before });
  } catch (error) {
    next(error);
  }
});

// ============ PHOTO ORDER / BULK / EXIF API ============

// POST /api/photo-order/:albumPath - Save custom photo order (drag & drop)
app.post('/api/photo-order/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const { order } = req.body;
    if (!Array.isArray(order) || !order.every(f => typeof f === 'string')) {
      return res.status(400).json({ error: 'order must be an array of filenames' });
    }

    const existing = readAlbumMeta(albumPath);
    if (!existing) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const photoOrder = order.map(f => safeFilename(f));
    const merged = mergeAlbumMeta(existing, { photoOrder });
    writeAlbumMeta(albumPath, merged, existing.body || '');
    res.json({ success: true, photoOrder });
  } catch (error) {
    next(error);
  }
});

// POST /api/photo-bulk/delete/:albumPath - Delete multiple photos/videos
app.post('/api/photo-bulk/delete/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const albumDir = resolveSafe(ALBUMS_DIR, albumPath);
    const { filenames } = req.body;
    if (!Array.isArray(filenames) || filenames.length === 0) {
      return res.status(400).json({ error: 'filenames must be a non-empty array' });
    }

    const deleted = [];
    const missing = [];
    for (const raw of filenames) {
      const filename = safeFilename(raw);
      const filePath = join(albumDir, filename);
      if (existsSync(filePath)) {
        unlinkSync(filePath);
        deleted.push(filename);
      } else {
        missing.push(filename);
      }
    }
    res.json({ success: true, deleted, missing });
  } catch (error) {
    next(error);
  }
});

// POST /api/photo-bulk/move/:albumPath - Move photos/videos to another album
app.post('/api/photo-bulk/move/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const sourceDir = resolveSafe(ALBUMS_DIR, albumPath);
    const { filenames, target } = req.body;
    if (!Array.isArray(filenames) || filenames.length === 0) {
      return res.status(400).json({ error: 'filenames must be a non-empty array' });
    }
    if (typeof target !== 'string' || !target.trim()) {
      return res.status(400).json({ error: 'target album path is required' });
    }
    const targetDir = resolveSafe(ALBUMS_DIR, sanitizePath(target.trim()));
    if (!existsSync(targetDir) || !statSync(targetDir).isDirectory()) {
      return res.status(404).json({ error: 'Target album not found' });
    }
    if (targetDir === sourceDir) {
      return res.status(400).json({ error: 'Target album is the same as the source' });
    }

    const moved = [];
    const renamed = [];
    const missing = [];
    for (const raw of filenames) {
      const filename = safeFilename(raw);
      const src = join(sourceDir, filename);
      if (!existsSync(src)) {
        missing.push(filename);
        continue;
      }
      // Collision handling: never overwrite in the target album
      let destName = filename;
      const ext = extname(filename);
      const stem = basename(filename, ext);
      let counter = 1;
      while (existsSync(join(targetDir, destName))) {
        destName = `${stem}-${counter}${ext}`;
        counter++;
      }
      renameSync(src, join(targetDir, destName));
      moved.push(destName);
      if (destName !== filename) renamed.push({ from: filename, to: destName });
    }
    res.json({ success: true, moved, renamed, missing });
  } catch (error) {
    next(error);
  }
});

// GET /api/photo-exif/:photoPath - EXIF data for the admin preview
app.get('/api/photo-exif/*photoPath', async (req, res, next) => {
  try {
    const photoPath = getPathParam(req.params.photoPath);
    const filePath = resolveSafe(ALBUMS_DIR, photoPath);
    if (!existsSync(filePath) || !isImage(filePath)) {
      return res.status(404).json({ error: 'Photo not found' });
    }
    try {
      const exif = await exifr.parse(filePath, {
        pick: ['DateTimeOriginal', 'Make', 'Model', 'LensModel', 'FocalLength', 'FNumber', 'ExposureTime', 'ISO']
      });
      res.json({ exif: exif || {} });
    } catch {
      res.json({ exif: {} });
    }
  } catch (error) {
    next(error);
  }
});

// ============ ALBUM RENAME / REORDER API ============

// POST /api/album-rename/:path - Rename or move an album folder
app.post('/api/album-rename/*path', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const src = resolveSafe(ALBUMS_DIR, albumPath);
    const { newPath } = req.body;

    if (typeof newPath !== 'string' || !newPath.trim()) {
      return res.status(400).json({ error: 'newPath is required' });
    }
    const sanitized = sanitizePath(newPath.trim().replace(/^\/+|\/+$/g, ''));
    const dest = resolveSafe(ALBUMS_DIR, sanitized);

    if (!existsSync(src)) {
      return res.status(404).json({ error: 'Album not found' });
    }
    if (src === ALBUMS_DIR || dest === ALBUMS_DIR) {
      return res.status(400).json({ error: 'Invalid path' });
    }
    if (existsSync(dest)) {
      return res.status(409).json({ error: 'An album already exists at that path' });
    }
    if ((dest + sep).startsWith(src + sep)) {
      return res.status(400).json({ error: 'Cannot move an album inside itself' });
    }

    mkdirSync(dirname(dest), { recursive: true });
    renameSync(src, dest);
    res.json({ success: true, path: sanitized });
  } catch (error) {
    next(error);
  }
});

// POST /api/albums-reorder - Persist sibling album order (writes `order` fields)
app.post('/api/albums-reorder', (req, res, next) => {
  try {
    const { parent, order } = req.body; // parent: '' for root; order: folder names
    if (!Array.isArray(order) || !order.every(n => typeof n === 'string')) {
      return res.status(400).json({ error: 'order must be an array of folder names' });
    }
    const parentPath = typeof parent === 'string' ? parent : '';
    resolveSafe(ALBUMS_DIR, parentPath); // validate

    const skipped = [];
    order.forEach((name, index) => {
      const childPath = parentPath ? `${parentPath}/${safeFilename(name)}` : safeFilename(name);
      const existing = readAlbumMeta(childPath);
      if (!existing) {
        skipped.push(name); // bare folder without index.md — nothing to write to
        return;
      }
      const merged = mergeAlbumMeta(existing, { order: index + 1 });
      writeAlbumMeta(childPath, merged, existing.body || '');
    });
    res.json({ success: true, skipped });
  } catch (error) {
    next(error);
  }
});

// ============ PROOFING API ============

// Helper: proofing submissions directory for an album
function proofingDir(albumPath) {
  return join(resolveSafe(ALBUMS_DIR, albumPath), '.meta', 'proofing');
}

// Helper: count proofing submissions (for tree badges)
function countProofingSubmissions(fullPath) {
  const dir = join(fullPath, '.meta', 'proofing');
  if (!existsSync(dir)) return 0;
  return readdirSync(dir).filter(f => f.endsWith('.json')).length;
}

// GET /api/proofing/:albumPath - List proofing submissions (newest first)
app.get('/api/proofing/*albumPath', (req, res, next) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const dir = proofingDir(albumPath);
    if (!existsSync(dir)) return res.json([]);

    const submissions = readdirSync(dir)
      .filter(f => f.endsWith('.json'))
      .sort()
      .reverse()
      .map(f => {
        try {
          const data = JSON.parse(readFileSync(join(dir, f), 'utf-8'));
          return { id: f, ...data };
        } catch {
          return null;
        }
      })
      .filter(Boolean);

    res.json(submissions);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/proofing/:albumPath/file/:id - Delete one submission
app.delete('/api/proofing/:albumPath/file/:id', (req, res, next) => {
  try {
    const id = safeFilename(req.params.id);
    if (!id.endsWith('.json')) {
      return res.status(400).json({ error: 'Invalid submission id' });
    }
    const filePath = join(proofingDir(req.params.albumPath), id);
    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Submission not found' });
    }
    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    next(error);
  }
});

// ============ TOOLS API (script runner) ============

// Fixed whitelist — ids map to commands, nothing user-supplied is executed
const TOOL_SCRIPTS = {
  'build': { label: 'Build site', cmd: 'npm', args: ['run', 'build'], confirm: false },
  'deploy': { label: 'Deploy to production', cmd: 'npm', args: ['run', 'deploy'], confirm: true },
  'deploy-parallel': { label: 'Deploy (parallel sync)', cmd: 'npm', args: ['run', 'deploy:parallel'], confirm: true },
  'update-albums': { label: 'Normalize albums (npm run update)', cmd: 'node', args: ['scripts/update-albums.mjs'], confirm: true },
  'sanitize': { label: 'Sanitize folder names', cmd: 'node', args: ['scripts/sanitize-folders.mjs'], confirm: true },
  'fix-covers': { label: 'Fix broken cover photos', cmd: 'node', args: ['scripts/fix-broken-cards.mjs'], confirm: false },
};

let runningTool = null;

// GET /api/tools/scripts - List runnable scripts
app.get('/api/tools/scripts', (req, res) => {
  res.json({
    running: runningTool,
    scripts: Object.entries(TOOL_SCRIPTS).map(([id, s]) => ({
      id, label: s.label, confirm: s.confirm
    }))
  });
});

// GET /api/tools/run/:id - Run a whitelisted script, stream output via SSE
app.get('/api/tools/run/:id', (req, res) => {
  const id = req.params.id;
  const tool = TOOL_SCRIPTS[id];
  if (!tool) {
    return res.status(404).json({ error: 'Unknown script' });
  }
  if (runningTool) {
    return res.status(409).json({ error: `"${runningTool}" is already running` });
  }
  runningTool = id;

  res.writeHead(200, {
    'Content-Type': 'text/event-stream',
    'Cache-Control': 'no-cache',
    'Connection': 'keep-alive'
  });
  const send = (event, data) => {
    res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  };
  send('start', { id, label: tool.label });

  const child = spawn(tool.cmd, tool.args, { cwd: PROJECT_ROOT, env: process.env });
  child.stdout.on('data', d => send('output', d.toString()));
  child.stderr.on('data', d => send('output', d.toString()));
  child.on('close', (code) => {
    send('done', { code });
    runningTool = null;
    res.end();
  });
  child.on('error', (err) => {
    send('output', `Failed to start: ${err.message}\n`);
    send('done', { code: -1 });
    runningTool = null;
    res.end();
  });
  // If the browser disconnects, let the script finish (a deploy must not be
  // killed mid-flight) — the lock clears when the process exits.
});

// ============ UTILITY API ============

// GET /api/token - Generate new internal token
app.get('/api/token', (req, res) => {
  res.json({ token: generateToken() });
});

// GET /api/share-token - Generate new random share token (secret link)
app.get('/api/share-token', (req, res) => {
  res.json({ shareToken: generateShareToken() });
});

// GET /api/config - Admin-relevant configuration for the frontend
app.get('/api/config', (req, res) => {
  res.json({
    previewUrl: process.env.ADMIN_PREVIEW_URL || 'http://localhost:4321',
    siteUrl: process.env.SITE_URL || null
  });
});

// Serve album images for preview
app.use('/albums', express.static(ALBUMS_DIR));

// Serve public assets (hero images, card images, landing bg)
app.use('/home', express.static(join(PUBLIC_DIR, 'home')));
app.use('/images', express.static(join(PUBLIC_DIR, 'images')));

// Error-handling middleware: always answer JSON, map known error types,
// and never leak internal stack traces / absolute paths to the client.
// eslint-disable-next-line no-unused-vars
app.use((err, req, res, next) => {
  if (err instanceof HttpError) {
    return res.status(err.status).json({ error: err.message });
  }
  if (err instanceof multer.MulterError) {
    const messages = {
      LIMIT_FILE_SIZE: 'File too large',
      LIMIT_FILE_COUNT: 'Too many files in one upload',
      LIMIT_UNEXPECTED_FILE: 'Unexpected upload field'
    };
    return res.status(400).json({ error: messages[err.code] || `Upload error: ${err.code}` });
  }
  if (err.type === 'entity.too.large') {
    return res.status(413).json({ error: 'Request body too large' });
  }
  console.error('[admin]', err);
  res.status(500).json({ error: 'Internal server error' });
});

// Start server
app.listen(PORT, '127.0.0.1', () => {
  console.log(`\n  Admin panel running at http://localhost:${PORT}\n`);
  console.log(`  Project root: ${PROJECT_ROOT}`);
  console.log(`  Albums dir: ${ALBUMS_DIR}`);
  console.log(`  Home dir: ${HOME_DIR}\n`);
});
