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

// Only macOS draws its window buttons *inside* the page — `titleBarStyle:
// Transparent` makes the frame see-through and leaves the traffic lights on
// top of our own bar. Windows and Linux ignore that setting and put a system
// frame above the page instead, so the gutter this class turns on would be a
// dead notch on the left of every window there.
//
// Set synchronously, before anything is painted: a round trip to the shell to
// ask would mean a visible jump on the first frame.
if (navigator.userAgent.includes('Mac')) {
  document.documentElement.classList.add('mac');
}

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
  /// Servers this library knows (tokens never travel here — only has_token).
  remotes: [],
  /// The server the sync modal is browsing.
  syncRemoteId: null,
  /// remoteId -> Map(albumPath -> subscription), for the per-server chips.
  subsByRemote: new Map(),
  /// Every root this library catalogues photographs under, the primary first.
  /// Held outside the Sources panel because `online` is what tells a grid cell
  /// "that drive is in the other room" rather than "this file is corrupt".
  sources: [],
  /// Canonical root of the open library, as the core reports it — what the
  /// switcher menu compares against to mark the current entry.
  libraryRoot: '',
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
/// The keys that mean "scale the interface" when ⌘ or Ctrl is down.
const ZOOM_KEYS = new Set(['+', '=', '-', '_', '0']);

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

  applyZoom();
  updateInspectorVisibility();
  renderInfoOverlay();
}

// ------------------------------------------------------------------- zoom
//
// The webview's own page zoom, not a CSS transform: it re-lays out and
// re-renders, so type gets bigger and stays sharp. On a 4K display that is the
// difference between a readable interface and a magnified blurry one.

/// Hand the level to the shell, and tell the stylesheet what it is.
function applyZoom() {
  invoke('set_zoom', { factor: prefs.zoom }).catch(() => {});
  // The macOS traffic lights are drawn by the system and do not zoom, so the
  // gutter reserved for them must shrink as the page grows or the window title
  // drifts right of them. Everything else in the UI should scale, and does.
  document.documentElement.style.setProperty('--zoom', String(prefs.zoom));
}

function setZoom(factor) {
  prefs.zoom = factor;
  persistPrefs();
  applyZoom();
  // Every box on screen just changed size, and both overlays are measured
  // against the picture inside one of them.
  repositionOverlays();
  status(`Interface at ${Math.round(factor * 100)}%`);
}

/// The inspector is up when there is something to inspect *and* the
/// photographer has not put it away. Two conditions, one place — otherwise
/// selecting a photo silently reopens a sidebar that was deliberately closed.
function updateInspectorVisibility() {
  $('inspector').hidden = !prefs.sidebars.right || state.cursor < 0;
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
  $('zoom-readout').textContent = `${Math.round(prefs.zoom * 100)}%`;
  renderOverlayFieldChecks();
  openModal('settings-modal');
}

// The dialog is open while these are pressed, so the readout is updated here
// rather than in setZoom — which also runs from the keyboard, with nothing to
// update.
function zoomFromDialog(factor) {
  setZoom(factor);
  $('zoom-readout').textContent = `${Math.round(prefs.zoom * 100)}%`;
}

$('zoom-in').addEventListener('click', () => zoomFromDialog(zoomStep(prefs.zoom, 1)));
$('zoom-out').addEventListener('click', () => zoomFromDialog(zoomStep(prefs.zoom, -1)));
$('zoom-reset').addEventListener('click', () => zoomFromDialog(1));

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

/// macOS takes its traffic lights away in fullscreen, so the gutter the bar
/// reserves for them has to go too — otherwise it is an empty pocket to the
/// left of the app's own name.
///
/// Driven by the shell rather than by `toggleFullscreen` above, because that
/// key is only one of four ways in: the green button, ⌃⌘F and the Window menu
/// are the others, and none of them passes through this file.
function setFullscreenClass(on) {
  document.documentElement.classList.toggle('fullscreen', Boolean(on));
}

