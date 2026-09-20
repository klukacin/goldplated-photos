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
// Granted by `opener:default`, which covers revealing a path in the system's
// own file manager.
const { revealItemInDir } = window.__TAURI__.opener;
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

// ------------------------------------------------------------ preferences
//
// How the photographer arranged the window — panels open or shut, in what
// order, sidebars showing, what the overlay says — is read once at boot and
// written on every change. The reading, writing and repair of stored values
// lives in prefs.js, apart from the DOM and under test; this half is only the
// one path from a preference to the screen.

let prefs = loadPrefs(localStorage);

function persistPrefs() {
  savePrefs(localStorage, prefs);
}

/// Human names for the overlay's rows. Keyed by the field names prefs.js knows,
/// so a field added there without a label here is a visible blank rather than a
/// silent omission.
const OVERLAY_LABELS = {
  filename: 'File',
  captured: 'Captured',
  camera: 'Camera',
  lens: 'Lens',
  exposure: 'Exposure',
  dimensions: 'Size',
  size: 'On disk',
  rating: 'Rating',
};

/// Put the whole stored arrangement on screen.
///
/// One function, called at boot and after every change, rather than each
/// setting updating its own corner: with two of them the second way works, and
/// by the fifth the window is in a state no single line of code intended.
function applyPrefs() {
  const container = $('panels');
  // Appending in order sorts them — the nodes are moved, never rebuilt, so a
  // half-dragged slider or an open crop tool survives a reorder.
  for (const name of prefs.panelOrder) {
    const panel = container.querySelector(`[data-panel="${name}"]`);
    if (panel) container.appendChild(panel);
  }
  for (const [name, open] of Object.entries(prefs.panelOpen)) {
    const panel = container.querySelector(`[data-panel="${name}"]`);
    if (panel) panel.open = open;
  }
  updatePanelMoveButtons();

  $('sidebar').hidden = !prefs.sidebars.left;
  $('toggle-left').setAttribute('aria-pressed', String(prefs.sidebars.left));
  $('toggle-left').classList.toggle('on', prefs.sidebars.left);
  $('toggle-right').setAttribute('aria-pressed', String(prefs.sidebars.right));
  $('toggle-right').classList.toggle('on', prefs.sidebars.right);
  $('overlay-btn').setAttribute('aria-pressed', String(prefs.overlay.on));
  $('overlay-btn').classList.toggle('on', prefs.overlay.on);

  updateInspectorVisibility();
  renderInfoOverlay();
}

// ----------------------------------------------------------- panel plumbing

/// An end panel cannot move further that way. Dim rather than gone, so the two
/// buttons stay where the hand expects them.
function updatePanelMoveButtons() {
  const panels = [...$('panels').querySelectorAll('.panel')];
  panels.forEach((panel, i) => {
    panel.querySelector('[data-move="up"]').disabled = i === 0;
    panel.querySelector('[data-move="down"]').disabled = i === panels.length - 1;
  });
}

function movePanel(name, delta) {
  const order = prefs.panelOrder;
  const from = order.indexOf(name);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= order.length) return;
  [order[from], order[to]] = [order[to], order[from]];
  persistPrefs();
  applyPrefs();
}

// The buttons live inside <summary>, where a plain click would also collapse
// the panel being moved.
$('panels').addEventListener('click', (e) => {
  const button = e.target.closest('.panel-move');
  if (!button) return;
  e.preventDefault();
  e.stopPropagation();
  movePanel(button.closest('.panel').dataset.panel, button.dataset.move === 'up' ? -1 : 1);
});

// `toggle` does not bubble, so it is caught on the way down instead.
$('panels').addEventListener(
  'toggle',
  (e) => {
    const panel = e.target.closest?.('.panel');
    if (!panel) return;
    prefs.panelOpen[panel.dataset.panel] = panel.open;
    persistPrefs();
    // Closing develop is the end of previewing, so let the core drop the
    // decoded pixels it was rendering proxies from. Reopening pays one decode.
    if (panel.dataset.panel === 'develop' && !panel.open) {
      invoke('release_preview').catch(() => {});
    }
  },
  true,
);

// ---------------------------------------------------------- info overlay
//
// Over the picture rather than beside it, so it can be read while culling
// without the eye leaving the photograph — and over the *enlarged* picture too,
// because it sits inside the frame the loupe moves.

/// The rows this photo can offer, in the order the overlay draws them.
///
/// `null` means "leave it out". The inspector's own list writes an em dash for
/// a value the file does not carry, which is right in a panel beside the photo
/// and wrong in an overlay on top of it: a frame with no EXIF would be covered
/// by a column of dashes. So the shared formatters' em dash is mapped back to
/// nothing here.
function overlayRows(photo) {
  const orNothing = (text) => (text && text !== '—' ? text : null);
  return {
    filename: photo.filename,
    captured: photo.captured_at || null,
    camera: [photo.camera_make, photo.camera_model].filter(Boolean).join(' ') || null,
    lens: photo.lens || null,
    exposure: orNothing(formatExposure(photo)),
    dimensions: photo.width && photo.height ? `${photo.width} × ${photo.height}` : null,
    size: orNothing(formatBytes(photo.file_size)),
    rating: photo.rating ? '★'.repeat(photo.rating) : null,
  };
}

