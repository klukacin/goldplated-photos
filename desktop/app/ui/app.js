/* Goldplated Photos — desktop UI.
 *
 * Deliberately dependency-free vanilla JS: every operation is one `invoke`
 * into the Rust core, which is where the logic and the tests live. This file
 * only renders and gathers input.
 */

const { invoke, convertFileSrc } = window.__TAURI__.core;
// The webview's own window.confirm() is not usable here: WebKitGTK has no
// script-dialog handler inside a Tauri window, so it returns true without ever
// asking. Everything guarded by it — deleting an album, deleting files off the
// server — would then go ahead unasked. Tauri's dialog plugin is a real native
// prompt on every platform.
const { open: openDialog, confirm: askConfirm } = window.__TAURI__.dialog;
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

function openModal(id) {
  $(id).hidden = false;
  // Put the caret where the user is about to type. Without this the keystrokes
  // go to the document, where digits are rating shortcuts — start typing an
  // album path beginning "2026/" and four photos silently change rating.
  $(id).querySelector('input:not([type=checkbox]), select, textarea')?.focus();
}
function closeModal(id) { $(id).hidden = true; }

/// True while any dialog is up. Triage shortcuts must not fire behind one.
function modalIsOpen() {
  return document.querySelector('.modal:not([hidden])') !== null;
}

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
  // A library named on the command line is already open in the core before the
  // window appears — adopt it rather than asking again.
  try {
    const info = await invoke('library_status');
    localStorage.setItem(LIBRARY_KEY, info.root);
    await enterApp(info);
    return;
  } catch {
    // Nothing open yet; fall through to the remembered library.
  }

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
  await enterApp(info);
}

/// Swap the welcome screen for the app and load its contents.
async function enterApp(info) {
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
  // Camera names come out of EXIF, which is to say out of the files — escape
  // them. A photo whose Model field is "<b>bold</b>" must not restyle the app.
  $('library-stats').innerHTML =
    `${info.photo_count} photos · ${info.album_count} albums` +
    (info.cameras.length ? `<br>${escapeHtml(info.cameras.join(', '))}` : '');
}

// ------------------------------------------------------------------ import

/// How far the running import got. Kept because the summary counts what was
/// imported, not what was on the card — and a stopped import has to be able to
/// say "612 of 2000" rather than implying 612 was the whole job.
let importReach = { processed: 0, total: 0 };

$('import-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Import from folder' });
  if (!dir) return;

  importReach = { processed: 0, total: 0 };
  $('progress').hidden = false;
  $('import-cancel').hidden = false;
  $('import-cancel').disabled = false;
  status('Importing…');
  try {
    const summary = await invoke('import_photos', { dir });
    const done = summary.imported + summary.updated + summary.skipped;
    status(
      // A folder from outside the library is copied in; say where it landed,
      // because that folder is now part of the library's own tree.
      (summary.copied_into ? `Copied ${summary.copied_in} file(s) into ${summary.copied_into} · ` : '') +
      // Never let a stopped import read as a finished one: lead with the fact
      // that it stopped, and with how much of the folder it never reached.
      (summary.cancelled
        ? `Stopped: imported ${done} of ${importReach.total || done} · `
        : '') +
      `imported ${summary.imported} · updated ${summary.updated} · ` +
      `unchanged ${summary.skipped}` +
      (summary.undecodable.length ? ` · ${summary.undecodable.length} without a preview` : '') +
      (summary.failed.length ? ` · ${summary.failed.length} failed` : '')
    );
    await refreshAll();
  } catch (err) {
    status(`Import failed: ${err}`);
  } finally {
    $('progress').hidden = true;
    $('import-cancel').hidden = true;
    $('progress-fill').style.width = '0';
  }
});

$('import-cancel').addEventListener('click', async () => {
  // The button only raises a flag in the core; the import ends at its next
  // file. Disable it so a second click cannot read as "it did not work", and
  // leave it visible until the import actually returns.
  $('import-cancel').disabled = true;
  status('Stopping after the current file…');
  await invoke('cancel_import');
});

