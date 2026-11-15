// API Base URL
const API_BASE = '';

// Admin configuration (loaded from /api/config at startup; safe defaults).
// The extension list is computed in src/site-features.mjs and served by the
// admin server — this file is a classic <script>, so /api/config is its only
// way to read the shared module. The default below is a fail-safe for the
// moment before (or if) that fetch completes, and deliberately the narrow
// universal set: falling back to "browser can paint it" is harmless, falling
// back to "browser can paint HEIC" is a broken preview.
const adminConfig = {
  previewUrl: 'http://localhost:4321',
  siteUrl: null,
  browserDisplayableImageExtensions: ['.jpg', '.jpeg', '.png', '.gif', '.webp'],
  // What the album upload endpoint accepts. Unlike the displayable list above
  // this one may include HEIC/HEIF (the gallery shows those via /api/thumbnail),
  // so the fail-safe is the full set — the server re-validates every upload.
  imageExtensions: ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif']
};

async function loadAdminConfig() {
  try {
    const config = await api.get('/api/config');
    Object.assign(adminConfig, config);
  } catch {
    // Keep defaults — dev server on :4321
  }
}

// API Helper functions
const api = {
  async get(endpoint) {
    const response = await fetch(`${API_BASE}${endpoint}`);
    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }
    return response.json();
  },

  async post(endpoint, data) {
    const response = await fetch(`${API_BASE}${endpoint}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data)
    });
    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }
    return response.json();
  },

  async put(endpoint, data) {
    const response = await fetch(`${API_BASE}${endpoint}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data)
    });
    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }
    return response.json();
  },

  async delete(endpoint) {
    const response = await fetch(`${API_BASE}${endpoint}`, {
      method: 'DELETE'
    });
    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }
    return response.json();
  },

  async upload(endpoint, files, fieldName = 'photos') {
    return api.uploadWithProgress(endpoint, files, fieldName, null);
  },

  // Upload via XHR so we can report progress (fetch has no upload progress)
  uploadWithProgress(endpoint, files, fieldName = 'photos', onProgress = null) {
    const formData = new FormData();
    if (Array.isArray(files)) {
      files.forEach(file => formData.append(fieldName, file));
    } else {
      formData.append(fieldName, files);
    }

    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', `${API_BASE}${endpoint}`);

      if (onProgress) {
        xhr.upload.addEventListener('progress', (e) => {
          if (e.lengthComputable) {
            onProgress(Math.round((e.loaded / e.total) * 100), e.loaded, e.total);
          }
        });
      }

      xhr.addEventListener('load', () => {
        let data = null;
        try { data = JSON.parse(xhr.responseText); } catch { /* non-JSON */ }
        if (xhr.status >= 200 && xhr.status < 300) {
          resolve(data ?? {});
        } else {
          reject(new Error(data?.error || `Upload failed (${xhr.status})`));
        }
      });
      xhr.addEventListener('error', () => reject(new Error('Upload failed (network error)')));
      xhr.addEventListener('abort', () => reject(new Error('Upload cancelled')));

      xhr.send(formData);
    });
  }
};

// Notifications
const notifications = {
  container: null,

  init() {
    this.container = document.getElementById('notifications');
  },

  show(message, type = 'info', duration = 4000) {
    const notification = document.createElement('div');
    notification.className = `notification ${type}`;
    notification.textContent = message;

    this.container.appendChild(notification);

    setTimeout(() => {
      notification.style.opacity = '0';
      notification.style.transform = 'translateX(100%)';
      setTimeout(() => notification.remove(), 300);
    }, duration);
  },

  success(message) {
    this.show(message, 'success');
  },

  error(message) {
    this.show(message, 'error', 6000);
  },

  warning(message) {
    this.show(message, 'warning');
  }
};