listen('fullscreen-changed', (e) => setFullscreenClass(e.payload));
// A window restored into fullscreen is already there before the first resize.
invoke('is_fullscreen').then(setFullscreenClass).catch(() => {});

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
  // Topmost first: the chip popover floats over the modals; then a dialog;
  // only once nothing is open does Escape mean "stop framing".
  if (!$('chip-menu').hidden) {
    $('chip-menu').hidden = true;
  } else if (modalIsOpen()) {
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

/// The folder-picker flow — the welcome button and the switcher menu's "Open
/// other library…" both land here, so there is one boot path, not two.
async function pickAndOpenLibrary() {
  const dir = await openDialog({ directory: true, title: 'Choose a photo folder' });
  if (!dir) return;
  try {
    await openLibrary(dir);
  } catch (err) {
    // Report where the user is: the welcome card's error line before a library
    // is open, the status bar once one is — a failed switch leaves the current
    // library on screen, and the welcome card is not.
    if ($('app').hidden) showError('welcome-error', String(err));
    else status(`Could not open library: ${err}`);
  }
}

$('open-library-btn').addEventListener('click', pickAndOpenLibrary);

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
  // Servers, sources and the Lightroom wizard all describe the library being
  // left; cleared here so none of it labels the next one's photos.
  resetLibraryState();
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

/// Clear per-library UI state. The rating/flag/text filters stay: their chips
/// are on screen and still mean what they say against any library.
function resetLibraryState() {
  if (cropMode.active) closeCrop();
  state.currentAlbum = '';
  state.selected.clear();
  state.cursor = -1;
  state.editingAlbum = null;
  state.remoteAlbums = [];
  // Servers and subscriptions belong to the library that is being left.
  state.remotes = [];
  state.syncRemoteId = null;
  state.subsByRemote = new Map();
  // Sources belong to the library too — a stale list would label the new
  // library's photos with the old one's drives.
  state.sources = [];
  $('sources-list').innerHTML = '';
  showError('sources-error', '');
  // A scan describes where photos would land in the library that was open when
  // it ran. Left standing, the wizard would offer to import it into the new one.
  resetLrWizard();
  document.querySelectorAll('.nav-item').forEach((n) =>
    n.classList.toggle('active', (n.dataset.album || '') === ''));
  $('inspector').hidden = true;
}

/// Swap the welcome screen for the app and load its contents.
async function enterApp(info) {
  $('welcome').hidden = true;
  $('app').hidden = false;
  // Before the first paint, so the window comes up arranged the way it was left
  // rather than snapping into it a moment later.
  applyPrefs();
  renderStatus(info);
  // Sources first: the grid asks which of them are attached while it paints.
  await refreshSources();
  await Promise.all([refreshAlbums(), refreshPhotos()]);
  await loadPublishTarget();
}

function renderStatus(info) {
  // The sidebar names the library; the titlebar names the application.
  state.libraryRoot = info.root;
  $('library-name').textContent = libraryName(info.root);
  $('library-name').title = info.root;
  $('all-count').textContent = info.photo_count;
  // Camera names come out of EXIF, which is to say out of the files — escape
  // them. A photo whose Model field is "<b>bold</b>" must not restyle the app.
  $('library-stats').innerHTML =
    `${info.photo_count} photos · ${info.album_count} albums` +
    (info.cameras.length ? `<br>${escapeHtml(info.cameras.join(', '))}` : '');
}

// ----------------------------------------------------------------- sources
//
// A library is one folder by default, and its own root is source #1 — the
// primary, where the catalog and every thumbnail live. Any number of other
// folders may be registered: their photographs are catalogued *where they
// are*, never copied and never moved, which is what makes a decade of work on
// an external drive importable without migrating a terabyte first.
//
// A source that is not attached right now is offline, not missing. The core
// says so per source (`online`, probed on every listing) and refuses the
// operations that need the original by name, so this panel is also where the
// answer to a blank frame in the grid — plug that drive in — is on screen.

async function refreshSources() {
  try {
    state.sources = await invoke('list_sources');
    showError('sources-error', '');
  } catch (err) {
    state.sources = [];
    // Only worth reporting where it was asked for; at boot a failure here just
    // leaves the grid without source names, which it renders fine without.
    if (!$('sources-modal').hidden) showError('sources-error', String(err));
  }
  renderSources();
}

/// One source by id, or null — a photo's row may name a source this listing
/// has not caught up with.
function sourceById(id) {
  return state.sources.find((s) => s.id === id) || null;
}

/// The registered sources that are not reachable right now, by name. This is
/// what a count of skipped photos is actually waiting for, and naming it is
/// the whole of the remedy.
function offlineSourceNames() {
  return state.sources.filter((s) => !s.online).map((s) => s.name);
}

/// "3 waiting on Archive 2019" — the phrase batch reports use for work that
/// was stepped over because a drive is elsewhere. Nothing is wrong with those
/// photographs, so this never reads as a failure.
function offlineNote(count) {
  const names = offlineSourceNames();
  return `${count} waiting on ${names.length ? names.join(', ') : 'a source that is not attached'}`;
}

const SOURCE_KIND_LABELS = {
  primary: 'library root',
  internal: 'internal disk',
  external: 'external drive',
  network: 'network share',
};

function openSources() {
  showError('sources-error', '');
  openModal('sources-modal');
  refreshSources();
}

function renderSources() {
  const list = $('sources-list');
  list.innerHTML = '';
  if (!state.sources.length) {
    list.innerHTML = '<p class="muted">No sources — open a library first.</p>';
    return;
  }

  state.sources.forEach((s) => {
    const row = document.createElement('div');
    row.className = 'remote-row';

    const info = document.createElement('div');
    info.className = 'remote-info';
    const title = document.createElement('strong');
    title.textContent = s.name;
    info.appendChild(title);

    if (s.is_primary) {
      const badge = document.createElement('span');
      badge.className = 'badge-new';
      badge.textContent = 'primary';
      info.appendChild(badge);
    }

    // Probed at the moment of the listing, so it is worth saying plainly
    // rather than leaving the photographer to infer it from an empty grid.
    const dot = document.createElement('span');
    dot.className = 'source-state' + (s.online ? ' online' : '');
    dot.textContent = s.online ? '● attached' : '○ not attached';
    info.appendChild(dot);

    const sub = document.createElement('div');
    sub.className = 'remote-sub muted';
    sub.title = s.path;
    // Kind and count lead: the row ellipsizes a long path, and the count is
    // the number the Remove dialog is about — losing it off the end of the
    // line is how "forget this folder" reads as harmless when it is not.
    sub.textContent =
      `${SOURCE_KIND_LABELS[s.kind] || s.kind} · ${s.photo_count} photo(s) · ${s.path}`;
    info.appendChild(sub);
    row.appendChild(info);

    const actions = document.createElement('div');
    actions.className = 'remote-actions';

    // The primary is the library itself: it moves by opening the library
    // somewhere else, and it cannot be forgotten at all.
    if (!s.is_primary) {
      const relocate = document.createElement('button');
      relocate.className = 'btn btn-sm';
      relocate.textContent = 'Relocate…';
      relocate.title = 'Point this source at the folder it lives in now';
      relocate.addEventListener('click', () => relocateSource(s));
      actions.appendChild(relocate);

      const remove = document.createElement('button');
      remove.className = 'btn btn-sm btn-danger';
      remove.textContent = 'Remove';
      remove.addEventListener('click', () => removeSource(s));
      actions.appendChild(remove);
    }

    row.appendChild(actions);
    list.appendChild(row);
  });
}

$('source-add-btn').addEventListener('click', async () => {
  const dir = await openDialog({
    directory: true,
    title: 'Choose a folder to catalogue in place',
  });
  if (!dir) return;
  showError('sources-error', '');
  status('Registering source…');
  try {
    await invoke('add_source', { path: dir, name: null, kind: $('source-kind').value });
    await refreshSources();
    status(`Added source ${dir} — import it to catalogue its photos in place`);
  } catch (err) {
    // Overlap with the library or another source comes back as a sentence
    // naming the source it clashes with. Show it as it is.
    showError('sources-error', String(err));
    status('Source not added');
  }
});

async function relocateSource(s) {
  const dir = await openDialog({
    directory: true,
    title: `Where is "${s.name}" now?`,
  });
  if (!dir) return;
  showError('sources-error', '');
  status(`Relocating ${s.name}…`);
  try {
    await invoke('relocate_source', { id: s.id, newPath: dir });
    await refreshSources();
    await refreshPhotos();
    status(`${s.name} now points at ${dir}`);
  } catch (err) {
    // A folder that does not hold this source's photographs is refused with
    // the file that gave it away named in the message, and nothing changed.
    // That sentence is the whole diagnosis — show it, and leave the row alone.
    showError('sources-error', String(err));
    status(`${s.name} unchanged`);
  }
}

async function removeSource(s) {
  // The count is the difference between forgetting an empty folder and
  // discarding a shoot's worth of ratings, so it decides the question asked —
  // and the answer to it is what travels as `drop_photos`. Either way no file
  // on disk is touched, which is the sentence a photographer needs to read
  // before pressing this.
  const holdsPhotos = s.photo_count > 0;
  const sure = await askConfirm(
    holdsPhotos
      ? `"${s.name}" holds ${s.photo_count} catalogued photo(s). Removing it drops ` +
        'those catalog rows — their ratings, flags, album memberships and ' +
        'adjustments go with them.\n\n' +
        'The photographs themselves are never touched: every file stays exactly ' +
        'where it is on disk, and adding the folder back re-imports them.'
      : `"${s.name}" holds no catalogued photos.\n\n` +
        'Removing it only makes this library stop looking there. No file on disk ' +
        'is touched.',
    {
      title: `Remove source "${s.name}"?`,
      kind: 'warning',
      okLabel: holdsPhotos ? `Remove and drop ${s.photo_count} row(s)` : 'Remove',
    }
  );
  if (!sure) return;

  showError('sources-error', '');
  try {
    const dropped = await invoke('remove_source', { id: s.id, dropPhotos: holdsPhotos });
    await refreshSources();
    await refreshAll();
    status(
      dropped
        ? `Removed ${s.name} · ${dropped} catalog row(s) dropped, no file touched`
        : `Removed ${s.name} — no file touched`
    );
  } catch (err) {
    // Reached when the count this panel held was stale and the core found
    // photos on it after all. Its message says so; nothing was removed.
    showError('sources-error', String(err));
    status(`${s.name} unchanged`);
  }
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
      (summary.failed.length ? ` · ${summary.failed.length} failed` : '') +
      // A registered source that was not attached is skipped rather than
      // failed, and the note names it. Dropping these turns "your external
      // drive was not there" into a run that read as complete.
      (summary.notes.length ? ` · ${summary.notes.join(' · ')}` : '')
    );
    // A source may have come back — or gone — while the import ran.
    await refreshSources();
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
    //
    // A photo on a source that is not attached is neither of those things:
    // nothing is wrong with it, the drive is simply in the other room. Calling
    // that "No preview" sends someone looking for a corrupt file that is
    // perfectly safe, so it names the drive instead.
    const undecodable = () => {
      const offline = state.sources.find((s) => s.id === photo.source_id && !s.online);
      cell.classList.add(offline ? 'offline' : 'undecodable');
      img.remove();
      const note = document.createElement('div');
      note.className = 'cell-note';
      note.textContent = offline ? `${offline.name} not attached` : 'No preview';
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

  // Before the digits: ⌘0 means "back to 100%", a bare 0 means "clear the
  // rating", and the digit branch below does not look at modifiers.
  if ((e.metaKey || e.ctrlKey) && ZOOM_KEYS.has(e.key)) {
    e.preventDefault();
    // `+` needs Shift on most layouts, so `=` is the key actually reported;
    // accept both, and the numeric keypad's own signs too.
    setZoom(e.key === '0' ? 1 : zoomStep(prefs.zoom, e.key === '-' || e.key === '_' ? -1 : 1));
  } else if (e.key >= '0' && e.key <= '5') {
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
    // rel_path is relative to this photo's own source, so it does not name a
    // place on its own once a library catalogues more than one root.
    ['Source', describeSourceOf(photo)],
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

/// The source a photo's path is relative to, and whether it is attached —
/// what turns "develop failed" into "the Card drive is not plugged in".
function describeSourceOf(photo) {
  const s = sourceById(photo.source_id);
  if (!s) return '—';
  return s.name + (s.online ? '' : ' — not attached');
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
  $('export-xmp-result').textContent = '';
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
    // Photos stepped over because their drive is elsewhere are named against
    // the source they are waiting on, and `online` is only true as of the last
    // listing — re-probe before saying which drive it is.
    if (results.some((r) => r.offline.length)) await refreshSources();
    const total = results.reduce((n, r) => n + r.photos_copied, 0);
    $('publish-output').hidden = false;
    $('publish-output').textContent = results
      .map((r) => {
        // Anything not shipped is named here. Silence would read as success.
        const notes = [];
        if (r.missing.length) notes.push(`${r.missing.length} missing from disk`);
        if (r.unrenderable.length) notes.push(`${r.unrenderable.length} without a preview, not published`);
        // Not a failure and not a loss: the album is simply short until the
        // drive is attached, and the next publish ships them. Left unsaid, an
        // album that quietly went up without a third of its frames looks
        // exactly like one that went up whole.
        if (r.offline.length) notes.push(offlineNote(r.offline.length));
        if (r.removed.length) notes.push(`${r.removed.length} removed`);
        if (r.collisions.length) notes.push(`${r.collisions.length} name clash(es)`);
        let line = `${r.album_path}: ${r.photos_copied} copied, ${r.photos_skipped} unchanged` +
          (notes.length ? ` · ${notes.join(' · ')}` : '');
        // Each entry already names the photo and the source it is waiting on.
        for (const o of r.offline) line += `\n  not published yet: ${o}`;
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
// One album at a time, in the direction chosen for that album, per server.
// Albums with no direction set are never touched — not pushed, not pulled,
// not deleted. A library can know several servers; each keeps its own
// subscriptions and baselines, so a conflict on one is not a conflict on
// another.

$('sync-btn').addEventListener('click', async () => {
  showError('sync-error', '');
  $('sync-output').hidden = true;
  openModal('sync-modal');
  await refreshServers();
  if (state.syncRemoteId != null) {
    await refreshRemote();
  } else {
    $('remote-list').innerHTML = '<p class="muted">Add a server to begin.</p>';
  }
});

/// The remote the modal is browsing, or null while none is configured.
function currentRemote() {
  return state.remotes.find((r) => r.id === state.syncRemoteId) || null;
}

const DIRECTION_GLYPHS = { push: '↑', pull: '↓', both: '⇅' };

/// This machine's subscription for one album on one remote, or null.
function subFor(remoteId, albumPath) {
  return state.subsByRemote.get(remoteId)?.get(albumPath) || null;
}

// ---------------------------------------------------------------- servers

async function refreshServers() {
  try {
    state.remotes = await invoke('list_remotes');
  } catch (err) {
    state.remotes = [];
    showError('sync-error', String(err));
  }
  // Keep the browsed server if it survived; otherwise fall to the default.
  if (!state.remotes.some((r) => r.id === state.syncRemoteId)) {
    const fallback = state.remotes.find((r) => r.is_default) || state.remotes[0];
    state.syncRemoteId = fallback ? fallback.id : null;
  }
  await refreshSubscriptions();
  renderServers();
}

/// Subscriptions per remote, so an album row can show one chip per server
/// without a round trip per row.
async function refreshSubscriptions() {
  state.subsByRemote = new Map();
  for (const r of state.remotes) {
    try {
      const subs = await invoke('album_subscriptions_on', { remoteId: r.id });
      state.subsByRemote.set(r.id, new Map(subs.map((s) => [s.album_path, s])));
    } catch {
      state.subsByRemote.set(r.id, new Map());
    }
  }
}

function renderServers() {
  const list = $('servers-list');
  list.innerHTML = '';
  if (!state.remotes.length) {
    list.innerHTML =
      '<p class="muted">No servers yet — add a folder (network share, external ' +
      'drive) or the gallery’s sync URL.</p>';
  }

  state.remotes.forEach((r) => {
    const row = document.createElement('div');
    row.className = 'remote-row';

    const info = document.createElement('div');
    info.className = 'remote-info';
    const title = document.createElement('strong');
    title.textContent = r.name;
    info.appendChild(title);
    if (r.is_default) {
      const badge = document.createElement('span');
      badge.className = 'badge-new';
      badge.textContent = 'default';
      info.appendChild(badge);
    }
    const sub = document.createElement('div');
    sub.className = 'remote-sub muted';
    sub.textContent = r.target + (r.has_token ? ' · token stored' : '');
    info.appendChild(sub);
    row.appendChild(info);

    const actions = document.createElement('div');
    actions.className = 'remote-actions';

    if (!r.is_default) {
      const mkDefault = document.createElement('button');
      mkDefault.className = 'btn btn-sm';
      mkDefault.textContent = 'Make default';
      mkDefault.addEventListener('click', async () => {
        try {
          await invoke('set_default_remote', { id: r.id });
          await refreshServers();
        } catch (err) {
          showError('sync-error', String(err));
        }
      });
      actions.appendChild(mkDefault);
    }

    const edit = document.createElement('button');
    edit.className = 'btn btn-sm';
    edit.textContent = 'Edit';
    edit.addEventListener('click', () => openServerModal(r));
    actions.appendChild(edit);

    const remove = document.createElement('button');
    remove.className = 'btn btn-sm btn-danger';
    remove.textContent = 'Remove';
    remove.addEventListener('click', async () => {
      const sure = await askConfirm(
        'This library forgets the server: its subscriptions and sync baselines ' +
        `for "${r.name}" are dropped — locally. Nothing on the server itself ` +
        'is touched.',
        { title: `Remove server "${r.name}"?`, kind: 'warning', okLabel: 'Remove' }
      );
      if (!sure) return;
      try {
        await invoke('remove_remote', { id: r.id });
        await refreshServers();
        if (state.syncRemoteId != null) await refreshRemote();
        else $('remote-list').innerHTML = '<p class="muted">Add a server to begin.</p>';
      } catch (err) {
        showError('sync-error', String(err));
      }
    });
    actions.appendChild(remove);

    row.appendChild(actions);
    list.appendChild(row);
  });

  // The browse selector mirrors the same list.
  const select = $('sync-remote-select');
  select.innerHTML = '';
  state.remotes.forEach((r) => {
    const opt = document.createElement('option');
    opt.value = r.id;
    opt.textContent = r.name;
    select.appendChild(opt);
  });
  if (state.syncRemoteId != null) select.value = String(state.syncRemoteId);
  select.disabled = !state.remotes.length;
  $('sync-all-scope').disabled = !state.remotes.length;
}

$('sync-remote-select').addEventListener('change', async () => {
  state.syncRemoteId = Number($('sync-remote-select').value);
  await refreshRemote();
});

// One modal for add and edit; which it is lives here.
let editingRemoteId = null;

$('server-add-btn').addEventListener('click', () => openServerModal(null));

function openServerModal(remote) {
  editingRemoteId = remote ? remote.id : null;
  $('server-modal-title').textContent = remote ? `Edit "${remote.name}"` : 'Add server';
  $('server-name').value = remote ? remote.name : '';
  $('server-target').value = remote ? remote.target : '';
  $('server-token').value = '';
  // The token itself never comes back out of the catalog — only whether one
  // is stored, so the field can say so without echoing a secret.
  $('server-token').placeholder = remote?.has_token
    ? 'stored — type to replace'
    : 'paste once — it is kept in the catalog';
  syncServerTokenRow();
  showError('server-error', '');
  openModal('server-modal');
}

/// A URL remote authenticates with a token; a folder does not.
function syncServerTokenRow() {
  $('server-token-row').hidden = !/^https?:\/\//i.test($('server-target').value.trim());
}
$('server-target').addEventListener('input', syncServerTokenRow);

$('server-pick-btn').addEventListener('click', async () => {
  const dir = await openDialog({ directory: true, title: 'Choose the shared album folder' });
  if (!dir) return;
  $('server-target').value = dir;
  syncServerTokenRow();
});

$('server-save-btn').addEventListener('click', async () => {
  const name = $('server-name').value.trim();
  const target = $('server-target').value.trim();
  const token = $('server-token').value.trim();
  try {
    if (editingRemoteId != null) {
      // An empty token box means "leave the stored secret alone".
      const update = { name, target, ...(token ? { token } : {}) };
      await invoke('update_remote', { id: editingRemoteId, update });
    } else {
      await invoke('add_remote', { name, target, token: token || null });
    }
    closeModal('server-modal');
    await refreshServers();
    await refreshRemote();
  } catch (err) {
    showError('server-error', String(err));
  }
});

// ------------------------------------------------------------- album list

$('remote-refresh-btn').addEventListener('click', () => refreshRemote());

async function refreshRemote() {
  const list = $('remote-list');
  if (state.syncRemoteId == null) {
    list.innerHTML = '<p class="muted">Add a server to begin.</p>';
    return;
  }
  list.innerHTML = '<p class="muted">Reading…</p>';
  try {
    state.remoteAlbums = await invoke('remote_albums_on', { remoteId: state.syncRemoteId });
  } catch (err) {
    list.innerHTML = '';
    showError('sync-error', String(err));
    return;
  }
  showError('sync-error', '');
  await refreshSubscriptions();
  renderRemoteList();
}

function renderRemoteList() {
  const list = $('remote-list');
  list.innerHTML = '';

  if (!state.remoteAlbums.length) {
    list.innerHTML = '<p class="muted">No albums here or on the server yet.</p>';
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

    // One chip per server the album is tracked on: name, direction glyph,
    // scope. Untracked everywhere leaves only the ＋ affordance.
    const chips = document.createElement('div');
    chips.className = 'sync-chips';
    let untracked = [];
    state.remotes.forEach((r) => {
      const s = subFor(r.id, album.path);
      if (!s) {
        untracked.push(r);
        return;
      }
      const chip = document.createElement('button');
      chip.className = 'sync-chip';
      chip.textContent = `${r.name} ${DIRECTION_GLYPHS[s.direction] || '?'} ${s.scope}`;
      chip.title = `${r.name}: ${s.direction} · ${s.scope} — click to change`;
      chip.addEventListener('click', (e) => {
        e.stopPropagation();
        openChipMenu(chip, album, r);
      });
      chips.appendChild(chip);
    });
    if (untracked.length && state.remotes.length) {
      const add = document.createElement('button');
      add.className = 'sync-chip add';
      add.textContent = '＋';
      add.title = 'Track this album on a server';
      add.addEventListener('click', (e) => {
        e.stopPropagation();
        openChipAddMenu(add, album, untracked);
      });
      chips.appendChild(add);
    }
    info.appendChild(chips);
    row.appendChild(info);

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
      invoke('pull_album_on', { path: album.path, remoteId: state.syncRemoteId })));
    actions.appendChild(pull);

    const push = document.createElement('button');
    push.className = 'btn btn-sm';
    push.textContent = album.remote ? 'Push' : 'Add to server';
    push.disabled = !album.local;
    push.title = 'Upload this path — its folders, itself, and everything under it';
    push.addEventListener('click', () => runSync(row, album.path, () =>
      pushWithDeleteCheck('push_album_on', album.path, { remoteId: state.syncRemoteId })));
    actions.appendChild(push);

    // Any destination at all — this library's other servers, or a remote a
    // different known library holds the credentials for.
    if (album.local) {
      const pushTo = document.createElement('button');
      pushTo.className = 'btn btn-sm';
      pushTo.textContent = 'Push to…';
      pushTo.title = 'Push this path to any server — including one of another library’s';
      pushTo.addEventListener('click', () => openPushTo(album.path));
      actions.appendChild(pushTo);
    }

    // Both-ways in one click, only for albums tracked both ways here.
    if (subFor(state.syncRemoteId, album.path)?.direction === 'both') {
      const sync = document.createElement('button');
      sync.className = 'btn btn-sm btn-primary';
      sync.textContent = 'Sync';
      sync.title = 'Pull the server’s changes for this whole path, then push this machine’s';
      sync.addEventListener('click', () => runSync(row, album.path, () =>
        pushWithDeleteCheck('sync_album_on', album.path, {
          direction: 'both',
          remoteId: state.syncRemoteId,
        })));
      actions.appendChild(sync);
    }

    row.appendChild(actions);
    list.appendChild(row);
  });
}

// ---------------------------------------------------------------- chip menu
//
// One small popover for "how does this album sync on that server": direction
// (untracked / push / pull / both) and scope (web / full). Filled per click,
// positioned by the chip that asked for it.

function closeChipMenu() {
  $('chip-menu').hidden = true;
}

document.addEventListener('click', (e) => {
  if (!e.target.closest('.chip-menu')) closeChipMenu();
});

function placeChipMenu(anchor) {
  const menu = $('chip-menu');
  menu.hidden = false;
  const at = anchor.getBoundingClientRect();
  const width = menu.offsetWidth || 240;
  const height = menu.offsetHeight || 120;
  menu.style.left = `${Math.min(at.left, window.innerWidth - width - 8)}px`;
  menu.style.top = at.bottom + height + 8 > window.innerHeight
    ? `${at.top - height - 4}px`
    : `${at.bottom + 4}px`;
}

async function applyTracking(album, remote, direction, scope) {
  try {
    if (!direction) {
      await invoke('untrack_album_on', { path: album.path, remoteId: remote.id });
      status(`${album.path} is no longer synced with ${remote.name}`);
    } else {
      await invoke('track_album_on', {
        path: album.path,
        direction,
        scope,
        remoteId: remote.id,
      });
      status(`${album.path} syncs ${direction} (${scope || 'web'}) with ${remote.name}`);
    }
    await refreshSubscriptions();
    renderRemoteList();
  } catch (err) {
    showError('sync-error', String(err));
  }
}

function openChipMenu(anchor, album, remote) {
  const menu = $('chip-menu');
  menu.innerHTML = '';
  const s = subFor(remote.id, album.path);

  const head = document.createElement('div');
  head.className = 'chip-menu-head muted';
  head.textContent = `${album.path} on ${remote.name}`;
  menu.appendChild(head);

  const dirRow = document.createElement('div');
  dirRow.className = 'chip-menu-row';
  [
    ['', 'Untracked'],
    ['push', '↑ Push'],
    ['pull', '↓ Pull'],
    ['both', '⇅ Both'],
  ].forEach(([value, label]) => {
    const b = document.createElement('button');
    b.className = 'chip' + ((s?.direction || '') === value ? ' active' : '');
    b.textContent = label;
    b.addEventListener('click', async (e) => {
      e.stopPropagation();
      closeChipMenu();
      // Changing direction keeps the scope the subscription already carries.
      await applyTracking(album, remote, value, value ? (s?.scope || 'web') : null);
    });
    dirRow.appendChild(b);
  });
  menu.appendChild(dirRow);

  const scopeRow = document.createElement('div');
  scopeRow.className = 'chip-menu-row';
  [
    ['web', 'Web — published tree'],
    ['full', 'Full — plus originals'],
  ].forEach(([value, label]) => {
    const b = document.createElement('button');
    b.className = 'chip' + (s?.scope === value ? ' active' : '');
    b.textContent = label;
    b.disabled = !s;
    b.title = s ? '' : 'Pick a direction first';
    b.addEventListener('click', async (e) => {
      e.stopPropagation();
      closeChipMenu();
      await applyTracking(album, remote, s.direction, value);
    });
    scopeRow.appendChild(b);
  });
  menu.appendChild(scopeRow);

  placeChipMenu(anchor);
}

/// The ＋ chip: pick which of the still-untracked servers to track on, then
/// fall into the same direction/scope menu.
function openChipAddMenu(anchor, album, untracked) {
  const menu = $('chip-menu');
  menu.innerHTML = '';

  const head = document.createElement('div');
  head.className = 'chip-menu-head muted';
  head.textContent = `Track ${album.path} on…`;
  menu.appendChild(head);

  untracked.forEach((r) => {
    const row = document.createElement('div');
    row.className = 'chip-menu-row';
    const b = document.createElement('button');
    b.className = 'chip';
    b.textContent = r.name;
    b.addEventListener('click', (e) => {
      e.stopPropagation();
      openChipMenu(anchor, album, r);
    });
    row.appendChild(b);
    menu.appendChild(row);
  });

  placeChipMenu(anchor);
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
  showError('sync-error', '');
  // One server, or every server this library knows — the selector says which.
  const targets = $('sync-all-scope').value === 'all'
    ? state.remotes
    : state.remotes.filter((r) => r.id === state.syncRemoteId);
  if (!targets.length) {
    showError('sync-error', 'Add a server first.');
    return;
  }
  status('Syncing tracked albums…');
  $('sync-all-btn').disabled = true;
  try {
    const lines = [];
    let albums = 0;
    let failedAlbums = 0;
    for (const r of targets) {
      // One server being unreachable must not stop the others, same as one
      // album's failure never stops the batch.
      let results;
      try {
        results = await invoke('sync_all_tracked_on', {
          allowDeletes: false,
          remoteId: r.id,
        });
      } catch (err) {
        lines.push(`${r.name}: ${err}`);
        failedAlbums += 1;
        continue;
      }
      if (!results.length) {
        lines.push(`${r.name}: nothing tracked yet — pick a direction for an album first.`);
        continue;
      }
      albums += results.length;
      failedAlbums += results.filter(([, o]) => o.failed?.length).length;
      // One album failing no longer stops the others, so the reason has to be
      // shown per album — a count alone would leave the photographer knowing
      // something went wrong and not which album or why.
      lines.push(
        ...results.map(([path, o]) =>
          [
            `${r.name} · ${path}: ${describeOutcome(o)}`,
            ...(o.failed || []).map(([file, why]) => `    ${file} — ${why}`),
          ].join('\n')
        )
      );
    }
    $('sync-output').hidden = false;
    $('sync-output').textContent = lines.join('\n');
    status(
      failedAlbums
        ? `Synced ${albums - failedAlbums} of ${albums} album(s)`
        : `Synced ${albums} album(s)`
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
    // The read twin of a push's failures, and the same rule: one item failing
    // never stops a transfer, so a run that drops this list has turned a
    // partial pull into something that looks exactly like a complete one.
    (o.failed?.length
      ? `\n\nCould not be done — the rest of the transfer still happened:\n  ` +
        `${o.failed.map(([file, why]) => `${file} — ${why}`).join('\n  ')}`
      : '') +
    // A parent folder someone else configured is not this push's to rewrite.
    (o.folders_left_alone?.length
      ? `\n\nAlready on the server and configured elsewhere — left as they are:\n  ` +
        `${o.folders_left_alone.join('\n  ')}\n` +
        `Push a folder on its own row to change it.`
      : '');
  status(summary);
}

// ----------------------------------------------------------------- push to…
//
// Push one album anywhere: this library's own servers first, then remotes
// read out of the other libraries this machine knows — their catalogs hold
// the credentials, so nothing has to be retyped. A foreign push is stateless:
// no subscription, no baseline, never a delete.

let pushToAlbum = null;

async function openPushTo(albumPath) {
  pushToAlbum = albumPath;
  $('push-to-album').textContent = albumPath;
  $('push-to-scope').value = 'web';
  showError('push-to-error', '');
  $('push-to-output').hidden = true;
  openModal('push-to-modal');

  const list = $('push-to-list');
  list.innerHTML = '<p class="muted">Reading destinations…</p>';
  const rows = [];

  const destRow = (label, target, note, onPush) => {
    const row = document.createElement('div');
    row.className = 'remote-row';
    const info = document.createElement('div');
    info.className = 'remote-info';
    const title = document.createElement('strong');
    title.textContent = label;
    info.appendChild(title);
    const sub = document.createElement('div');
    sub.className = 'remote-sub muted';
    sub.textContent = target + (note ? ` · ${note}` : '');
    info.appendChild(sub);
    row.appendChild(info);
    const actions = document.createElement('div');
    actions.className = 'remote-actions';
    const push = document.createElement('button');
    push.className = 'btn btn-sm btn-primary';
    push.textContent = 'Push';
    push.addEventListener('click', async () => {
      push.disabled = true;
      showError('push-to-error', '');
      status(`Pushing ${albumPath}…`);
      try {
        await onPush();
        await refreshAll();
      } catch (err) {
        showError('push-to-error', String(err));
        status('Push failed');
      } finally {
        push.disabled = false;
      }
    });
    actions.appendChild(push);
    row.appendChild(actions);
    return row;
  };

  const noteRow = (text) => {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = text;
    return p;
  };

  // This library's own servers, default first.
  [...state.remotes]
    .sort((a, b) => Number(b.is_default) - Number(a.is_default))
    .forEach((r) => {
      rows.push(destRow(
        r.name + (r.is_default ? ' (default)' : ''),
        r.target,
        'this library',
        async () => {
          const outcome = await pushWithDeleteCheck('push_album_on', albumPath, {
            remoteId: r.id,
          });
          reportForeignish(`${r.name}: ${describeOutcome(outcome)}`);
          await refreshSubscriptions();
        }
      ));
    });

  // Other known libraries' remotes. An unreadable library — unplugged drive,
  // newer schema — is skipped with a note; the rest still show.
  const libraries = loadRecent(localStorage)
    .filter((path) => path !== state.libraryRoot)
    .map((path) => ({ path, name: libraryName(path) }));
  for (const lib of libraries) {
    let foreign;
    try {
      foreign = await invoke('read_library_remotes', { libraryRoot: lib.path });
    } catch (err) {
      rows.push(noteRow(`Library "${lib.name}" skipped — ${err}`));
      continue;
    }
    foreign.forEach((fr) => {
      rows.push(destRow(
        `${fr.name} — library ${lib.name}`,
        fr.target,
        'stateless — never deletes',
        async () => {
          const outcome = await invoke('push_album_to', {
            path: albumPath,
            target: fr.target,
            token: fr.token ?? null,
            scope: $('push-to-scope').value,
          });
          reportForeignPush(outcome);
        }
      ));
    });
  }

  list.innerHTML = '';
  if (!rows.length) {
    list.appendChild(noteRow('No destination yet — add a server first.'));
  }
  rows.forEach((r) => list.appendChild(r));
}

function reportForeignish(text) {
  $('push-to-output').hidden = false;
  $('push-to-output').textContent = text;
  status(text);
}

/// A foreign push names its provenance and every overwrite — with no baseline
/// there is no way to know whose copy was newer, so the photographer gets the
/// list rather than silence.
function reportForeignPush(o) {
  const lines = [
    `${o.album_path}: ${o.files_pushed} pushed · ${o.skipped_unchanged} unchanged` +
      (o.albums?.length > 1 ? ` · ${o.albums.length} albums` : ''),
    `Pushed from library "${o.library_name}" (${o.library_id})`,
  ];
  if (o.overwritten?.length) {
    lines.push('', 'Replaced a differing copy on the server:',
      ...o.overwritten.map((f) => `  ${f}`));
  }
  if (o.failed?.length) {
    lines.push('', 'Never reached the server:',
      ...o.failed.map(([f, why]) => `  ${f} — ${why}`));
  }
  $('push-to-output').hidden = false;
  $('push-to-output').textContent = lines.join('\n');
  status(`Pushed ${o.files_pushed} file(s)` +
    (o.overwritten?.length ? ` · replaced ${o.overwritten.length}` : '') +
    (o.failed?.length ? ` · ${o.failed.length} failed` : ''));
}

// --------------------------------------------------------- lightroom import
//
// Wizard: pick a .lrcat → scan (read-only) → choose options → dry run or
// import. Progress and cancel ride the same statusbar affordances an ordinary
// import uses — the core emits the same event and honours the same flag.

const lrState = { path: null, report: null };

/// Put the wizard back to "no catalog chosen". Called on a library switch:
/// a scan is only meaningful against the library it was read for.
function resetLrWizard() {
  lrState.path = null;
  lrState.report = null;
  $('lr-path').value = '';
  // The per-root selects go with the report; this one is markup, so it has to
  // be put back by hand or it would describe a scan that is no longer here.
  $('lr-placement-all').value = '';
  $('lr-report').hidden = true;
  $('lr-output').hidden = true;
  $('lr-dry-run-btn').disabled = true;
  $('lr-run-btn').disabled = true;
  showError('lr-error', '');
}

$('lr-import-btn').addEventListener('click', () => {
  showError('lr-error', '');
  $('lr-output').hidden = true;
  openModal('lr-modal');
});

$('lr-pick-btn').addEventListener('click', async () => {
  const file = await openDialog({
    title: 'Choose a Lightroom Classic catalog',
    filters: [{ name: 'Lightroom catalog', extensions: ['lrcat'] }],
  });
  if (!file) return;
  lrState.path = file;
  $('lr-path').value = file;
  showError('lr-error', '');
  $('lr-output').hidden = true;
  $('lr-report').hidden = true;
  $('lr-dry-run-btn').disabled = true;
  $('lr-run-btn').disabled = true;
  status('Reading catalog…');
  try {
    lrState.report = await invoke('lr_scan', { lrcatPath: file });
    renderLrReport(lrState.report);
    $('lr-report').hidden = false;
    $('lr-dry-run-btn').disabled = false;
    $('lr-run-btn').disabled = false;
    status('Catalog read — nothing imported yet');
  } catch (err) {
    showError('lr-error', String(err));
    status('Could not read catalog');
  }
});

function renderLrReport(report) {
  const bits = [
    `${report.root_folders.length} folder(s)`,
    `${report.collections.length} collection(s)`,
    `${report.keyword_count} keyword(s)`,
  ];
  if (report.images_without_files) {
    bits.push(`${report.images_without_files} image(s) without files`);
  }
  $('lr-summary').textContent =
    bits.join(' · ') + (report.notes.length ? `\n${report.notes.join(' · ')}` : '');

  const roots = $('lr-roots');
  roots.innerHTML = '';
  report.root_folders.forEach((root) => {
    const row = document.createElement('div');
    row.className = 'remote-row';
    const info = document.createElement('div');
    info.className = 'remote-info';
    const title = document.createElement('strong');
    title.textContent = root.name;
    info.appendChild(title);
    const badge = document.createElement('span');
    badge.className = 'badge-new';
    // In place: the folder already lives under a registered source, so its
    // files are catalogued where they lie rather than copied.
    badge.textContent = root.in_place ? 'in place' : 'outside the library';
    info.appendChild(badge);
    const sub = document.createElement('div');
    sub.className = 'remote-sub muted';
    sub.title = root.path;
    sub.textContent = `${root.path} · ${root.file_count} file(s)` +
      (root.missing_files ? ` · ${root.missing_files} missing on disk` : '');
    info.appendChild(sub);
    row.appendChild(info);

    const actions = document.createElement('div');
    actions.className = 'remote-actions';
    actions.appendChild(placementSelect(root));
    row.appendChild(actions);

    roots.appendChild(row);
  });

  const coll = $('lr-collections');
  coll.innerHTML = '';
  if (!report.collections.length) {
    coll.innerHTML = '<p class="muted">No collections — photos and keywords still import.</p>';
  }
  report.collections.forEach((c) => {
    const row = document.createElement('label');
    row.className = 'check lr-coll';
    const depth = c.path.split('/').length - 1;
    row.style.marginLeft = `${depth * 1.1}rem`;
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = true;
    box.dataset.path = c.path;
    row.appendChild(box);
    const name = document.createElement('span');
    name.textContent = c.path.split('/').pop();
    row.appendChild(name);
    const sub = document.createElement('span');
    sub.className = 'muted';
    sub.textContent = `${c.kind} · ${c.member_count}`;
    row.appendChild(sub);
    coll.appendChild(row);
  });
}

/// Where one Lightroom root folder's files should end up. Values are
/// `LrPlacement`'s own serde spellings (kebab-case) — the core reads them
/// straight off the wire, so a re-spelling here is a rejected import, not a
/// silently ignored option.
const LR_PLACEMENTS = [
  { value: 'auto', label: 'Auto' },
  { value: 'in-place', label: 'In place' },
  { value: 'copy', label: 'Copy' },
  { value: 'reference', label: 'Reference' },
];

/// The placement dropdown for one scanned root, with the choices that root
/// cannot honour disabled — and each disabled one saying why, since "greyed
/// out" on its own is only a puzzle.
function placementSelect(root) {
  const sel = document.createElement('select');
  sel.title = 'Where this folder\'s photos end up';
  LR_PLACEMENTS.forEach(({ value, label }) => {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = label;
    if (value === 'in-place' && !root.in_place) {
      // The scan reports in_place only for a root that already lies under a
      // registered source. Anywhere else, importing in place would have to
      // register the folder as a source first — which is Reference, and is
      // worth choosing deliberately rather than arriving at by accident.
      opt.disabled = true;
      opt.textContent = `${label} — outside every source; use Reference`;
    }
    if (value === 'reference' && !root.can_reference && !root.in_place) {
      opt.disabled = true;
      opt.textContent = `${label} — the folder is not readable from here`;
    }
    sel.appendChild(opt);
  });
  sel.value = 'auto';
  return sel;
}

// One choice for every folder — the shape a mixed selection cannot currently
// take (see lrPlacementProblem), and the quick way to say "reference all of it".
$('lr-placement-all').addEventListener('change', () => {
  const pick = $('lr-placement-all').value;
  if (!pick) return;
  document.querySelectorAll('#lr-roots select').forEach((sel) => {
    const option = [...sel.options].find((o) => o.value === pick);
    if (option && !option.disabled) sel.value = pick;
  });
  showError('lr-error', '');
});

/// What each scanned root is set to, paired with the id `LrRootPlacement` is
/// keyed by — `AgLibraryRootFolder.id_local`.
function lrRootChoices() {
  const roots = lrState.report?.root_folders || [];
  return [...document.querySelectorAll('#lr-roots select')].map((sel, i) => ({
    // `LrRootReport` carries path, name, counts, in_place and can_reference —
    // and no id. Read whichever name it grows for one, so per-root choices
    // start travelling the moment the core hands them over.
    rootId: roots[i]?.root_id ?? roots[i]?.id ?? null,
    mode: sel.value,
  }));
}

/// Why the chosen placements cannot be sent as picked, or null when they can.
///
/// `roots` is a list of `{ root_id, mode }` keyed by the id the scan reports
/// per folder, so normally every selection travels as picked. The guard is for
/// a scan that named no ids — an older core — where one answer for every
/// folder still travels exactly (the core falls back to the run-wide `mode`),
/// and only a mixed selection is stuck: inventing an id there would hand some
/// folder another folder's answer without a word.
function lrPlacementProblem() {
  const choices = lrRootChoices();
  if (choices.length < 2) return null;
  if (choices.every((c) => c.rootId != null)) return null;
  if (new Set(choices.map((c) => c.mode)).size <= 1) return null;
  return (
    'Different folders are set to different placements, and this catalog scan ' +
    'did not name the folder ids the core needs to tell them apart. Set every ' +
    'folder to the same placement to run it, or import the folders in ' +
    'separate runs.'
  );
}

$('lr-coll-all-btn').addEventListener('click', () => {
  document.querySelectorAll('#lr-collections input[type=checkbox]')
    .forEach((b) => (b.checked = true));
});
$('lr-coll-none-btn').addEventListener('click', () => {
  document.querySelectorAll('#lr-collections input[type=checkbox]')
    .forEach((b) => (b.checked = false));
});

/// The options the form currently describes. Field names are the core's own
/// (`LrImportOptions` is deny_unknown_fields — a stray name would be an
/// error, not a silently dropped filter).
function lrOptions(dryRun) {
  const boxes = [...document.querySelectorAll('#lr-collections input[type=checkbox]')];
  const chosen = boxes.filter((b) => b.checked).map((b) => b.dataset.path);
  const choices = lrRootChoices();

  // Per-root overrides when the report names its roots; otherwise the one
  // answer they all share, which the core applies to every root it holds no
  // override for. lrPlacementProblem() has already refused anything else.
  const named = choices.length > 0 && choices.every((c) => c.rootId != null);
  const modes = [...new Set(choices.map((c) => c.mode))];

  return {
    dest_subdir: $('lr-dest-subdir').value.trim() || null,
    album_prefix: $('lr-album-prefix').value.trim() || null,
    // Everything ticked means no filter at all — new collections in a later
    // re-run are then included rather than silently skipped.
    collections: chosen.length === boxes.length ? null : chosen,
    collision: $('lr-collision').value,
    mode: named ? 'auto' : (modes[0] || 'auto'),
    roots: named ? choices.map((c) => ({ root_id: c.rootId, mode: c.mode })) : null,
    dry_run: dryRun,
  };
}

function describeLrReport(r) {
  const lines = [];
  if (r.dry_run) lines.push('Dry run — nothing was written. This is a forecast:');
  if (r.cancelled) lines.push('Stopped on request — every count is a partial tally.');
  lines.push(
    // photos_referenced is the subset of photos_in_place that lives on a
    // source other than the library root — the number that answers "how much
    // of my Lightroom library did I take in without copying a byte".
    `photos: ${r.photos_copied} copied · ${r.photos_in_place} in place ` +
    `(${r.photos_referenced} referenced) · ` +
    `${r.photos_linked_existing} already here · ${r.skipped_missing_files} missing on disk`,
    `albums: ${r.albums_created} created · ${r.albums_updated} updated · ` +
    `${r.memberships_added} membership(s) · ${r.tags_added} tag(s)`,
  );
  if (r.bytes_copied) lines.push(`${formatBytes(r.bytes_copied)} copied`);
  // A source is a lasting change to the library — it is what makes those files
  // findable again next time — so a run that adds one has to say so.
  if (r.sources_registered.length) {
    lines.push('', 'Registered as sources — their photos stay where they are:',
      ...r.sources_registered.map((s) => `  ${s}`));
  }
  if (r.collisions.length) {
    lines.push('', 'Album paths already taken:', ...r.collisions.map((c) => `  ${c}`));
  }
  if (r.conflicts.length) {
    lines.push('', 'Left untouched — diverged on both sides:',
      ...r.conflicts.map((c) => `  ${c}`));
  }
  if (r.lr_deleted.length) {
    lines.push('', 'Gone from Lightroom, kept here (deletions never propagate):',
      ...r.lr_deleted.map((d) => `  ${d}`));
  }
  return lines.join('\n');
}

async function runLrImport(dryRun) {
  if (!lrState.path) return;
  const problem = lrPlacementProblem();
  if (problem) {
    showError('lr-error', problem);
    return;
  }
  showError('lr-error', '');
  $('lr-dry-run-btn').disabled = true;
  $('lr-run-btn').disabled = true;
  if (!dryRun) {
    importReach = { processed: 0, total: 0 };
    $('progress').hidden = false;
    $('import-cancel').hidden = false;
    $('import-cancel').disabled = false;
    status('Importing from Lightroom…');
  } else {
    status('Computing dry run…');
  }
  try {
    const report = await invoke('lr_import', {
      lrcatPath: lrState.path,
      options: lrOptions(dryRun),
    });
    $('lr-output').hidden = false;
    $('lr-output').textContent = describeLrReport(report);
    status(dryRun
      ? 'Dry run complete — nothing was written'
      : (report.cancelled ? 'Lightroom import stopped' : 'Lightroom import complete'));
    if (!dryRun) {
      // A referenced root is registered as a source by the run itself.
      await refreshSources();
      await refreshAll();
    }
  } catch (err) {
    showError('lr-error', String(err));
    status('Lightroom import failed');
  } finally {
    $('lr-dry-run-btn').disabled = false;
    $('lr-run-btn').disabled = false;
    $('progress').hidden = true;
    $('import-cancel').hidden = true;
    $('progress-fill').style.width = '0';
  }
}

$('lr-dry-run-btn').addEventListener('click', () => runLrImport(true));
$('lr-run-btn').addEventListener('click', () => runLrImport(false));

// ------------------------------------------------------------------- xmp
//
// Sidecars for interchange: standard fields plus the develop stack under the
// gpp: namespace. Never touches an image file; a sidecar another tool wrote
// is left strictly alone and reported, not replaced.

async function runXmpExport(album, resultEl) {
  status('Writing XMP sidecars…');
  try {
    const o = await invoke('export_xmp', { album });
    // Same as publish: name the drive the skipped sidecars are waiting on,
    // from a listing taken now rather than whenever the panel last looked.
    if (o.offline.length) await refreshSources();
    const text = `${o.written} sidecar(s) written` +
      (o.skipped_foreign.length
        ? ` · ${o.skipped_foreign.length} from other tools left alone`
        : '') +
      (o.missing.length ? ` · ${o.missing.length} original(s) missing on disk` : '') +
      // A sidecar sits beside its original, so a photo whose drive is elsewhere
      // has nowhere to put one yet. Name the drive: "missing on disk" would be
      // the wrong diagnosis and the wrong remedy.
      (o.offline.length ? ` · ${offlineNote(o.offline.length)}` : '');
    if (resultEl) resultEl.textContent = text;
    status(`XMP: ${text}`);
  } catch (err) {
    if (resultEl) resultEl.textContent = String(err);
    status(`XMP export failed: ${err}`);
  }
}

$('export-xmp-all-btn').addEventListener('click', () => runXmpExport(null, null));
$('export-xmp-btn').addEventListener('click', () => {
  if (state.editingAlbum) runXmpExport(state.editingAlbum.path, $('export-xmp-result'));
});

boot();
