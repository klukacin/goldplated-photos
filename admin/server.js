import express from 'express';
import cors from 'cors';
import multer from 'multer';
import matter from 'gray-matter';
import { fileURLToPath } from 'url';
import { dirname, join, extname, basename } from 'path';
import { existsSync, readdirSync, statSync, readFileSync, writeFileSync, mkdirSync, unlinkSync, renameSync, rmSync } from 'fs';
import crypto from 'crypto';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const PROJECT_ROOT = dirname(__dirname);
const ALBUMS_DIR = join(PROJECT_ROOT, 'src/content/albums');
const HOME_DIR = join(PROJECT_ROOT, 'src/content/home');
const PUBLIC_DIR = join(PROJECT_ROOT, 'public');
// Note: Thumbnails are stored per-album in src/content/albums/{path}/.meta/thumbnails/

const app = express();
const PORT = 4444;

// Middleware
app.use(cors({ origin: ['http://localhost:4444', 'http://127.0.0.1:4444'] }));
app.use(express.json());
app.use(express.static(join(__dirname)));

// Multer setup for file uploads
const storage = multer.diskStorage({
  destination: (req, file, cb) => {
    const uploadPath = req.uploadPath || join(PROJECT_ROOT, 'uploads');
    if (!existsSync(uploadPath)) {
      mkdirSync(uploadPath, { recursive: true });
    }
    cb(null, uploadPath);
  },
  filename: (req, file, cb) => {
    cb(null, file.originalname);
  }
});
const upload = multer({ storage });