// Modal helpers
const modal = {
  show(modalId) {
    document.getElementById(modalId).classList.remove('hidden');
  },

  hide(modalId) {
    document.getElementById(modalId).classList.add('hidden');
  },

  confirm(title, message, yesLabel = 'Confirm') {
    return new Promise((resolve) => {
      document.getElementById('confirm-title').textContent = title;
      document.getElementById('confirm-message').textContent = message;

      const yesBtn = document.getElementById('confirm-yes');
      const noBtn = document.getElementById('confirm-no');
      yesBtn.textContent = yesLabel;
      yesBtn.classList.toggle('btn-danger', /delete|remove|discard/i.test(yesLabel));

      const cleanup = () => {
        modal.hide('confirm-modal');
        yesBtn.removeEventListener('click', onYes);
        noBtn.removeEventListener('click', onNo);
      };

      const onYes = () => {
        cleanup();
        resolve(true);
      };

      const onNo = () => {
        cleanup();
        resolve(false);
      };

      yesBtn.addEventListener('click', onYes);
      noBtn.addEventListener('click', onNo);

      modal.show('confirm-modal');
    });
  }
};

// Date formatter
function formatDate(dateString) {
  if (!dateString) return '';
  const date = new Date(dateString);
  return date.toISOString().split('T')[0];
}

// Parse date for API
function parseDate(dateString) {
  if (!dateString) return undefined;
  return new Date(dateString).toISOString();
}

// Get album image URL
function getAlbumImageUrl(albumPath, filename) {
  return `/albums/${albumPath}/${filename}`;
}

// The gallery's thumbnail endpoint, on the dev server. The admin panel has no
// image pipeline of its own; it borrows :4321's, which is also what already
// fills the photo grid.
function getPreviewThumbnailUrl(albumPath, filename, size = 'large') {
  return `${adminConfig.previewUrl}/api/thumbnail?path=${encodeURIComponent(`${albumPath}/${filename}`)}&size=${size}`;
}

// Which image formats a browser can paint.
//
// Chrome and Firefox cannot decode HEIC — only Safari can. The photographer
// uploads straight off an iPhone, so pointing an <img> at the original file
// gives most of them a broken-image icon while it looks fine on a Mac. Every
// admin preview therefore goes through the dev server's thumbnail endpoint,
// which converts to JPEG/WebP, and never falls back to a raw .heic.
//
// The list itself comes from src/site-features.mjs via /api/config (see
// adminConfig above), so it cannot drift from the gallery's.
function isBrowserDisplayableImage(filenameOrUrl) {
  if (!filenameOrUrl) return false;
  const base = filenameOrUrl.split(/[?#]/)[0];
  const name = base.slice(base.lastIndexOf('/') + 1);
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return false;
  return adminConfig.browserDisplayableImageExtensions.includes(name.slice(dot).toLowerCase());
}

// Whether a filename is an acceptable album *upload* (by extension). Wider
// than isBrowserDisplayableImage: HEIC/HEIF may be uploaded because the
// gallery only ever displays album photos through /api/thumbnail, which
// converts them. Used as the fallback when drag & drop gives an empty MIME
// type (Chrome/Firefox do this for HEIC). The server re-validates regardless.
function isAllowedImageFilename(filename) {
  if (!filename) return false;
  const dot = filename.lastIndexOf('.');
  if (dot <= 0) return false;
  return adminConfig.imageExtensions.includes(filename.slice(dot).toLowerCase());
}

// The original when the browser can render it, the transcode when it cannot.
function browserSafeImageUrl(originalUrl, transcodedUrl) {
  return isBrowserDisplayableImage(originalUrl) ? originalUrl : transcodedUrl;
}

// Initialize close buttons for all modals
function initModals() {
  document.querySelectorAll('.modal-close, .modal-cancel').forEach(btn => {
    btn.addEventListener('click', () => {
      btn.closest('.modal').classList.add('hidden');
    });
  });

  // Close modal on outside click
  document.querySelectorAll('.modal').forEach(modalEl => {
    modalEl.addEventListener('click', (e) => {
      if (e.target === modalEl) {
        modalEl.classList.add('hidden');
      }
    });
  });
}

// Initialize password toggle buttons
function initPasswordToggles() {
  document.querySelectorAll('.toggle-password').forEach(btn => {
    btn.addEventListener('click', () => {
      const input = btn.previousElementSibling;
      if (input.type === 'password') {
        input.type = 'text';
        btn.textContent = 'Hide';
      } else {
        input.type = 'password';
        btn.textContent = 'Show';
      }
    });
  });
}
