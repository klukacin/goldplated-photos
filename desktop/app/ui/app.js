/* Goldplated Photos — desktop UI.
 *
 * Deliberately dependency-free vanilla JS: every operation is one `invoke`
 * into the Rust core, which is where the logic and the tests live. This file
 * only renders and gathers input.
 */

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { open: openDialog } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const LIBRARY_KEY = 'gpp.lastLibrary';

const state = {
  photos: [],
  albums: [],
  selected: new Set(),
  cursor: -1,
  currentAlbum: '',
  filter: { minRating: null, flag: null, text: '' },
  editingAlbum: null,
  remoteAlbums: [],
};

const $ = (id) => document.getElementById(id);

// --------------------------------------------------------------- utilities

function status(text) {
  $('status-text').textContent = text;
}

function showError(el, message) {
  const node = $(el);
  node.textContent = message;
  node.hidden = !message;
}

function openModal(id) { $(id).hidden = false; }
function closeModal(id) { $(id).hidden = true; }

document.addEventListener('click', (e) => {
  if (e.target.matches('[data-close]')) {
    e.target.closest('.modal').hidden = true;
  }
  // Click on the backdrop, not the card, dismisses.
  if (e.target.classList.contains('modal')) e.target.hidden = true;
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    document.querySelectorAll('.modal:not([hidden])').forEach((m) => (m.hidden = true));
  }
});

// ----------------------------------------------------------------- startup

async function boot() {
  const remembered = localStorage.getItem(LIBRARY_KEY);
  if (remembered) {
    try {
      await openLibrary(remembered);
      return;
    } catch {
      // The folder moved or is unreadable — fall through to the picker.
      localStorage.removeItem(LIBRARY_KEY);
    }
  }
  $('welcome').hidden = false;
}

$('open-library-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Choose a photo folder' });
  if (!dir) return;
  try {
    await openLibrary(dir);
  } catch (err) {
    showError('welcome-error', String(err));
  }
});

async function openLibrary(path) {
  const info = await invoke('open_library', { path });
  localStorage.setItem(LIBRARY_KEY, path);
  $('welcome').hidden = true;
  $('app').hidden = false;
  renderStatus(info);
  await Promise.all([refreshAlbums(), refreshPhotos()]);
  await loadPublishTarget();
}

function renderStatus(info) {
  const name = info.root.split('/').filter(Boolean).pop() || info.root;
  $('library-name').textContent = name;
  $('all-count').textContent = info.photo_count;
  $('library-stats').innerHTML =
    `${info.photo_count} photos · ${info.album_count} albums` +
    (info.cameras.length ? `<br>${info.cameras.join(', ')}` : '');
}

// ------------------------------------------------------------------ import

$('import-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Import from folder' });
  if (!dir) return;

  $('progress').hidden = false;
  status('Importing…');
  try {
    const summary = await invoke('import_photos', { dir });
    status(
      `Imported ${summary.imported} · updated ${summary.updated} · ` +
      `unchanged ${summary.skipped}` +
      (summary.failed.length ? ` · ${summary.failed.length} failed` : '')
    );
    await refreshAll();
  } catch (err) {
    status(`Import failed: ${err}`);
  } finally {
    $('progress').hidden = true;
    $('progress-fill').style.width = '0';
  }
});

listen('import-progress', ({ payload }) => {
  const pct = payload.total ? (payload.processed / payload.total) * 100 : 0;
  $('progress-fill').style.width = `${pct}%`;
  status(`Importing ${payload.processed}/${payload.total} — ${payload.current}`);
});

// ------------------------------------------------------------------ photos

async function refreshPhotos() {
  const filter = {
    minRating: state.filter.minRating,
    flag: state.filter.flag,
    text: state.filter.text || null,
    albumPath: state.currentAlbum || null,
    sort: state.currentAlbum ? 'album-order' : 'captured-asc',
    limit: 2000,
  };
  state.photos = await invoke('list_photos', { filter });
  state.selected.clear();
  state.cursor = state.photos.length ? 0 : -1;
  renderGrid();
  updateSelectionUI();
}

async function refreshAll() {
  renderStatus(await invoke('library_status'));
  await Promise.all([refreshAlbums(), refreshPhotos()]);
}