// Helper: Generate random token
function generateToken() {
  return crypto.randomBytes(6).toString('hex');
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
  const indexPath = join(ALBUMS_DIR, albumPath, 'index.md');
  if (!existsSync(indexPath)) return null;

  const fileContent = readFileSync(indexPath, 'utf-8');
  const { data, content: indexBody } = matter(fileContent);

  // Read body from separate body.md file, fall back to index.md content for backwards compatibility
  const bodyPath = join(ALBUMS_DIR, albumPath, 'body.md');
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
  const indexPath = join(ALBUMS_DIR, albumPath, 'index.md');
  const bodyPath = join(ALBUMS_DIR, albumPath, 'body.md');
  const dir = dirname(indexPath);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
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
        isCollection: meta?.isCollection || children.length > 0
      });
    }
  }

  return items.sort((a, b) => a.name.localeCompare(b.name));
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
app.get('/api/albums', (req, res) => {
  try {
    const tree = buildAlbumTree();
    res.json(tree);
  } catch (error) {
    res.status(500).json({ error: error.message });
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
app.get('/api/albums/*path', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const fullPath = join(ALBUMS_DIR, albumPath);

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
        allowAnonymous: true,
        sort: 'date-desc',
        style: 'grid',
        body: ''
      });
    }
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// POST /api/albums - Create new album
app.post('/api/albums', (req, res) => {
  try {
    const { path: rawPath, ...metadata } = req.body;

    if (!rawPath) {
      return res.status(400).json({ error: 'Path is required' });
    }

    // Sanitize path to lowercase for case-sensitive filesystem compatibility
    const albumPath = sanitizePath(rawPath);
    const fullPath = join(ALBUMS_DIR, albumPath);
    if (existsSync(join(fullPath, 'index.md'))) {
      return res.status(409).json({ error: 'Album already exists' });
    }

    // Ensure token exists
    if (!metadata.token) {
      metadata.token = generateToken();
    }

    // Set defaults
    const defaultMeta = {
      title: basename(albumPath).replace(/-/g, ' '),
      allowAnonymous: true,
      sort: 'date-desc',
      style: 'grid',
      ...metadata
    };

    writeAlbumMeta(albumPath, defaultMeta, req.body.body || '');
    res.json({ success: true, path: albumPath, token: defaultMeta.token });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// PUT /api/albums/:path - Update album
app.put('/api/albums/*path', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const { body, ...metadata } = req.body;

    const existing = readAlbumMeta(albumPath);
    if (!existing) {
      return res.status(404).json({ error: 'Album not found' });
    }

    writeAlbumMeta(albumPath, metadata, body || '');
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/albums/:path - Delete album
app.delete('/api/albums/*path', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.path);
    const fullPath = join(ALBUMS_DIR, albumPath);

    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    rmSync(fullPath, { recursive: true });
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// ============ PHOTOS API ============

// GET /api/photos/:albumPath - List photos in album
app.get('/api/photos/*albumPath', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const fullPath = join(ALBUMS_DIR, albumPath);

    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const photos = getPhotosInDir(fullPath);
    res.json(photos);
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// POST /api/photos/:albumPath - Upload photos
app.post('/api/photos/*albumPath', (req, res, next) => {
  const albumPath = getPathParam(req.params.albumPath);
  req.uploadPath = join(ALBUMS_DIR, albumPath);
  next();
}, upload.array('photos', 50), (req, res) => {
  try {
    const uploaded = req.files.map(f => f.originalname);
    res.json({ success: true, uploaded });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/photos/:albumPath/:filename - Delete photo
app.delete('/api/photos/:albumPath/file/:filename', (req, res) => {
  try {
    const { albumPath, filename } = req.params;
    const filePath = join(ALBUMS_DIR, albumPath, filename);

    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Photo not found' });
    }

    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// ============ VIDEOS API ============

// GET /api/videos/:albumPath - List videos in album
app.get('/api/videos/*albumPath', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const fullPath = join(ALBUMS_DIR, albumPath);

    if (!existsSync(fullPath)) {
      return res.status(404).json({ error: 'Album not found' });
    }

    const videos = getVideosInDir(fullPath);
    res.json(videos);
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// ============ FOLDERS API ============

// POST /api/folders - Create folder
app.post('/api/folders', (req, res) => {
  try {
    const { path: rawPath } = req.body;

    if (!rawPath) {
      return res.status(400).json({ error: 'Path is required' });
    }

    // Sanitize path to lowercase for case-sensitive filesystem compatibility
    const folderPath = sanitizePath(rawPath);
    const fullPath = join(ALBUMS_DIR, folderPath);
    if (!existsSync(fullPath)) {
      mkdirSync(fullPath, { recursive: true });
    }

    res.json({ success: true, path: folderPath });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// ============ HOME API ============

// GET /api/home/intro - Get intro content
app.get('/api/home/intro', (req, res) => {
  try {
    const introPath = join(HOME_DIR, 'intro.md');
    if (!existsSync(introPath)) {
      return res.json({ type: 'intro', body: '' });
    }

    const content = readFileSync(introPath, 'utf-8');
    const { data, content: body } = matter(content);
    res.json({ ...data, body });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// PUT /api/home/intro - Update intro
app.put('/api/home/intro', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// GET /api/home/cards - List all cards
app.get('/api/home/cards', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// POST /api/home/cards - Create card
app.post('/api/home/cards', (req, res) => {
  try {
    const { id, body, ...metadata } = req.body;
    const cardId = id || metadata.title?.toLowerCase().replace(/\s+/g, '-') || `card-${Date.now()}`;
    const cardsDir = join(HOME_DIR, 'cards');

    if (!existsSync(cardsDir)) {
      mkdirSync(cardsDir, { recursive: true });
    }

    const cardPath = join(cardsDir, `${cardId}.md`);
    const content = matter.stringify(body || '', { type: 'card', ...metadata });
    writeFileSync(cardPath, content);

    res.json({ success: true, id: cardId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// PUT /api/home/cards/:id - Update card
app.put('/api/home/cards/:id', (req, res) => {
  try {
    const { id } = req.params;
    const { body, ...metadata } = req.body;
    const cardPath = join(HOME_DIR, 'cards', `${id}.md`);

    if (!existsSync(cardPath)) {
      return res.status(404).json({ error: 'Card not found' });
    }

    const content = matter.stringify(body || '', { type: 'card', ...metadata });
    writeFileSync(cardPath, content);

    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/home/cards/:id - Delete card
app.delete('/api/home/cards/:id', (req, res) => {
  try {
    const { id } = req.params;
    const cardPath = join(HOME_DIR, 'cards', `${id}.md`);

    if (!existsSync(cardPath)) {
      return res.status(404).json({ error: 'Card not found' });
    }

    unlinkSync(cardPath);
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// POST /api/home/cards/reorder - Reorder cards
app.post('/api/home/cards/reorder', (req, res) => {
  try {
    const { order } = req.body; // Array of card IDs in new order
    const cardsDir = join(HOME_DIR, 'cards');

    order.forEach((cardId, index) => {
      const cardPath = join(cardsDir, `${cardId}.md`);
      if (existsSync(cardPath)) {
        const content = readFileSync(cardPath, 'utf-8');
        const { data, content: body } = matter(content);
        data.order = index + 1;
        writeFileSync(cardPath, matter.stringify(body, data));
      }
    });

    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// ============ ASSETS API ============

// GET /api/assets/hero - List hero images
app.get('/api/assets/hero', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// POST /api/assets/hero - Upload hero image
app.post('/api/assets/hero', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'home/hero');
  next();
}, upload.single('image'), (req, res) => {
  try {
    res.json({ success: true, filename: req.file.originalname });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/assets/hero/:name - Delete hero image
app.delete('/api/assets/hero/:name', (req, res) => {
  try {
    const filePath = join(PUBLIC_DIR, 'home/hero', req.params.name);
    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Image not found' });
    }
    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// GET /api/assets/cards - List card images
app.get('/api/assets/cards', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// POST /api/assets/cards - Upload card image
app.post('/api/assets/cards', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'home/cards');
  next();
}, upload.single('image'), (req, res) => {
  try {
    const filename = req.file.originalname;
    const url = `/home/cards/${filename}`;
    res.json({ success: true, filename, url });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/assets/cards/:name - Delete card image
app.delete('/api/assets/cards/:name', (req, res) => {
  try {
    const filePath = join(PUBLIC_DIR, 'home/cards', req.params.name);
    if (!existsSync(filePath)) {
      return res.status(404).json({ error: 'Image not found' });
    }
    unlinkSync(filePath);
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// POST /api/assets/landing - Update landing background
app.post('/api/assets/landing', (req, res, next) => {
  req.uploadPath = join(PUBLIC_DIR, 'images');
  next();
}, upload.single('image'), (req, res) => {
  try {
    // Rename to landing-bg.jpg
    const oldPath = join(PUBLIC_DIR, 'images', req.file.originalname);
    const newPath = join(PUBLIC_DIR, 'images', 'landing-bg.jpg');
    if (oldPath !== newPath) {
      renameSync(oldPath, newPath);
    }
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: error.message });
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
  return join(ALBUMS_DIR, albumPath, '.meta/thumbnails');
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
app.get('/api/cache/stats', (req, res) => {
  try {
    const stats = countAllThumbnails();
    res.json(stats);
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/cache/thumbnails - Clear all thumbnails across all albums
app.delete('/api/cache/thumbnails', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// GET /api/cache/album/:path - Get thumbnail stats for specific album
app.get('/api/cache/album/*albumPath', (req, res) => {
  try {
    const albumPath = getPathParam(req.params.albumPath);
    const stats = countAlbumThumbnails(albumPath);
    res.json(stats);
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// DELETE /api/cache/album/:path - Clear thumbnails for specific album
app.delete('/api/cache/album/*albumPath', (req, res) => {
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
    res.status(500).json({ error: error.message });
  }
});

// ============ UTILITY API ============

// GET /api/token - Generate new token
app.get('/api/token', (req, res) => {
  res.json({ token: generateToken() });
});

// Serve album images for preview
app.use('/albums', express.static(ALBUMS_DIR));

// Serve public assets (hero images, card images, landing bg)
app.use('/home', express.static(join(PUBLIC_DIR, 'home')));
app.use('/images', express.static(join(PUBLIC_DIR, 'images')));

// Start server
app.listen(PORT, '127.0.0.1', () => {
  console.log(`\n  Admin panel running at http://localhost:${PORT}\n`);
  console.log(`  Project root: ${PROJECT_ROOT}`);
  console.log(`  Albums dir: ${ALBUMS_DIR}`);
  console.log(`  Home dir: ${HOME_DIR}\n`);
});