listen('import-progress', ({ payload }) => {
  importReach = payload;
  const pct = payload.total ? (payload.processed / payload.total) * 100 : 0;
  $('progress-fill').style.width = `${pct}%`;
  // A cancel already announced is not overwritten by the files still finishing.
  if (!$('import-cancel').disabled) {
    status(`Importing ${payload.processed}/${payload.total} — ${payload.current}`);
  }
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
    // A file no decoder can read — a corrupt JPEG, a RAW format this build
    // does not develop — gets a named tile. A broken-image icon tells the
    // photographer nothing about which file is the problem.
    const undecodable = () => {
      cell.classList.add('undecodable');
      img.remove();
      const note = document.createElement('div');
      note.className = 'cell-note';
      note.textContent = 'No preview';
      cell.prepend(note);
    };
    img.addEventListener('error', undecodable, { once: true });
    invoke('thumbnail_path', { id: photo.id, size: 'small' })
      .then((p) => { img.src = convertFileSrc(p); })
      .catch(undecodable);
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
  // Removing is a change to one album's membership, so it only means anything
  // while that album is the thing being shown.
  $('remove-from-album-btn').disabled = n === 0 || !state.currentAlbum;
  // The develop panel acts on the selection, so it has to say so here too —
  // otherwise Select All leaves it reading "one photo" while a slider drag
  // would repaint the whole shoot.
  updateDevelopScope();
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
  // Keep the inspector's own star row in step with what the grid now shows.
  if (state.cursor >= 0) showInspector(state.photos[state.cursor]);
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
  if (state.cursor >= 0) showInspector(state.photos[state.cursor]);
  status(`Flagged ${ids.length} photo(s): ${flag}`);
}

// Rating and triage keys — the reason this app is faster than a web admin.
document.addEventListener('keydown', (e) => {
  if (e.target.matches('input, select, textarea')) return;
  if ($('app').hidden) return;
  if (modalIsOpen()) return;

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

/// Bumped on every inspector paint. Held arrow keys start several of these at
/// once, and the slowest must not be allowed to finish last and leave one
/// photo's preview beside another photo's adjustments.
let inspectorGeneration = 0;

async function showInspector(photo) {
  const generation = ++inspectorGeneration;
  const panel = $('inspector');
  panel.hidden = false;
  $('inspector-name').textContent = photo.filename;

  try {
    const p = await invoke('thumbnail_path', { id: photo.id, size: 'medium' });
    if (generation !== inspectorGeneration) return;
    $('inspector-img').src = convertFileSrc(p);
  } catch {
    if (generation !== inspectorGeneration) return;
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

  renderDevelop(photo, generation);
}

// ----------------------------------------------------------------- develop
//
// Adjustments are recorded, never written into the original. Every change goes
// to the core, which re-renders the thumbnails under a new render key; the grid
// then simply asks for the thumbnail again and gets the adjusted one.

/** Slider definitions. `ev` is in stops; the rest are the -100..100 the core takes. */
const ADJUSTMENTS = [
  { kind: 'exposure',    label: 'Exposure',    min: -5,   max: 5,   step: 0.05, field: 'ev', unit: ' EV' },
  { kind: 'contrast',    label: 'Contrast',    min: -100, max: 100, step: 1,    field: 'amount' },
  { kind: 'highlights',  label: 'Highlights',  min: -100, max: 100, step: 1,    field: 'amount' },
  { kind: 'shadows',     label: 'Shadows',     min: -100, max: 100, step: 1,    field: 'amount' },
  { kind: 'saturation',  label: 'Saturation',  min: -100, max: 100, step: 1,    field: 'amount' },
  { kind: 'temperature', label: 'Temperature', min: -100, max: 100, step: 1,    field: 'amount' },
  { kind: 'tint',        label: 'Tint',        min: -100, max: 100, step: 1,    field: 'amount' },
];

/// Photos an adjustment applies to: the whole selection, or the cursor alone.
function developTargets() {
  return targetIds();
}

/// How many photos the next adjustment would hit. Rendered in the panel header
/// so a bulk edit can never come as a surprise.
function updateDevelopScope() {
  const n = developTargets().length;
  $('develop-scope').textContent = n > 1 ? `· ${n} photos` : '';
}

async function renderDevelop(photo, generation = inspectorGeneration) {
  const panel = $('develop-sliders');
  let stack;
  try {
    stack = await invoke('photo_edits', { id: photo.id });
  } catch {
    if (generation !== inspectorGeneration) return;
    panel.innerHTML = '';
    return;
  }
  if (generation !== inspectorGeneration) return;

  updateDevelopScope();

  const valueOf = (kind, field) => {
    const op = stack.ops.find((o) => opKind(o) === kind);
    return op ? op[field] : 0;
  };

  // A keyboard nudge on a range input fires `change` on every press, which
  // rebuilds this panel and would otherwise throw the focus away — the next
  // arrow key would then reach the grid and jump to another photo.
  const hadFocus = document.activeElement?.closest?.('.slider-row')?.dataset.kind;

  panel.innerHTML = '';
  for (const adj of ADJUSTMENTS) {
    const value = valueOf(adj.kind, adj.field);

    const row = document.createElement('div');
    row.className = 'slider-row' + (value !== 0 ? ' touched' : '');
    row.dataset.kind = adj.kind;

    const label = document.createElement('label');
    label.textContent = adj.label;
    // Double-clicking a label is the usual way back to neutral.
    label.title = 'Double-click to reset';
    label.addEventListener('dblclick', () => applyAdjustment(adj, 0));

    const input = document.createElement('input');
    input.type = 'range';
    Object.assign(input, { min: adj.min, max: adj.max, step: adj.step, value });

    const out = document.createElement('output');
    out.textContent = formatAmount(value, adj);

    // Update the readout while dragging, but only hit the core on release:
    // every change re-renders thumbnails, which is far too much work per pixel
    // of slider travel.
    input.addEventListener('input', () => {
      out.textContent = formatAmount(Number(input.value), adj);
      row.classList.toggle('touched', Number(input.value) !== 0);
    });
    input.addEventListener('change', () => applyAdjustment(adj, Number(input.value)));

    row.append(label, input, out);
    panel.appendChild(row);
  }

  $('dev-bw').checked = stack.ops.some((o) => opKind(o) === 'black-and-white');
  renderGeometry(stack);

  if (hadFocus) {
    panel.querySelector(`.slider-row[data-kind="${hadFocus}"] input`)?.focus();
  }
}

/// serde tags the enum with `op`, so that field is the kind.
function opKind(op) {
  return op.op;
}

function formatAmount(value, adj) {
  const sign = value > 0 ? '+' : '';
  return adj.unit ? `${sign}${value.toFixed(2)}${adj.unit}` : `${sign}${value}`;
}

// Adjustments are coalesced rather than sent one per keystroke; the scheduling
// itself lives in develop-queue.js, where it can be tested without a window.
const developQueue = createDevelopQueue({
  onError: (err) => status(`Develop failed: ${err}`),
  onSettled: () => afterDevelop(),
});

function applyAdjustment(adj, value) {
  const ids = developTargets();
  if (!ids.length) return;

  developQueue.queue(adj.kind, ids, async () => {
    // A zero is not stored — the core drops the op and the photo returns to its
    // original render key, so this doubles as "clear this adjustment".
    const op = { op: adj.kind, [adj.field]: value };
    const n = await invoke('set_photo_edit', { ids, op });
    status(n ? `${adj.label} ${formatAmount(value, adj)} on ${n} photo(s)` : 'No change');
  });
}

$('dev-bw').addEventListener('change', (e) => {
  const ids = developTargets();
  if (!ids.length) return;
  const on = e.target.checked;
  developQueue.queue('black-and-white', ids, async () => {
    const n = on
      ? await invoke('set_photo_edit', { ids, op: { op: 'black-and-white' } })
      : await invoke('clear_photo_edit', { ids, kind: 'black-and-white' });
    status(n ? `${on ? 'Black & white' : 'Colour'} on ${n} photo(s)` : 'No change');
  });
});

// ------------------------------------------------------- rotate and flip
//
// Geometry is applied before the tone operators, so these buttons decide the
// frame the sliders then work on. They queue like everything else, but they
// coalesce differently: a slider sends a value, so the newest one may simply
// replace the pending one, while these are *relative* — drop a press and the
// photo ends up at an angle nobody asked for. `queueRelative` counts them.

function rotate(quarterTurns) {
  const ids = developTargets();
  if (!ids.length) return;
  developQueue.queueRelative(`rotate${quarterTurns}`, ids, async (targets, presses) => {
    const n = await invoke('rotate_photos', {
      ids: targets,
      quarterTurns: quarterTurns * presses,
    });
    status(n ? `Rotated ${n} photo(s)` : 'No change');
  });
}

function flip(kind, label) {
  const ids = developTargets();
  if (!ids.length) return;
  developQueue.queueRelative(kind, ids, async (targets, presses) => {
    // An even number of presses is back where it started, so there is nothing
    // to send — a flip has no value, only a state to switch.
    if (presses % 2 === 0) return;
    const n = await invoke('toggle_photo_edit', { ids: targets, op: { op: kind } });
    status(n ? `${label} ${n} photo(s)` : 'No change');
  });
}

$('dev-rotate-left').addEventListener('click', () => rotate(-1));
$('dev-rotate-right').addEventListener('click', () => rotate(1));
$('dev-flip-h').addEventListener('click', () => flip('flip-horizontal', 'Flipped left to right,'));
$('dev-flip-v').addEventListener('click', () => flip('flip-vertical', 'Flipped top to bottom,'));

/// Say where the frame currently stands. The buttons are relative, so nothing
/// about them can show that this photo is already lying on its side.
function renderGeometry(stack) {
  const rotated = stack.ops.find((o) => opKind(o) === 'rotate');
  const flippedH = stack.ops.some((o) => opKind(o) === 'flip-horizontal');
  const flippedV = stack.ops.some((o) => opKind(o) === 'flip-vertical');

  $('dev-flip-h').classList.toggle('on', flippedH);
  $('dev-flip-v').classList.toggle('on', flippedV);

  const bits = [];
  if (rotated) bits.push(`${rotated.quarter_turns * 90}°`);
  if (flippedH) bits.push('mirrored');
  if (flippedV) bits.push('upside down');
  $('dev-geometry-state').textContent = bits.join(' · ');
}

$('develop-reset').addEventListener('click', () => {
  const ids = developTargets();
  if (!ids.length) return;
  // A reset supersedes everything still queued for these photos — including
  // presses counted for a rotate that has not been sent yet. Only for these
  // photos: clearing the whole queue would throw away an adjustment someone
  // just made to a different selection.
  developQueue.forgetFor(ids);
  developQueue.queue('reset', ids, async () => {
    const n = await invoke('reset_photo_edits', { ids });
    status(`Reset ${n} photo(s) to the original`);
  });
});

/// Repaint what an adjustment changed: the grid thumbnails and the panel.
async function afterDevelop() {
  renderGrid();
  if (state.cursor >= 0) await showInspector(state.photos[state.cursor]);
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

    // Indent by real depth. A single "is nested" class drew a three-level
    // tree as two, so an album and its own parent sat at the same margin.
    const depth = entry.path.split('/').length - 1;
    const name = document.createElement('span');
    name.className = 'name';
    name.style.paddingLeft = `${depth * 0.85}rem`;
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
  $('set-path').value = entry.path;
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

/// Move the album being edited, once the user has seen what comes with it.
///
/// Returns the path it ended up at — the core lowercases and tidies what was
/// typed, so the rest of the save has to use its answer, not the field's —
/// or null if the move was declined.
async function moveEditedAlbum(from, to) {
  const descendants = state.albums.filter((a) => a.path.startsWith(`${from}/`)).length;
  const sure = await askConfirm(
    (descendants
      ? `${descendants} sub-album${descendants > 1 ? 's move' : ' moves'} with it.\n\n`
      : '') +
    'Photos stay in your library. The published copy moves the next time you publish.',
    {
      title: `Move "${from}" to "${to}"?`,
      kind: 'warning',
      okLabel: 'Move',
    }
  );
  if (!sure) return null;

  const album = await invoke('move_album', { from, to });
  // Anything still pointing at the old path — the sidebar selection, the album
  // the grid is showing — has to follow it, or the app is looking at nothing.
  if (state.currentAlbum === from) {
    state.currentAlbum = album.path;
  } else if (state.currentAlbum.startsWith(`${from}/`)) {
    state.currentAlbum = album.path + state.currentAlbum.slice(from.length);
  }
  return album.path;
}

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
    // The path field renames or moves the album; everything else is a plain
    // field update, and has to be applied where the album now lives.
    let path = state.editingAlbum.path;
    const typed = $('set-path').value.trim();
    if (typed && typed !== path) {
      const moved = await moveEditedAlbum(path, typed);
      if (!moved) return;
      path = moved;
    }
    await invoke('update_album', { path, update });
    closeModal('settings-album-modal');
    await refreshAll();
    status(path === state.editingAlbum.path
      ? 'Album saved'
      : `Moved to ${path} and saved`);
  } catch (err) {
    showError('settings-error', String(err));
  }
});

$('album-delete-btn').addEventListener('click', async () => {
  if (!state.editingAlbum) return;
  const path = state.editingAlbum.path;
  const sure = await askConfirm(`Photos stay in your library.`, {
    title: `Delete album "${path}"?`,
    kind: 'warning',
    okLabel: 'Delete',
  });
  if (!sure) return;
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
  // Collections hold sub-albums, not photos — the gallery would never draw a
  // photo put in one, so it is not offered as a destination.
  const targets = state.albums.filter((a) => !a.is_collection);
  targets.forEach((a) => {
    const opt = document.createElement('option');
    opt.value = a.path;
    opt.textContent = a.path;
    select.appendChild(opt);
  });
  $('add-count').textContent = state.selected.size;
  $('add-empty').hidden = targets.length > 0;
  $('add-confirm-btn').disabled = targets.length === 0;
  openModal('add-modal');
});

$('add-confirm-btn').addEventListener('click', async () => {
  const path = $('add-album-select').value;
  if (!path) return;
  try {
    const n = await invoke('add_to_album', { path, ids: [...state.selected] });
    closeModal('add-modal');
    await refreshAlbums();
    status(`Added ${n} photo(s) to ${path}`);
  } catch (err) {
    showError('add-error', String(err));
  }
});

// Take the selected photos out of the album on screen. An album is a view over
// the library, so this is a membership change and nothing more — the wording
// has to leave no room to read it as a delete.
$('remove-from-album-btn').addEventListener('click', async () => {
  const path = state.currentAlbum;
  const ids = [...state.selected];
  if (!path || !ids.length) return;

  const sure = await askConfirm(
    `The photo${ids.length > 1 ? 's stay' : ' stays'} in your library and in any ` +
    `other album — this only takes ${ids.length > 1 ? 'them' : 'it'} out of "${path}".`,
    {
      title: `Take ${ids.length} photo${ids.length > 1 ? 's' : ''} out of "${path}"?`,
      okLabel: 'Take out of album',
    }
  );
  if (!sure) return;

  try {
    const n = await invoke('remove_from_album', { path, ids });
    await refreshAll();
    status(`Removed ${n} photo(s) from ${path} — still in your library`);
  } catch (err) {
    status(`Could not remove from ${path}: ${err}`);
  }
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
      .map((r) => {
        // Anything not shipped is named here. Silence would read as success.
        const notes = [];
        if (r.missing.length) notes.push(`${r.missing.length} missing from disk`);
        if (r.unrenderable.length) notes.push(`${r.unrenderable.length} without a preview, not published`);
        if (r.removed.length) notes.push(`${r.removed.length} removed`);
        return `${r.album_path}: ${r.photos_copied} copied, ${r.photos_skipped} unchanged` +
          (notes.length ? ` · ${notes.join(' · ')}` : '');
      })
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
    await refreshTokenRow();
  } catch { /* no library open */ }
  if ($('remote-dir').value) await refreshRemote();
});

/// A URL remote authenticates with a token; a folder does not.
function remoteIsUrl() {
  return /^https?:\/\//i.test($('remote-dir').value.trim());
}

async function refreshTokenRow() {
  const row = $('remote-token-row');
  row.hidden = !remoteIsUrl();
  if (row.hidden) return;
  try {
    // The token itself never comes back out of the catalog — only whether one
    // is stored, so the field can say so without echoing a secret.
    $('remote-token').placeholder = (await invoke('has_remote_token'))
      ? 'stored — type to replace'
      : 'paste once — it is kept in the catalog';
  } catch { /* no library open */ }
}

$('remote-dir').addEventListener('input', () => refreshTokenRow());

$('remote-token').addEventListener('change', async () => {
  const token = $('remote-token').value.trim();
  if (!token) return;
  try {
    await invoke('set_remote_token', { token });
    $('remote-token').value = '';
    $('remote-token').placeholder = 'stored — type to replace';
    showError('sync-error', '');
  } catch (err) {
    showError('sync-error', String(err));
  }
});

$('pick-remote-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Choose the shared album folder' });
  if (!dir) return;
  $('remote-dir').value = dir;
  await refreshTokenRow();
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

  const ok = await askConfirm(
    `${gone.slice(0, 12).join('\n')}` +
    `${gone.length > 12 ? `\n…and ${gone.length - 12} more` : ''}\n\n` +
    'Remove them from the server? Cancel leaves them online.',
    {
      title: `${gone.length} file(s) are on the server but no longer in this album`,
      kind: 'warning',
      okLabel: 'Remove from server',
    }
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
    // One album failing no longer stops the others, so the reason has to be
    // shown per album — a count alone would leave the photographer knowing
    // something went wrong and not which album or why.
    $('sync-output').textContent = results
      .map(([path, o]) =>
        [
          `${path}: ${describeOutcome(o)}`,
          ...(o.failed || []).map(([file, why]) => `    ${file} — ${why}`),
        ].join('\n')
      )
      .join('\n');
    const failedAlbums = results.filter(([, o]) => o.failed?.length).length;
    status(
      failedAlbums
        ? `Synced ${results.length - failedAlbums} of ${results.length} album(s)`
        : `Synced ${results.length} album(s)`
    );
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
