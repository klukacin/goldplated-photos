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
  // Topmost first: the chip popover floats over the modals, the switcher menu
  // over the chrome; then a dialog; only once nothing is open does Escape mean
  // "stop framing".
  if (!$('chip-menu').hidden) {
    $('chip-menu').hidden = true;
  } else if (!$('library-menu').hidden) {
    $('library-menu').hidden = true;
  } else if (modalIsOpen()) {
    document.querySelectorAll('.modal:not([hidden])').forEach((m) => (m.hidden = true));
  } else if (cropMode.active) {
    cancelCrop();
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
  localStorage.setItem(LIBRARY_KEY, path);
  // Everything below here may be a *switch* from another library, so state
  // pointing into the old one — the album filter, the selection, the photo in
  // the inspector — has to go before the new one's contents load.
  resetLibraryState();
  await enterApp(info);
}

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
  renderStatus(info);
  // Sources first: the grid asks which of them are attached while it paints.
  await refreshSources();
  await Promise.all([refreshAlbums(), refreshPhotos()]);
  await loadPublishTarget();
}

function renderStatus(info) {
  state.libraryRoot = info.root;
  const name = info.root.split('/').filter(Boolean).pop() || info.root;
  $('library-name').textContent = name;
  $('all-count').textContent = info.photo_count;
  // Camera names come out of EXIF, which is to say out of the files — escape
  // them. A photo whose Model field is "<b>bold</b>" must not restyle the app.
  $('library-stats').innerHTML =
    `${info.photo_count} photos · ${info.album_count} albums` +
    (info.cameras.length ? `<br>${escapeHtml(info.cameras.join(', '))}` : '');
}

// -------------------------------------------------------- library switcher
//
// The titlebar title is a button; under it, a menu of every library this
// machine has opened (the Rust side records each successful open in
// libraries.json in the app config dir). Switching is nothing special: the
// core persists everything to the catalog as it happens, so there is no save
// prompt — just open the other library and reload, the same path the picker
// takes. Forget edits the list and only the list.

$('library-switcher-btn').addEventListener('click', async (e) => {
  e.stopPropagation();
  const menu = $('library-menu');
  if (!menu.hidden) {
    menu.hidden = true;
    return;
  }
  await renderLibraryMenu();
  menu.hidden = false;
});

// A click anywhere else dismisses the menu, like any dropdown.
document.addEventListener('click', (e) => {
  if (!e.target.closest('.library-switcher')) $('library-menu').hidden = true;
});

async function renderLibraryMenu() {
  const menu = $('library-menu');
  menu.innerHTML = '';

  let libraries = [];
  try {
    libraries = await invoke('known_libraries');
  } catch {
    // An unreadable registry degrades to just the picker entry below.
  }

  libraries.forEach((lib) => {
    const row = document.createElement('div');
    row.className = 'library-row' + (lib.path === state.libraryRoot ? ' current' : '');

    const pick = document.createElement('button');
    pick.className = 'library-pick';
    pick.title = lib.path;
    const name = document.createElement('strong');
    name.textContent = lib.name;
    const path = document.createElement('span');
    path.className = 'muted';
    path.textContent = lib.path;
    pick.append(name, path);
    pick.addEventListener('click', () => switchLibrary(lib));
    row.appendChild(pick);

    const forget = document.createElement('button');
    forget.className = 'library-forget';
    forget.textContent = '×';
    forget.title = 'Forget — removes it from this list only; the library on disk is untouched';
    forget.addEventListener('click', async (e) => {
      e.stopPropagation();
      try {
        await invoke('forget_library', { path: lib.path });
        await renderLibraryMenu();
      } catch (err) {
        status(`Could not forget ${lib.name}: ${err}`);
      }
    });
    row.appendChild(forget);

    menu.appendChild(row);
  });

  // Sources are configuration of the library that is open, so they hang off
  // the library menu rather than a titlebar button of their own.
  const sources = document.createElement('button');
  sources.className = 'library-open-other';
  sources.textContent = 'Sources…';
  sources.title = 'Folders outside the library that it catalogues photographs in';
  sources.addEventListener('click', () => {
    $('library-menu').hidden = true;
    openSources();
  });
  menu.appendChild(sources);

  const other = document.createElement('button');
  other.className = 'library-open-other';
  other.textContent = 'Open other library…';
  other.addEventListener('click', () => {
    $('library-menu').hidden = true;
    pickAndOpenLibrary();
  });
  menu.appendChild(other);
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

async function switchLibrary(lib) {
  $('library-menu').hidden = true;
  if (lib.path === state.libraryRoot) return;
  status(`Opening ${lib.name}…`);
  try {
    await openLibrary(lib.path);
    status(`Opened ${lib.name}`);
  } catch (err) {
    // The core only swaps libraries once the new one opened, so a failed
    // switch — the external drive is unplugged, the folder was moved — leaves
    // the current library on screen untouched. Name the failure and offer to
    // drop the dead entry; declining keeps it for when the drive comes back.
    const forget = await askConfirm(
      `${err}\n\nThe current library stays open. Forget "${lib.name}"? ` +
      `This only removes it from the list — nothing on disk is touched.`,
      { title: `Could not open ${lib.path}`, kind: 'warning', okLabel: 'Forget' }
    );
    if (forget) {
      try {
        await invoke('forget_library', { path: lib.path });
      } catch { /* the entry stays; the menu still works */ }
    }
    status('Library unchanged');
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

  // Framing is about one picture. Moving to another leaves the rectangle
  // describing a frame that is no longer on screen, so put back whatever crop
  // the photo being left had and close the tool.
  if (cropMode.active && photo.id !== cropMode.photoId) cancelCrop();

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
$('inspector-img').addEventListener('load', positionCropOverlay);
window.addEventListener('resize', positionCropOverlay);

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
  let libraries = [];
  try {
    libraries = await invoke('known_libraries');
  } catch { /* no registry — own servers only */ }
  for (const lib of libraries) {
    if (lib.path === state.libraryRoot) continue;
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