function renderGrid() {
  const grid = $('grid');
  grid.innerHTML = '';
  $('empty-state').hidden = state.photos.length > 0;

  const frag = document.createDocumentFragment();
  state.photos.forEach((photo, index) => {
    const cell = document.createElement('div');
    cell.className = 'cell';
    cell.dataset.index = index;
    if (state.selected.has(photo.id)) cell.classList.add('selected');
    if (index === state.cursor) cell.classList.add('cursor');
    if (photo.flag === 'reject') cell.classList.add('rejected');

    const img = document.createElement('img');
    img.loading = 'lazy';
    img.alt = photo.filename;
    // Blur placeholder from the catalog until the real thumbnail resolves.
    if (photo.blur_lqip) {
      img.style.background = `url(${photo.blur_lqip}) center/cover`;
    }
    invoke('thumbnail_path', { id: photo.id, size: 'small' })
      .then((p) => { img.src = convertFileSrc(p); })
      .catch(() => {});
    cell.appendChild(img);

    const name = document.createElement('div');
    name.className = 'cell-name';
    name.textContent = photo.filename;
    cell.appendChild(name);

    const bar = document.createElement('div');
    bar.className = 'cell-bar';
    const stars = document.createElement('span');
    stars.className = 'cell-stars';
    stars.textContent = '★'.repeat(photo.rating);
    bar.appendChild(stars);
    if (photo.flag !== 'none') {
      const flag = document.createElement('span');
      flag.className = 'cell-flag';
      flag.textContent = photo.flag === 'pick' ? '⚑' : '✕';
      bar.appendChild(flag);
    }
    cell.appendChild(bar);

    cell.addEventListener('click', (e) => {
      if (e.metaKey || e.ctrlKey) {
        state.selected.has(photo.id)
          ? state.selected.delete(photo.id)
          : state.selected.add(photo.id);
      } else if (e.shiftKey && state.cursor >= 0) {
        const [a, b] = [state.cursor, index].sort((x, y) => x - y);
        for (let i = a; i <= b; i++) state.selected.add(state.photos[i].id);
      } else {
        state.selected.clear();
        state.selected.add(photo.id);
      }
      state.cursor = index;
      renderGrid();
      updateSelectionUI();
      showInspector(photo);
    });

    frag.appendChild(cell);
  });
  grid.appendChild(frag);
}

function updateSelectionUI() {
  const n = state.selected.size;
  $('selection-info').textContent = n ? `${n} selected` : '';
  $('add-to-album-btn').disabled = n === 0;
}

/// Ids the next action applies to: the selection, or the cursor when empty.
function targetIds() {
  if (state.selected.size) return [...state.selected];
  if (state.cursor >= 0) return [state.photos[state.cursor].id];
  return [];
}

async function applyRating(rating) {
  const ids = targetIds();
  if (!ids.length) return;
  await invoke('set_rating', { ids, rating });
  ids.forEach((id) => {
    const p = state.photos.find((x) => x.id === id);
    if (p) p.rating = rating;
  });
  renderGrid();
  status(`Rated ${ids.length} photo(s) ${rating}★`);
}

async function applyFlag(flag) {
  const ids = targetIds();
  if (!ids.length) return;
  await invoke('set_flag', { ids, flag });
  ids.forEach((id) => {
    const p = state.photos.find((x) => x.id === id);
    if (p) p.flag = flag;
  });
  renderGrid();
  status(`Flagged ${ids.length} photo(s): ${flag}`);
}

// Rating and triage keys — the reason this app is faster than a web admin.
document.addEventListener('keydown', (e) => {
  if (e.target.matches('input, select, textarea')) return;
  if ($('app').hidden) return;

  if (e.key >= '0' && e.key <= '5') {
    e.preventDefault();
    applyRating(Number(e.key));
  } else if (e.key === 'p' || e.key === 'P') {
    e.preventDefault();
    applyFlag('pick');
  } else if (e.key === 'x' || e.key === 'X') {
    e.preventDefault();
    applyFlag('reject');
  } else if (e.key === 'u' || e.key === 'U') {
    e.preventDefault();
    applyFlag('none');
  } else if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
    e.preventDefault();
    moveCursor(e.key === 'ArrowRight' ? 1 : -1);
  } else if (e.key === 'a' && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    state.photos.forEach((p) => state.selected.add(p.id));
    renderGrid();
    updateSelectionUI();
  }
});