function renderInfoOverlay() {
  const el = $('info-overlay');
  const photo = state.cursor >= 0 ? state.photos[state.cursor] : null;
  if (!prefs.overlay.on || !photo) {
    el.hidden = true;
    return;
  }
  const values = overlayRows(photo);
  // A field with nothing in it is left out rather than shown as a dash: the
  // overlay covers the photograph, so every row has to earn its place.
  const rows = prefs.overlay.fields
    .filter((f) => values[f])
    .map((f) => `<dt>${escapeHtml(OVERLAY_LABELS[f] ?? f)}</dt><dd>${escapeHtml(String(values[f]))}</dd>`);
  if (!rows.length) {
    el.hidden = true;
    return;
  }
  el.innerHTML = `<dl>${rows.join('')}</dl>`;
  el.hidden = false;
  positionInfoOverlay();
}

/// Inset from the corner of the *picture*, in pixels.
const OVERLAY_INSET = 12;

/// Lay the overlay on the photograph rather than on the box around it.
///
/// Same problem the crop rectangle has, and the same solution: `object-fit:
/// contain` centres the picture and leaves bars on one axis, so an overlay
/// pinned to the element sits in the black band above a wide photo in a tall
/// frame — floating over nothing, which is where it first appeared in the
/// loupe.
function positionInfoOverlay() {
  const el = $('info-overlay');
  if (el.hidden) return;
  const pic = drawnPictureBox();
  if (!pic) return;
  el.style.left = `${pic.left + OVERLAY_INSET}px`;
  el.style.top = `${pic.top + OVERLAY_INSET}px`;
  el.style.maxWidth = `${Math.max(120, pic.width - OVERLAY_INSET * 2)}px`;
}

function toggleOverlay(on = !prefs.overlay.on) {
  prefs.overlay.on = on;
  persistPrefs();
  applyPrefs();
}

$('overlay-btn').addEventListener('click', () => toggleOverlay());

// -------------------------------------------------------- sidebar toggles

function toggleSidebar(side, on = !prefs.sidebars[side]) {
  prefs.sidebars[side] = on;
  persistPrefs();
  applyPrefs();
  // The picture's box changed width, and both overlays are measured against it.
  repositionOverlays();
}

$('toggle-left').addEventListener('click', () => toggleSidebar('left'));
$('toggle-right').addEventListener('click', () => toggleSidebar('right'));

// ------------------------------------------------------ settings dialog

/// Build the overlay's field checkboxes from what prefs.js says exists, so the
/// dialog cannot drift out of step with what the overlay can actually draw.
function renderOverlayFieldChecks() {
  const box = $('pref-overlay-fields');
  box.innerHTML = '';
  for (const field of OVERLAY_FIELDS) {
    const label = document.createElement('label');
    label.className = 'check';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = prefs.overlay.fields.includes(field);
    input.addEventListener('change', () => {
      // Rebuilt from the canonical list rather than pushed and spliced, so the
      // stored order always matches the order the overlay draws.
      const chosen = new Set(prefs.overlay.fields);
      input.checked ? chosen.add(field) : chosen.delete(field);
      prefs.overlay.fields = OVERLAY_FIELDS.filter((f) => chosen.has(f));
      persistPrefs();
      renderInfoOverlay();
    });
    label.append(input, ` ${OVERLAY_LABELS[field] ?? field}`);
    box.appendChild(label);
  }
}

function openSettings() {
  $('pref-overlay-on').checked = prefs.overlay.on;
  $('pref-left').checked = prefs.sidebars.left;
  $('pref-right').checked = prefs.sidebars.right;
  renderOverlayFieldChecks();
  openModal('settings-modal');
}

$('settings-btn').addEventListener('click', openSettings);
$('pref-overlay-on').addEventListener('change', (e) => toggleOverlay(e.target.checked));
$('pref-left').addEventListener('change', (e) => toggleSidebar('left', e.target.checked));
$('pref-right').addEventListener('change', (e) => toggleSidebar('right', e.target.checked));

$('pref-reset-btn').addEventListener('click', () => {
  prefs = defaultPrefs();
  persistPrefs();
  applyPrefs();
  // The dialog is showing the values that just changed underneath it.
  openSettings();
  status('Settings back to their defaults');
});

$('shortcuts-btn').addEventListener('click', () => openModal('shortcuts-modal'));

// ------------------------------------------------------------- fullscreen
//
// The window's own fullscreen, not the loupe: the two compose, and the pair of
// them is what "show me this photograph" actually means.

async function toggleFullscreen() {
  try {
    const on = await invoke('toggle_fullscreen');
    status(on ? 'Fullscreen — F to leave' : 'Fullscreen off');
  } catch (err) {
    status(`Fullscreen failed: ${err}`);
  }
}

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
  if (e.key !== 'Escape') return;
  // A dialog is in front of everything, so it goes first; only once nothing is
  // open does Escape mean "stop framing".
  if (modalIsOpen()) {
    document.querySelectorAll('.modal:not([hidden])').forEach((m) => (m.hidden = true));
  } else if (cropMode.active) {
    cancelCrop();
  } else if (loupe.active) {
    // Last, because framing happens inside the loupe: one Escape puts the crop
    // tool away and leaves the photograph up, a second goes back to the grid.
    setLoupe(false);
  }
});

