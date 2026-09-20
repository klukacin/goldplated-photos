// Persisted UI preferences for the desktop shell.
//
// What the photographer arranged — which panels are open, in what order, which
// sidebars are showing, what the overlay says — has to survive a restart, or it
// is not an arrangement, it is a thing to redo every morning.
//
// The reading and writing is two lines; the part worth having on its own is
// what happens to a stored value that no longer makes sense. A panel this
// version does not have, a panel it has that the stored order does not mention,
// the same panel listed twice, a field removed from the overlay, `null` where a
// boolean should be — every one of those is reachable by downgrading, upgrading
// or hand-editing localStorage, and every one of them, left alone, ends with a
// panel that cannot be reached from the UI at all. `normalize` is what makes
// them all land on something usable, and it lives here, apart from the DOM, so
// a test can put any of them in without a window.

/// The panels the inspector can show, in the order a fresh install gets them.
/// Info first: what the photograph *is* comes before what has been done to it.
const PANELS = ['info', 'develop'];

/// Rows the info overlay can draw, in the order it draws them.
const OVERLAY_FIELDS = [
  'filename',
  'captured',
  'camera',
  'lens',
  'exposure',
  'dimensions',
  'size',
  'rating',
];

function defaultPrefs() {
  return {
    panelOrder: [...PANELS],
    panelOpen: { info: true, develop: true },
    /// The overlay starts off: it covers the photograph, and someone opening
    /// the app to look at pictures should be shown pictures.
    overlay: { on: false, fields: [...OVERLAY_FIELDS] },
    sidebars: { left: true, right: true },
  };
}

/// A boolean, whatever was stored. `undefined` keeps the default rather than
/// reading as false — an absent key is a version that never wrote it, not a
/// photographer who turned it off.
function asBool(value, fallback) {
  return typeof value === 'boolean' ? value : fallback;
}

/// Make any stored value usable, without ever losing a panel.
///
/// The order is rebuilt rather than corrected in place: keep the known panels
/// the stored order lists, in that order, then append every panel it failed to
/// mention. Dropping the unknown ones and appending the missing ones is what
/// makes a downgrade and an upgrade both survivable — the panel a newer version
/// added is simply on the end, and the one it removed is gone.
function normalize(raw) {
  const base = defaultPrefs();
  if (!raw || typeof raw !== 'object') return base;

  const listed = Array.isArray(raw.panelOrder) ? raw.panelOrder : [];
  const kept = [];
  for (const name of listed) {
    if (PANELS.includes(name) && !kept.includes(name)) kept.push(name);
  }
  for (const name of PANELS) {
    if (!kept.includes(name)) kept.push(name);
  }

  const open = {};
  for (const name of PANELS) {
    open[name] = asBool(raw.panelOpen?.[name], base.panelOpen[name]);
  }

  const storedFields = Array.isArray(raw.overlay?.fields) ? raw.overlay.fields : null;
  // Order by what this version draws, not by what was stored: a field's place
  // in the overlay is the code's decision, only whether it appears is the
  // photographer's.
  const fields = storedFields
    ? OVERLAY_FIELDS.filter((f) => storedFields.includes(f))
    : [...OVERLAY_FIELDS];

  return {
    panelOrder: kept,
    panelOpen: open,
    overlay: { on: asBool(raw.overlay?.on, base.overlay.on), fields },
    sidebars: {
      left: asBool(raw.sidebars?.left, base.sidebars.left),
      right: asBool(raw.sidebars?.right, base.sidebars.right),
    },
  };
}

// --------------------------------------------------------- recent libraries
//
// A library is a folder the photographer chose, and photographers have more
// than one — this shoot's card, last year's archive, the drive that is only
// plugged in sometimes. Remembering the last one is not a switcher; remembering
// the last few is.
//
// Stored apart from the arrangement above because it is a different kind of
// thing: paths from a file picker, not settings. Same repair problem though —
// the list is written on every open, so a duplicate, a stale entry or a value
// that is not a path at all all reach it.

const RECENT_KEY = 'gpp.recentLibraries';

/// How many to keep. Long enough to cover the drives someone actually works
/// from, short enough that the list stays a list and not a history.
const RECENT_LIMIT = 8;

/// Strings only, no duplicates, newest first, capped.
///
/// Order is meaning here, unlike the overlay's fields: the most recently opened
/// library goes first because that is the one most likely wanted next.
function normalizeRecent(raw) {
  if (!Array.isArray(raw)) return [];
  const seen = new Set();
  const out = [];
  for (const entry of raw) {
    if (typeof entry !== 'string') continue;
    const path = entry.trim();
    if (!path || seen.has(path)) continue;
    seen.add(path);
    out.push(path);
    if (out.length === RECENT_LIMIT) break;
  }
  return out;
}

function loadRecent(storage, key = RECENT_KEY) {
  try {
    return normalizeRecent(JSON.parse(storage.getItem(key)));
  } catch {
    return [];
  }
}

/// Put a library at the front, keeping the list clean. Returns the new list.
function rememberLibrary(storage, path, key = RECENT_KEY) {
  const list = normalizeRecent([path, ...loadRecent(storage, key)]);
  try {
    storage.setItem(key, JSON.stringify(list));
  } catch {
    // A blocked profile loses the history, not the library that just opened.
  }
  return list;
}

/// Drop one — a folder that has been moved, or one the photographer is done
/// with. Never touches the library itself, only this list.
function forgetLibrary(storage, path, key = RECENT_KEY) {
  const list = loadRecent(storage, key).filter((entry) => entry !== path);
  try {
    storage.setItem(key, JSON.stringify(list));
  } catch {
    // As above.
  }
  return list;
}

const PREFS_KEY = 'gpp.prefs';

/// Read the stored preferences. Unreadable or unparsable storage is not an
/// error worth showing anyone — it means the defaults, and the next save fixes
/// it. A private window or a cleared profile both arrive here.
function loadPrefs(storage, key = PREFS_KEY) {
  try {
    return normalize(JSON.parse(storage.getItem(key)));
  } catch {
    return defaultPrefs();
  }
}

/// Write them back. Also non-fatal: a full or blocked storage must not take
/// down the click that changed the setting.
function savePrefs(storage, prefs, key = PREFS_KEY) {
  try {
    storage.setItem(key, JSON.stringify(prefs));
    return true;
  } catch {
    return false;
  }
}