function moveCursor(delta) {
  if (!state.photos.length) return;
  state.cursor = Math.max(0, Math.min(state.photos.length - 1, state.cursor + delta));
  const photo = state.photos[state.cursor];
  state.selected.clear();
  state.selected.add(photo.id);
  renderGrid();
  updateSelectionUI();
  showInspector(photo);
  document.querySelector('.cell.cursor')?.scrollIntoView({ block: 'nearest' });
}

// --------------------------------------------------------------- inspector

async function showInspector(photo) {
  const panel = $('inspector');
  panel.hidden = false;
  $('inspector-name').textContent = photo.filename;

  try {
    const p = await invoke('thumbnail_path', { id: photo.id, size: 'medium' });
    $('inspector-img').src = convertFileSrc(p);
  } catch {
    $('inspector-img').removeAttribute('src');
  }

  const rating = $('inspector-rating');
  rating.innerHTML = '';
  for (let i = 1; i <= 5; i++) {
    const star = document.createElement('button');
    star.className = 'star' + (i <= photo.rating ? ' on' : '');
    star.textContent = '★';
    star.title = `${i} star${i > 1 ? 's' : ''}`;
    star.addEventListener('click', async () => {
      // Clicking the current rating clears it, like Lightroom.
      await invoke('set_rating', { ids: [photo.id], rating: i === photo.rating ? 0 : i });
      photo.rating = i === photo.rating ? 0 : i;
      renderGrid();
      showInspector(photo);
    });
    rating.appendChild(star);
  }

  const rows = [
    ['Captured', photo.captured_at || '—'],
    ['Camera', [photo.camera_make, photo.camera_model].filter(Boolean).join(' ') || '—'],
    ['Lens', photo.lens || '—'],
    ['Exposure', formatExposure(photo)],
    ['Dimensions', photo.width && photo.height ? `${photo.width} × ${photo.height}` : '—'],
    ['Size', formatBytes(photo.file_size)],
    ['Path', photo.rel_path],
  ];
  $('inspector-meta').innerHTML = rows
    .map(([k, v]) => `<dt>${k}</dt><dd>${escapeHtml(String(v))}</dd>`)
    .join('');
}

function formatExposure(p) {
  const bits = [];
  if (p.focal_length) bits.push(`${Math.round(p.focal_length)}mm`);
  if (p.aperture) bits.push(`f/${p.aperture}`);
  if (p.shutter) bits.push(p.shutter < 1 ? `1/${Math.round(1 / p.shutter)}s` : `${p.shutter}s`);
  if (p.iso) bits.push(`ISO ${p.iso}`);
  return bits.join(' · ') || '—';
}