// ----------------------------------------------------------------- startup

async function boot() {
  // A library named on the command line is already open in the core before the
  // window appears — adopt it rather than asking again.
  try {
    const info = await invoke('library_status');
    localStorage.setItem(LIBRARY_KEY, info.root);
    rememberLibrary(localStorage, info.root);
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
  // The path the core resolved, not the one that was asked for — a picker can
  // hand over a symlink or a trailing slash, and the switcher has to offer back
  // exactly what would open again.
  localStorage.setItem(LIBRARY_KEY, info.root);
  rememberLibrary(localStorage, info.root);
  await enterApp(info);
}

// ---------------------------------------------------------------- libraries
//
// A photographer has more than one: this shoot's card, last year's archive, the
// drive that is only plugged in sometimes. Remembering the last one is not a
// switcher — the list of the last few lives in prefs.js, along with the repair
// of whatever ends up stored there.

/// Open another library, leaving nothing of the last one behind.
///
/// Ids, album paths and the cursor all mean something only within one catalog,
/// so every one of them is dropped rather than carried across. The album filter
/// especially: a path that existed in the old library would quietly filter the
/// new one down to nothing, and look like an empty import.
async function switchLibrary(path) {
  showError('library-error', '');
  state.currentAlbum = '';
  state.selected.clear();
  state.cursor = -1;
  setLoupe(false);
  // The decoded preview pixels belong to a photo in the library being left.
  invoke('release_preview').catch(() => {});
  try {
    await openLibrary(path);
    closeModal('library-modal');
    status(`Opened ${path}`);
  } catch (err) {
    showError('library-error', String(err));
    // The list keeps a folder that has moved or is on an unplugged drive, so
    // that plugging it back in is all that is needed — but say so plainly
    // rather than leaving a row that silently does nothing.
    renderLibraryModal();
  }
}

function renderLibraryModal() {
  const current = localStorage.getItem(LIBRARY_KEY) || '';
  $('library-path').textContent = current || 'No library open';
  $('library-reveal').disabled = !current;
  $('library-prune').disabled = !current;

  const others = loadRecent(localStorage).filter((path) => path !== current);
  const list = $('library-recent');
  list.innerHTML = '';
  $('library-recent-empty').hidden = others.length > 0;

  for (const path of others) {
    const row = document.createElement('div');
    row.className = 'library-row';

    const open = document.createElement('button');
    open.className = 'library-switch';
    // The name identifies it; the path is context and may truncate. The whole
    // path is on the tooltip, where truncation cannot hide it.
    open.title = path;
    open.innerHTML = `<strong>${escapeHtml(libraryName(path))}</strong><span class="path">${escapeHtml(path)}</span>`;
    open.addEventListener('click', () => switchLibrary(path));

    const forget = document.createElement('button');
    forget.className = 'library-forget';
    forget.textContent = '✕';
    forget.title = 'Forget this library. The folder and its photos are untouched.';
    forget.addEventListener('click', (e) => {
      e.stopPropagation();
      forgetLibrary(localStorage, path);
      renderLibraryModal();
    });

    row.append(open, forget);
    list.appendChild(row);
  }
}

/// The last segment of a path — what the folder is called.
function libraryName(path) {
  return path.split('/').filter(Boolean).pop() || path;
}

$('library-btn').addEventListener('click', () => {
  renderLibraryModal();
  openModal('library-modal');
});

$('library-open-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Choose a photo folder' });
  if (dir) await switchLibrary(dir);
});

$('library-reveal').addEventListener('click', async () => {
  const current = localStorage.getItem(LIBRARY_KEY);
  if (!current) return;
  try {
    await revealItemInDir(current);
  } catch (err) {
    showError('library-error', String(err));
  }
});

$('library-prune').addEventListener('click', async () => {
  // It deletes nothing on disk, but it does change what the catalog holds, and
  // a photographer looking at an unplugged drive would lose the whole library
  // from the grid without being asked.
  const ok = await askConfirm(
    'Drop catalog entries whose photo files are no longer there?\n\n' +
      'Nothing on disk is deleted. If a drive is unplugged, plug it back in first — ' +
      'its photos count as missing.',
    { title: 'Remove missing photos', kind: 'warning' },
  );
  if (!ok) return;
  try {
    const removed = await invoke('prune_missing');
    status(removed ? `Removed ${removed} missing photo(s)` : 'Nothing was missing');
    await refreshAll();
  } catch (err) {
    showError('library-error', String(err));
  }
});

/// Swap the welcome screen for the app and load its contents.
async function enterApp(info) {
  $('welcome').hidden = true;
  $('app').hidden = false;
  // Before the first paint, so the window comes up arranged the way it was left
  // rather than snapping into it a moment later.
  applyPrefs();
  renderStatus(info);
  await Promise.all([refreshAlbums(), refreshPhotos()]);
  await loadPublishTarget();
}

function renderStatus(info) {
  // The sidebar names the library; the titlebar names the application.
  $('library-name').textContent = libraryName(info.root);
  $('library-name').title = info.root;
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
  // A filter that matches nothing leaves no photo to enlarge, and the loupe
  // would otherwise stay up showing one that is no longer in the grid.
  if (loupe.active && state.cursor < 0) setLoupe(false);
  renderGrid();
  updateSelectionUI();
  // The cursor is already on the first photo, and the rating keys already act
  // on it — so show it. Leaving this out put an empty inspector on screen at
  // boot: no filename, no preview, no stars, no sliders, while pressing `3`
  // silently rated the photo it was not showing. It went unnoticed for as long
  // as the panel stayed hidden until the first click.
  if (state.cursor >= 0) {
    await showInspector(state.photos[state.cursor]);
  } else {
    // Nothing matches: nothing to inspect, and an overlay left up would be
    // describing a photo that is no longer on screen.
    updateInspectorVisibility();
    renderInfoOverlay();
  }
}

async function refreshAll() {
  renderStatus(await invoke('library_status'));
  await Promise.all([refreshAlbums(), refreshPhotos()]);
}

function renderGrid() {
  const grid = $('grid');
  grid.innerHTML = '';
  $('empty-state').hidden = state.photos.length > 0 || loupe.active;

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

    // The way out of the grid, and `dblclick` in the loupe is the way back —
    // both wired here so the gesture is symmetric.
    cell.addEventListener('dblclick', (e) => {
      e.preventDefault();
      state.cursor = index;
      setLoupe(true);
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
  // Keep the inspector's own star row — and the overlay, which can carry the
  // rating too — in step with what the grid now shows.
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
  } else if (e.key === 'e' || e.key === 'E' || e.key === 'Enter') {
    e.preventDefault();
    setLoupe(!loupe.active);
  } else if (e.key === 'i' || e.key === 'I') {
    e.preventDefault();
    toggleOverlay();
  } else if (e.key === 'f' || e.key === 'F') {
    e.preventDefault();
    toggleFullscreen();
  } else if (e.key === '[') {
    e.preventDefault();
    toggleSidebar('left');
  } else if (e.key === ']') {
    e.preventDefault();
    toggleSidebar('right');
  } else if (e.key === '?') {
    e.preventDefault();
    openModal('shortcuts-modal');
  } else if (e.key === ',') {
    e.preventDefault();
    openSettings();
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

// ------------------------------------------------------------------- loupe
//
// A 300px sidebar preview is enough to tell one frame from another and not
// nearly enough to judge one. The loupe gives the photograph the whole of the
// main area while leaving every control exactly where it was: the stars, the
// sliders, the crop tool and the rating keys all still act on the same photo,
// because none of them ever addressed the preview — they address the cursor.
//
// It does that by *moving* #inspector-frame onto the stage rather than drawing
// a second copy. A second copy would mean a second crop overlay over different
// pixels at a different scale, and the two would disagree about which part of
// the photograph the rectangle names.

const loupe = { active: false };

/// Switch between the grid and the single-photo view.
function setLoupe(on) {
  // Nothing to enlarge: leave the grid up rather than show an empty stage.
  if (on && state.cursor < 0) return;
  if (loupe.active === on) return;
  loupe.active = on;

  const frame = $('inspector-frame');
  if (on) {
    $('loupe').appendChild(frame);
  } else {
    // Back to the top of the inspector, above the filename and the stars.
    $('inspector').insertBefore(frame, $('inspector').firstChild);
  }
  frame.classList.toggle('in-loupe', on);
  $('loupe').hidden = !on;
  $('grid').hidden = on;
  $('loupe-btn').classList.toggle('on', on);
  $('loupe-btn').textContent = on ? '⤡ Grid' : '⤢ Enlarge';

  // The preview is a different size on each side of this, and the loupe wants
  // more pixels than the sidebar did — so repaint before re-laying the crop.
  if (state.cursor >= 0) showInspector(state.photos[state.cursor]);
  renderGrid();
  repositionOverlays();
}

$('loupe-btn').addEventListener('click', () => setLoupe(!loupe.active));

// Double-click enlarged a photo from the grid; double-click puts it back.
//
// On the picture itself, because there is nowhere else: the frame is sized to
// fill the stage, so a guard of `e.target === stage` never matched and the
// gesture silently did nothing. The one case it must not fire in is framing —
// the crop rectangle is dragged on these same pixels, and a double-click while
// adjusting a corner would throw away the loupe mid-crop.
$('loupe').addEventListener('dblclick', () => {
  if (cropMode.active) return;
  setLoupe(false);
});

// --------------------------------------------------------------- inspector

/// Bumped on every inspector paint. Held arrow keys start several of these at
/// once, and the slowest must not be allowed to finish last and leave one
/// photo's preview beside another photo's adjustments.
let inspectorGeneration = 0;

async function showInspector(photo) {
  const generation = ++inspectorGeneration;
  // Whatever was queued was for the photo being left.
  livePreview.next = null;
  // Whether the panel is up is now a preference as well as a question of
  // having something to show, so it is decided in one place.
  updateInspectorVisibility();
  $('inspector-name').textContent = photo.filename;

  // Framing is about one picture. Moving to another leaves the rectangle
  // describing a frame that is no longer on screen, so put back whatever crop
  // the photo being left had and close the tool.
  if (cropMode.active && photo.id !== cropMode.photoId) cancelCrop();

  try {
    const p = await invoke('thumbnail_path', {
      id: photo.id,
      size: loupe.active ? 'large' : 'medium',
    });
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

  // Triage beside the rating, because it is the same decision made twice a
  // second — and it shows the current state rather than only offering the
  // three choices, so a glance answers "have I done this one?".
  const flags = $('inspector-flags');
  flags.innerHTML = '';
  for (const [flag, label] of [['pick', '⚑ Pick'], ['none', '○ Unpick'], ['reject', '✕ Reject']]) {
    const button = document.createElement('button');
    button.className = 'btn btn-sm' + (flag === 'reject' ? ' reject' : '');
    button.classList.toggle('on', photo.flag === flag);
    button.textContent = label;
    // Acts on the selection, exactly as P/X/U do — the panel and the keys must
    // not disagree about what "this photo" means.
    button.addEventListener('click', () => applyFlag(flag));
    flags.appendChild(button);
  }

  renderInfoOverlay();

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

    // Dragging previews; releasing commits. The core is hit either way, but
    // with very different calls — see the live-preview section below for why
    // the committing one cannot be used per pixel of slider travel.
    input.addEventListener('input', () => {
      out.textContent = formatAmount(Number(input.value), adj);
      row.classList.toggle('touched', Number(input.value) !== 0);
      requestPreview(adj, Number(input.value));
    });
    input.addEventListener('change', () => applyAdjustment(adj, Number(input.value)));
    // The label has always done this; the slider is the bigger target and the
    // one the hand is already on. `input` fires on the way, so the readout and
    // the preview follow without a second case.
    input.addEventListener('dblclick', () => {
      input.value = 0;
      input.dispatchEvent(new Event('input'));
      applyAdjustment(adj, 0);
    });

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

// ------------------------------------------------------- live preview
//
// A slider has to answer while the hand is still moving. Committing cannot do
// that: `set_photo_edit` writes the stack, develops the frame at full size and
// rebuilds three thumbnails — measured at ~200 ms on a 24 MP file, and a cache
// entry for every value slid past on the way to the one that was wanted.
//
// So a drag asks for `preview_photo_edit` instead, which renders a small proxy
// from pixels the core keeps decoded and writes nothing at all: ~4 ms once the
// first call has paid for the decode. Release still calls the committing one,
// and the true render replaces the proxy — so what a drag shows is a proxy of
// exactly the same stack, differing only in resolution.
//
// One request in flight at a time, with the newest value kept. Firing per
// `input` event would queue a dozen renders behind a fast drag and land the
// preview several values in the past; dropping the ones in between and always
// finishing on the latest keeps it honest and self-pacing on any machine.

const livePreview = { inFlight: false, next: null };

/// Show what `value` would do, without committing it.
///
/// The frame on screen is the one under the cursor, so that is the only frame
/// worth previewing — and only when the adjustment would actually reach it. A
/// selection made with cmd-click can leave the cursor on a photo that is not in
/// it; previewing another of the targets there would put one photograph's
/// pixels in the other one's frame, and previewing the cursor's would show a
/// change the release is not going to make. So neither: show nothing, and let
/// the commit repaint the grid.
function requestPreview(adj, value) {
  if (state.cursor < 0) return;
  const id = state.photos[state.cursor]?.id;
  if (id == null || !developTargets().includes(id)) return;
  livePreview.next = { id, op: { op: adj.kind, [adj.field]: value } };
  drainPreview();
}

async function drainPreview() {
  if (livePreview.inFlight || !livePreview.next) return;
  livePreview.inFlight = true;
  try {
    while (livePreview.next) {
      const { id, op } = livePreview.next;
      livePreview.next = null;
      // The photo can change under a slow render — a held arrow key moves the
      // cursor. Painting the result then would put one photo's pixels beside
      // another photo's adjustments, which is the bug the generation counter
      // exists to prevent, so it is checked here too.
      const generation = inspectorGeneration;
      const preview = await invoke('preview_photo_edit', { id, op });
      if (generation !== inspectorGeneration) return;
      $('inspector-img').src = preview.dataUrl;
    }
  } catch (err) {
    // A preview is a courtesy. Losing one must not interrupt the drag or
    // overwrite the status line with noise the commit will report anyway.
  } finally {
    livePreview.inFlight = false;
    // A value that arrived while the last render was finishing.
    if (livePreview.next) drainPreview();
  }
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
  // An applied crop is carried through the turn by the core, but the rectangle
  // being dragged right now is not: it is fractions of the frame on screen, and
  // that frame is about to become a different shape. Close the tool rather than
  // commit it to a framing the photographer never saw. Cancel queues the
  // restore first, so the crop goes back before the turn is sent.
  if (cropMode.active) cancelCrop();
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
  if (cropMode.active) cancelCrop();
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
  // Read the whole framing rather than looking for particular ops. The core
  // stores which way up the photograph is as one mirror plus a number of
  // quarter turns — the eight ways a rectangle can be set down — because turns
  // and mirrors do not commute and editing the ops where they lay made the
  // buttons move the picture the wrong way. So a top-to-bottom flip is kept as
  // a left-to-right one and a half turn, and asking "is there a flip-vertical
  // op" would answer no about a photograph that is plainly upside down.
  let mirrored = false;
  let turns = 0;
  for (const op of stack.ops) {
    switch (opKind(op)) {
      case 'rotate':
        turns = (turns + op.quarter_turns) % 4;
        break;
      case 'flip-horizontal':
        mirrored = !mirrored;
        turns = (4 - turns) % 4;
        break;
      case 'flip-vertical':
        mirrored = !mirrored;
        turns = (6 - turns) % 4;
        break;
    }
  }

  // Both buttons light on a mirrored frame: either one will take the mirror
  // off again, and neither owns an axis of its own any more.
  $('dev-flip-h').classList.toggle('on', mirrored);
  $('dev-flip-v').classList.toggle('on', mirrored);

  currentCrop = stack.ops.find((o) => opKind(o) === 'crop') || null;
  syncCropUi();

  const bits = [];
  if (turns) bits.push(`${turns * 90}°`);
  if (mirrored) bits.push('mirrored');
  if (currentCrop) bits.push('cropped');
  $('dev-geometry-state').textContent = bits.join(' · ');
}

// ------------------------------------------------------------------- crop
//
// The core takes a crop as four fractions of the frame *after* rotation and
// flips — which is exactly the frame the inspector preview shows, so the
// rectangle can be read straight off the picture on screen. Two things make
// that less obvious than it sounds, and both name the wrong part of the
// photograph when got wrong:
//
//  - The preview is `object-fit: contain` in a box capped at 40vh, so the
//    drawn picture is usually smaller than the element and letterboxed on one
//    axis. Fractions of the element box are not fractions of the photograph.
//  - A photo that already carries a crop *renders cropped*, so its preview is
//    no longer the frame those fractions are measured against. Opening the
//    tool therefore takes the crop off first and reopens the rectangle where
//    it stood; Cancel puts it back.

/// The smallest side the rectangle may be dragged to, as a fraction of the
/// frame. A zero-width crop is not a picture, and the core would clamp it to
/// something arbitrary rather than refuse it.
const MIN_CROP = 0.02;

/// The crop on the photo the inspector is showing, as last read from the core.
/// Read by `syncCropUi`, so the Crop button carries the state the way the flip
/// buttons do — nothing about a cropped photo looks wrong on its own.
let currentCrop = null;

const cropMode = {
  active: false,
  /// Which photo the rectangle was drawn on. Everything else may be selected
  /// too, but this is the one the user is looking at.
  photoId: null,
  ids: [],
  /// The rectangle being dragged, in fractions of the whole frame.
  rect: null,
  /// The crop that was on the photo when the tool opened, for Cancel.
  restore: null,
};

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

/// Where the picture actually sits inside the img element, in the element's own
/// coordinates. `object-fit: contain` centres it and leaves bars on one axis.
function drawnPictureBox() {
  const img = $('inspector-img');
  const { naturalWidth: nw, naturalHeight: nh } = img;
  const box = img.getBoundingClientRect();
  if (!nw || !nh || !box.width || !box.height) return null;
  const scale = Math.min(box.width / nw, box.height / nh);
  const w = nw * scale;
  const h = nh * scale;
  return { left: (box.width - w) / 2, top: (box.height - h) / 2, width: w, height: h };
}

/// Lay the overlay over the picture and the rectangle over the overlay. Called
/// again on every drag, on every preview that loads, and on resize: 40vh is a
/// window measurement, so the picture changes size without the photo changing.
function positionCropOverlay() {
  if (!cropMode.active) return;
  const overlay = $('crop-overlay');
  const pic = drawnPictureBox();
  if (!pic) {
    overlay.hidden = true;
    return;
  }
  overlay.hidden = false;
  overlay.style.left = `${pic.left}px`;
  overlay.style.top = `${pic.top}px`;
  overlay.style.width = `${pic.width}px`;
  overlay.style.height = `${pic.height}px`;

  const r = cropMode.rect;
  const rect = $('crop-rect');
  rect.style.left = `${r.x * pic.width}px`;
  rect.style.top = `${r.y * pic.height}px`;
  rect.style.width = `${r.w * pic.width}px`;
  rect.style.height = `${r.h * pic.height}px`;
  $('crop-readout').textContent = `${Math.round(r.w * 100)}% × ${Math.round(r.h * 100)}%`;
}

function syncCropUi() {
  $('crop-overlay').hidden = !cropMode.active;
  $('crop-actions').hidden = !cropMode.active;
  $('crop-remove').hidden = !cropMode.restore;
  $('dev-crop').classList.toggle('on', cropMode.active || Boolean(currentCrop));
}

function enterCrop() {
  // The switch's second half (see features.js): the button is hidden at boot,
  // but a shortcut or a stray listener could still land here.
  if (!FEATURES.crop) return;
  const ids = developTargets();
  if (!ids.length || state.cursor < 0) return;

  cropMode.active = true;
  cropMode.photoId = state.photos[state.cursor].id;
  cropMode.ids = ids;
  cropMode.restore = currentCrop;
  cropMode.rect = currentCrop
    ? { x: currentCrop.x, y: currentCrop.y, w: currentCrop.w, h: currentCrop.h }
    : { x: 0, y: 0, w: 1, h: 1 };

  // Take the crop off while framing so the preview is the whole frame again.
  // The queue then repaints the inspector, and the `load` handler puts the
  // rectangle back over the picture at its new size.
  if (cropMode.restore) {
    developQueue.queue('crop', ids, async () => {
      await invoke('clear_photo_edit', { ids, kind: 'crop' });
    });
  }
  syncCropUi();
  positionCropOverlay();
  status('Drag the rectangle or its corners, then Apply.');
}

/// Close the tool. Callers decide what, if anything, to send to the core.
function closeCrop() {
  const { ids, restore } = cropMode;
  cropMode.active = false;
  cropMode.photoId = null;
  cropMode.ids = [];
  cropMode.rect = null;
  cropMode.restore = null;
  syncCropUi();
  return { ids, restore };
}

function applyCrop() {
  const rect = { ...cropMode.rect };
  const { ids } = closeCrop();
  developQueue.queue('crop', ids, async () => {
    // A rectangle still covering the whole frame is an identity op, which the
    // core drops — so dragging nothing and pressing Apply removes the crop
    // rather than storing a no-op and a pointless new render key.
    const n = await invoke('set_photo_edit', { ids, op: { op: 'crop', ...rect } });
    status(n ? `Cropped ${n} photo(s)` : 'No change');
  });
}

function cancelCrop() {
  const { ids, restore } = closeCrop();
  if (!restore) {
    // Nothing was taken off, so nothing has to go back — and no repaint either.
    status('Crop cancelled');
    return;
  }
  developQueue.queue('crop', ids, async () => {
    const { x, y, w, h } = restore;
    await invoke('set_photo_edit', { ids, op: { op: 'crop', x, y, w, h } });
    status('Crop left as it was');
  });
}

function removeCrop() {
  const { ids } = closeCrop();
  // Opening the tool already cleared it; this clear is what makes leaving it
  // cleared deliberate rather than a side effect nobody asked for.
  developQueue.queue('crop', ids, async () => {
    await invoke('clear_photo_edit', { ids, kind: 'crop' });
    status(`Removed the crop from ${ids.length} photo(s)`);
  });
}

$('dev-crop').addEventListener('click', () => (cropMode.active ? cancelCrop() : enterCrop()));
$('crop-apply').addEventListener('click', applyCrop);
$('crop-cancel').addEventListener('click', cancelCrop);
$('crop-remove').addEventListener('click', removeCrop);

// The crop switch (features.js). Hiding the button is the visible half; the
// early return in enterCrop is what holds when something else still reaches
// for the tool. The action row and overlay start hidden in the markup and only
// enterCrop un-hides them, so cutting the entry cuts everything. A crop a
// photo already carries still renders — the switch removes the tool, not the
// photographer's framing.
if (!FEATURES.crop) $('dev-crop').hidden = true;

// A new preview is a new picture box, and while cropping the rectangle has to
// follow it. Without this the overlay keeps the previous photo's dimensions
// after the tool takes an existing crop off.
/// Everything laid over the picture, laid again. The crop rectangle and the
/// info overlay are both measured against the drawn photograph, so they move
/// together or not at all.
function repositionOverlays() {
  positionCropOverlay();
  positionInfoOverlay();
}

$('inspector-img').addEventListener('load', repositionOverlays);
window.addEventListener('resize', repositionOverlays);

$('crop-overlay').addEventListener('pointerdown', (e) => {
  if (!cropMode.active) return;
  // Nothing inside the overlay is a native gesture, and the webview's default
  // for a press-and-drag is to select text.
  e.preventDefault();

  const handle = e.target.dataset.handle || null;
  // A press on the dimmed area is not a drag. Treating it as one would jump the
  // frame to wherever the pointer happened to land.
  if (!handle && !e.target.closest('.crop-rect')) return;

  const overlay = $('crop-overlay');
  const box = overlay.getBoundingClientRect();
  if (!box.width || !box.height) return;

  const at = (ev) => ({
    x: clamp((ev.clientX - box.left) / box.width, 0, 1),
    y: clamp((ev.clientY - box.top) / box.height, 0, 1),
  });
  const start = { ...cropMode.rect };
  const origin = at(e);

  // Capture, or a drag that leaves the small preview stops reporting and the
  // rectangle freezes halfway through the gesture.
  overlay.setPointerCapture(e.pointerId);
  const onMove = (ev) => {
    const p = at(ev);
    cropMode.rect = handle
      ? resizeCropRect(start, handle, p)
      : moveCropRect(start, p.x - origin.x, p.y - origin.y);
    positionCropOverlay();
  };
  const onUp = () => {
    overlay.removeEventListener('pointermove', onMove);
    overlay.removeEventListener('pointerup', onUp);
    overlay.removeEventListener('pointercancel', onUp);
  };
  overlay.addEventListener('pointermove', onMove);
  overlay.addEventListener('pointerup', onUp);
  overlay.addEventListener('pointercancel', onUp);
});

/// Slide the rectangle, keeping every edge on the picture. A crop that hangs
/// off the frame is not one the core can express; it would clamp it to some
/// other rectangle and crop a part of the photograph nobody chose.
function moveCropRect(start, dx, dy) {
  return {
    x: clamp(start.x + dx, 0, 1 - start.w),
    y: clamp(start.y + dy, 0, 1 - start.h),
    w: start.w,
    h: start.h,
  };
}

/// Drag one corner; the opposite one is pinned. Clamping against that pinned
/// corner is what stops the rectangle collapsing to nothing or turning inside
/// out as the pointer crosses it.
function resizeCropRect(start, handle, p) {
  const west = handle[1] === 'w';
  const north = handle[0] === 'n';
  const right = start.x + start.w;
  const bottom = start.y + start.h;

  const x0 = west ? Math.min(p.x, right - MIN_CROP) : start.x;
  const x1 = west ? right : Math.max(p.x, start.x + MIN_CROP);
  const y0 = north ? Math.min(p.y, bottom - MIN_CROP) : start.y;
  const y1 = north ? bottom : Math.max(p.y, start.y + MIN_CROP);
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

$('develop-reset').addEventListener('click', () => {
  const ids = developTargets();
  if (!ids.length) return;
  // Closing the tool first, without restoring: a reset is about to take the
  // crop off anyway, and putting it back on the way out would race the reset.
  if (cropMode.active) closeCrop();
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

// ------------------------------------------------- putting an album inside one
//
// Nesting has always worked — an album's path *is* its place in the tree, so
// typing `colleccija/test` into either dialog puts it inside `colleccija` and
// carries its sub-albums along. Nothing said so. The only hint was a line of
// small print under a text field, and a collection created for the purpose sat
// there with no visible way to put anything in it.
//
// So this is a picker over the same field rather than a second mechanism: it
// rewrites the path's prefix, and the path stays on screen showing exactly what
// will be created or moved. Typing a path by hand still works, still creates
// any missing collections above it, and now moves the picker to match.

/// Fill an "Inside" picker with the collections an album could sit in.
/// `exclude` is an album being moved: an album cannot be put inside itself, and
/// it cannot be put inside its own descendant either — that would ask the tree
/// to hold a loop.
function fillParentPicker(select, currentParent, exclude) {
  select.innerHTML = '';
  const top = document.createElement('option');
  top.value = '';
  top.textContent = '— top level —';
  select.appendChild(top);

  state.albums
    .filter((a) => a.is_collection)
    .filter((a) => !exclude || (a.path !== exclude && !a.path.startsWith(`${exclude}/`)))
    .forEach((a) => {
      const opt = document.createElement('option');
      opt.value = a.path;
      opt.textContent = a.path;
      select.appendChild(opt);
    });

  // A parent that is not on the list — the album sits under a plain album, or
  // under nothing that exists yet — is still where the album is, so offer it
  // rather than silently reading as "top level" and moving the album on save.
  if (currentParent && !select.querySelector(`option[value="${CSS.escape(currentParent)}"]`)) {
    const opt = document.createElement('option');
    opt.value = currentParent;
    opt.textContent = currentParent;
    select.appendChild(opt);
  }
  select.value = currentParent || '';
}

/// Everything before the last path segment, or '' at the top level.
function parentOf(path) {
  const parts = path.split('/').filter(Boolean);
  return parts.slice(0, -1).join('/');
}

/// Keep the two controls telling the same story: the picker rewrites the path's
/// prefix, and typing a path moves the picker to whatever prefix was typed.
function linkParentPicker(selectId, inputId) {
  const select = $(selectId);
  const input = $(inputId);
  select.addEventListener('change', () => {
    const name = input.value.trim().split('/').filter(Boolean).pop() || '';
    input.value = select.value ? `${select.value}/${name}` : name;
  });
  input.addEventListener('input', () => {
    const parent = parentOf(input.value.trim());
    // Only when the picker already offers it — a half-typed path should not add
    // an option for a collection nobody has created.
    if (select.querySelector(`option[value="${CSS.escape(parent)}"]`)) select.value = parent;
  });
}

linkParentPicker('album-parent', 'album-path');
linkParentPicker('set-parent', 'set-path');

$('new-album-btn').addEventListener('click', () => {
  // Creating an album while looking at a collection almost always means
  // creating it in there, so start the picker on it.
  const here = state.albums.find((a) => a.path === state.currentAlbum);
  const parent = here?.is_collection ? here.path : parentOf(state.currentAlbum || '');
  $('album-path').value = parent ? `${parent}/` : '';
  $('album-title').value = '';
  $('album-collection').checked = false;
  fillParentPicker($('album-parent'), parent, null);
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
  fillParentPicker($('set-parent'), parentOf(entry.path), entry.path);
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
        if (r.collisions.length) notes.push(`${r.collisions.length} name clash(es)`);
        let line = `${r.album_path}: ${r.photos_copied} copied, ${r.photos_skipped} unchanged` +
          (notes.length ? ` · ${notes.join(' · ')}` : '');
        // A clash needs both filenames spelled out — renaming one would change
        // a URL the client may already hold, so this is the photographer's call.
        for (const c of r.collisions) {
          line += `\n  ${c.dest} — published ${c.sources[0]}`;
          for (const skipped of c.sources.slice(1)) line += `\n    not published: ${skipped}`;
        }
        return line;
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
    // The library original is the negative. A pull updates the gallery copy
    // but never writes over it, so say which frames the server disagrees on.
    (o.kept_originals?.length
      ? `\n\nThe server has a different version of these — your originals were kept:\n  ` +
        `${o.kept_originals.join('\n  ')}`
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