function formatBytes(bytes) {
  if (!bytes) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(1)} ${units[i]}`;
}

function escapeHtml(s) {
  return s.replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}

// ------------------------------------------------------------------ albums

async function refreshAlbums() {
  state.albums = await invoke('list_albums');
  const list = $('album-list');
  list.innerHTML = '';

  if (!state.albums.length) {
    list.innerHTML = '<p class="muted" style="padding:0.4rem 0.55rem">No albums yet.</p>';
  }

  state.albums.forEach((entry) => {
    const item = document.createElement('button');
    item.className = 'album-item' + (state.currentAlbum === entry.path ? ' active' : '');

    const depth = entry.path.split('/').length - 1;
    const name = document.createElement('span');
    name.className = 'name' + (depth ? ' depth-1' : '');
    name.textContent = entry.title;
    item.appendChild(name);

    if (entry.is_locked) {
      const lock = document.createElement('span');
      lock.className = 'lock';
      lock.textContent = '🔒';
      lock.title = 'Password or share link set';
      item.appendChild(lock);
    }

    const count = document.createElement('span');
    count.className = 'count';
    count.textContent = entry.photo_count;
    item.appendChild(count);

    const cog = document.createElement('button');
    cog.className = 'album-cog';
    cog.textContent = '⚙';
    cog.title = 'Album settings';
    cog.addEventListener('click', (e) => {
      e.stopPropagation();
      openAlbumSettings(entry);
    });
    item.appendChild(cog);

    item.addEventListener('click', () => selectAlbum(entry.path));
    list.appendChild(item);
  });

  // Keep the publish dialog's album picker in step.
  const picker = $('publish-album');
  picker.innerHTML = '<option value="">All albums</option>';
  state.albums.forEach((a) => {
    const opt = document.createElement('option');
    opt.value = a.path;
    opt.textContent = a.path;
    picker.appendChild(opt);
  });
}

async function selectAlbum(path) {
  state.currentAlbum = path;
  document.querySelectorAll('.nav-item').forEach((n) =>
    n.classList.toggle('active', (n.dataset.album || '') === path));
  await refreshAlbums();
  await refreshPhotos();
}

document.querySelector('.nav-item[data-album=""]').addEventListener('click', () => selectAlbum(''));

$('new-album-btn').addEventListener('click', () => {
  $('album-path').value = '';
  $('album-title').value = '';
  $('album-collection').checked = false;
  showError('album-error', '');
  openModal('album-modal');
});

$('album-create-btn').addEventListener('click', async () => {
  const album = {
    path: $('album-path').value.trim(),
    title: $('album-title').value.trim() || null,
    isCollection: $('album-collection').checked,
  };
  if (!album.path) {
    showError('album-error', 'A path is required.');
    return;
  }
  try {
    await invoke('create_album', { album });
    closeModal('album-modal');
    await refreshAll();
    status(`Created album ${album.path}`);
  } catch (err) {
    showError('album-error', String(err));
  }
});

function openAlbumSettings(entry) {
  state.editingAlbum = entry;
  $('settings-album-title').textContent = entry.path;
  $('set-title').value = entry.title || '';
  $('set-description').value = entry.description || '';
  $('set-sort').value = entry.sort;
  $('set-style').value = entry.style;
  $('set-password').value = entry.password || '';
  $('set-tags').value = (entry.tags || []).join(', ');
  $('set-proofing').checked = entry.proofing;
  $('set-download').checked = entry.allow_download;
  $('set-hidden').checked = entry.hidden;
  $('share-link').value = entry.share_token
    ? `/photos/${entry.path}?token=${entry.share_token}`
    : '';
  showError('settings-error', '');
  openModal('settings-album-modal');
}

$('share-link-btn').addEventListener('click', async () => {
  if (!state.editingAlbum) return;
  try {
    const token = await invoke('generate_share_link', { path: state.editingAlbum.path });
    $('share-link').value = `/photos/${state.editingAlbum.path}?token=${token}`;
    await refreshAlbums();
    status('Share link generated — publish to activate it');
  } catch (err) {
    showError('settings-error', String(err));
  }
});

$('album-save-btn').addEventListener('click', async () => {
  if (!state.editingAlbum) return;
  const password = $('set-password').value.trim();
  const update = {
    title: $('set-title').value.trim(),
    description: $('set-description').value.trim() || null,
    sort: $('set-sort').value,
    style: $('set-style').value,
    // null clears the field; omitting it would leave it untouched.
    password: password || null,
    proofing: $('set-proofing').checked,
    allowDownload: $('set-download').checked,
    hidden: $('set-hidden').checked,
    tags: $('set-tags').value.split(',').map((t) => t.trim()).filter(Boolean),
  };
  try {
    await invoke('update_album', { path: state.editingAlbum.path, update });
    closeModal('settings-album-modal');
    await refreshAlbums();
    status('Album saved');
  } catch (err) {
    showError('settings-error', String(err));
  }
});

$('album-delete-btn').addEventListener('click', async () => {
  if (!state.editingAlbum) return;
  const path = state.editingAlbum.path;
  if (!confirm(`Delete album "${path}"?\n\nPhotos stay in your library.`)) return;
  try {
    await invoke('delete_album', { path });
    closeModal('settings-album-modal');
    if (state.currentAlbum === path) state.currentAlbum = '';
    await refreshAll();
    status(`Deleted album ${path}`);
  } catch (err) {
    showError('settings-error', String(err));
  }
});

// Add selected photos to an album
$('add-to-album-btn').addEventListener('click', () => {
  const select = $('add-album-select');
  select.innerHTML = '';
  state.albums.forEach((a) => {
    const opt = document.createElement('option');
    opt.value = a.path;
    opt.textContent = a.path;
    select.appendChild(opt);
  });
  $('add-count').textContent = state.selected.size;
  openModal('add-modal');
});

$('add-confirm-btn').addEventListener('click', async () => {
  const path = $('add-album-select').value;
  if (!path) return;
  const n = await invoke('add_to_album', { path, ids: [...state.selected] });
  closeModal('add-modal');
  await refreshAlbums();
  status(`Added ${n} photo(s) to ${path}`);
});

// ----------------------------------------------------------------- filters

$('search').addEventListener('input', debounce((e) => {
  state.filter.text = e.target.value.trim();
  refreshPhotos();
}, 250));

document.querySelectorAll('[data-rating]').forEach((chip) => {
  chip.addEventListener('click', () => {
    document.querySelectorAll('[data-rating]').forEach((c) => c.classList.remove('active'));
    chip.classList.add('active');
    state.filter.minRating = chip.dataset.rating ? Number(chip.dataset.rating) : null;
    refreshPhotos();
  });
});

document.querySelectorAll('[data-flag]').forEach((chip) => {
  chip.addEventListener('click', () => {
    document.querySelectorAll('[data-flag]').forEach((c) => c.classList.remove('active'));
    chip.classList.add('active');
    state.filter.flag = chip.dataset.flag || null;
    refreshPhotos();
  });
});

function debounce(fn, ms) {
  let t;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

// ----------------------------------------------------------------- publish

$('publish-btn').addEventListener('click', async () => {
  await loadPublishTarget();
  showError('publish-error', '');
  $('publish-output').hidden = true;
  openModal('publish-modal');
});

$('settings-btn').addEventListener('click', () => $('publish-btn').click());

async function loadPublishTarget() {
  try {
    const target = await invoke('get_publish_target');
    $('publish-dest').value = target.dest || '';
    $('publish-min-rating').value = target.min_rating ?? '';
  } catch { /* no library open yet */ }
}

$('pick-dest-btn').addEventListener('click', async () => {
  const dir = await openDialog({
    directory: true,
    title: 'Choose the gallery content folder (src/content/albums)',
  });
  if (dir) $('publish-dest').value = dir;
});

$('publish-run-btn').addEventListener('click', async () => {
  const dest = $('publish-dest').value.trim();
  if (!dest) {
    showError('publish-error', 'Choose the gallery content folder first.');
    return;
  }
  const minRating = $('publish-min-rating').value;
  const album = $('publish-album').value || null;

  showError('publish-error', '');
  status('Publishing…');
  try {
    await invoke('set_publish_target', {
      target: { dest, min_rating: minRating ? Number(minRating) : null },
    });
    const results = await invoke('publish', { album });
    const total = results.reduce((n, r) => n + r.photos_copied, 0);
    $('publish-output').hidden = false;
    $('publish-output').textContent = results
      .map((r) => `${r.album_path}: ${r.photos_copied} copied, ${r.photos_skipped} unchanged`)
      .join('\n');
    status(`Published ${results.length} album(s), ${total} file(s) copied`);
  } catch (err) {
    showError('publish-error', String(err));
    status('Publish failed');
  }
});

// -------------------------------------------------------------------- sync
//
// One album at a time, in the direction chosen for that album. Albums with no
// direction set are never touched — not pushed, not pulled, not deleted.

$('sync-btn').addEventListener('click', async () => {
  showError('sync-error', '');
  $('sync-output').hidden = true;
  openModal('sync-modal');
  try {
    $('remote-dir').value = (await invoke('get_remote_dir')) || '';
  } catch { /* no library open */ }
  if ($('remote-dir').value) await refreshRemote();
});

$('pick-remote-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Choose the shared album folder' });
  if (!dir) return;
  $('remote-dir').value = dir;
  await saveRemoteDir();
  await refreshRemote();
});

/// Typing a path by hand is allowed too — persist it on blur.
$('remote-dir').addEventListener('change', () => saveRemoteDir());

async function saveRemoteDir() {
  const dir = $('remote-dir').value.trim();
  if (!dir) return;
  try {
    await invoke('set_remote_dir', { dir });
    showError('sync-error', '');
  } catch (err) {
    showError('sync-error', String(err));
  }
}

$('remote-refresh-btn').addEventListener('click', async () => {
  await saveRemoteDir();
  await refreshRemote();
});

async function refreshRemote() {
  const list = $('remote-list');
  list.innerHTML = '<p class="muted">Reading…</p>';
  try {
    state.remoteAlbums = await invoke('remote_albums');
  } catch (err) {
    list.innerHTML = '';
    showError('sync-error', String(err));
    return;
  }
  showError('sync-error', '');
  renderRemoteList();
}

function renderRemoteList() {
  const list = $('remote-list');
  list.innerHTML = '';

  if (!state.remoteAlbums.length) {
    list.innerHTML = '<p class="muted">No albums here or on the remote yet.</p>';
    return;
  }

  state.remoteAlbums.forEach((album) => {
    const row = document.createElement('div');
    row.className = 'remote-row' + (album.is_collection ? ' is-folder' : '');
    // The list is a tree: indentation is what makes "2026 / weddings / ana"
    // legible as a path rather than three unrelated rows.
    row.style.marginLeft = `${album.depth * 1.1}rem`;

    const info = document.createElement('div');
    info.className = 'remote-info';
    const title = document.createElement('strong');
    title.textContent = (album.is_collection ? '📁 ' : '') + (album.title || album.path);
    info.appendChild(title);

    // The two states a fresh machine cares about, named plainly.
    if (!album.local) {
      const badge = document.createElement('span');
      badge.className = 'badge-new';
      badge.textContent = 'not here yet';
      info.appendChild(badge);
    } else if (!album.remote) {
      const badge = document.createElement('span');
      badge.className = 'badge-new';
      badge.textContent = 'not on server';
      info.appendChild(badge);
    }

    const sub = document.createElement('div');
    sub.className = 'remote-sub muted';
    const where = album.remote
      ? `${album.file_count} file(s) on server`
      : 'local only';
    sub.textContent = album.is_collection
      ? `${album.path} · folder · ${where}`
      : `${album.path} · ${where}`;
    info.appendChild(sub);
    row.appendChild(info);

    const direction = document.createElement('select');
    direction.title = album.is_collection
      ? 'How this folder and everything under it syncs'
      : 'How this album syncs';
    [
      ['', 'Not tracked'],
      ['both', 'Both ways'],
      ['push', 'Push only'],
      ['pull', 'Pull only'],
    ].forEach(([value, label]) => {
      const opt = document.createElement('option');
      opt.value = value;
      opt.textContent = label;
      direction.appendChild(opt);
    });
    direction.value = album.tracked || '';
    direction.addEventListener('change', async () => {
      try {
        if (direction.value) {
          await invoke('track_album', { path: album.path, direction: direction.value });
        } else {
          await invoke('untrack_album', { path: album.path });
        }
        album.tracked = direction.value || null;
        status(direction.value
          ? `${album.path} syncs ${direction.value}`
          : `${album.path} is no longer synced`);
      } catch (err) {
        showError('sync-error', String(err));
        direction.value = album.tracked || '';
      }
    });
    row.appendChild(direction);

    const actions = document.createElement('div');
    actions.className = 'remote-actions';

    const pull = document.createElement('button');
    pull.className = 'btn btn-sm';
    pull.textContent = album.local ? 'Pull' : 'Get';
    pull.disabled = !album.remote;
    pull.title = album.remote
      ? 'Download this path — its folders, itself, and everything under it'
      : 'The server does not have this path';
    pull.addEventListener('click', () => runSync(row, album.path, () =>
      invoke('pull_album', { path: album.path })));
    actions.appendChild(pull);

    const push = document.createElement('button');
    push.className = 'btn btn-sm';
    push.textContent = album.remote ? 'Push' : 'Add to server';
    push.disabled = !album.local;
    push.title = 'Upload this path — its folders, itself, and everything under it';
    push.addEventListener('click', () => runSync(row, album.path, () =>
      pushWithDeleteCheck('push_album', album.path)));
    actions.appendChild(push);

    // Both-ways in one click, only for albums that are tracked both ways.
    if (album.tracked === 'both') {
      const sync = document.createElement('button');
      sync.className = 'btn btn-sm btn-primary';
      sync.textContent = 'Sync';
      sync.title = 'Pull the server’s changes for this whole path, then push this machine’s';
      sync.addEventListener('click', () => runSync(row, album.path, () =>
        pushWithDeleteCheck('sync_album', album.path, { direction: 'both' })));
      actions.appendChild(sync);
    }

    row.appendChild(actions);
    list.appendChild(row);
  });
}

/// Upload an album, then ask about anything the server still holds that the
/// album no longer has.
///
/// The safe pass runs first and reports what it withheld, so the question names
/// the real files. A plan made beforehand would be guessing: it reads the
/// published tree as it was *before* this push rewrote it.
async function pushWithDeleteCheck(command, path, extra = {}) {
  const first = await invoke(command, { path, allowDeletes: false, ...extra });
  const gone = first.withheld_deletes || [];
  if (!gone.length) return first;

  const ok = confirm(
    `${gone.length} file(s) are on the server but no longer in this album:\n\n` +
    `${gone.slice(0, 12).join('\n')}` +
    `${gone.length > 12 ? `\n…and ${gone.length - 12} more` : ''}\n\n` +
    'Remove them from the server? Cancel leaves them online.'
  );
  if (!ok) return first;

  const second = await invoke(command, { path, allowDeletes: true, ...extra });
  // The uploads happened in the first pass; report both halves as one result.
  return {
    ...second,
    files_pushed: (first.files_pushed || 0) + (second.files_pushed || 0),
    pushed: (first.pushed || 0) + (second.pushed || 0),
    pulled: (first.pulled || 0) + (second.pulled || 0),
    withheld_deletes: [],
  };
}

/// Run one sync call with the row disabled, then report and refresh.
async function runSync(row, path, fn) {
  const buttons = [...row.querySelectorAll('button')];
  buttons.forEach((b) => (b.disabled = true));
  showError('sync-error', '');
  status('Syncing…');
  try {
    const outcome = await fn();
    reportOutcome(path, outcome);
    await refreshAll();
    await refreshRemote();
  } catch (err) {
    showError('sync-error', String(err));
    status('Sync failed');
    buttons.forEach((b) => (b.disabled = false));
  }
}

$('sync-all-btn').addEventListener('click', async () => {
  await saveRemoteDir();
  showError('sync-error', '');
  status('Syncing tracked albums…');
  $('sync-all-btn').disabled = true;
  try {
    const results = await invoke('sync_all_tracked', { allowDeletes: false });
    if (!results.length) {
      $('sync-output').hidden = false;
      $('sync-output').textContent =
        'Nothing is tracked yet. Pick a direction for an album first.';
      status('Nothing to sync');
      return;
    }
    $('sync-output').hidden = false;
    $('sync-output').textContent = results
      .map(([path, o]) => `${path}: ${describeOutcome(o)}`)
      .join('\n');
    status(`Synced ${results.length} album(s)`);
    await refreshAll();
    await refreshRemote();
  } catch (err) {
    showError('sync-error', String(err));
    status('Sync failed');
  } finally {
    $('sync-all-btn').disabled = false;
  }
});

function describeOutcome(o) {
  const bits = [];
  // Pull and push outcomes carry different field names; both are shown here.
  if (o.files_pulled) bits.push(`${o.files_pulled} pulled`);
  if (o.photos_imported) bits.push(`${o.photos_imported} catalogued`);
  if (o.files_pushed) bits.push(`${o.files_pushed} pushed`);
  if (o.pulled) bits.push(`${o.pulled} pulled`);
  if (o.pushed) bits.push(`${o.pushed} pushed`);
  if (o.deleted) bits.push(`${o.deleted} deleted on server`);
  if (o.deleted_remote) bits.push(`${o.deleted_remote} deleted on server`);
  if (o.left_alone) bits.push(`${o.left_alone} left alone`);
  if (o.skipped) bits.push(`${o.skipped} skipped`);
  if (o.skipped_unchanged) bits.push(`${o.skipped_unchanged} unchanged`);
  if (o.withheld_deletes?.length) bits.push(`${o.withheld_deletes.length} left on server`);
  if (o.folders_left_alone?.length) {
    bits.push(`${o.folders_left_alone.length} shared folder(s) untouched`);
  }
  if (o.conflicts?.length) bits.push(`${o.conflicts.length} conflict(s)`);
  if (o.failed?.length) bits.push(`${o.failed.length} failed`);
  return bits.join(', ') || 'already up to date';
}

function reportOutcome(path, o) {
  const summary = describeOutcome(o);
  $('sync-output').hidden = false;
  $('sync-output').textContent = `${path}: ${summary}` +
    // Conflicts are never resolved for you — name the files so they can be.
    (o.conflicts?.length
      ? `\n\nChanged on both sides, nothing overwritten:\n  ${o.conflicts.join('\n  ')}`
      : '') +
    // A parent folder someone else configured is not this push's to rewrite.
    (o.folders_left_alone?.length
      ? `\n\nAlready on the server and configured elsewhere — left as they are:\n  ` +
        `${o.folders_left_alone.join('\n  ')}\n` +
        `Push a folder on its own row to change it.`
      : '');
  status(summary);
}

boot();
