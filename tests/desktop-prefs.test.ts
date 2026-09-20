/**
 * The desktop UI remembers how the photographer arranged it: which inspector
 * panels are open, in what order, which sidebars are showing, what the info
 * overlay says. Reading and writing that is two lines. What needs testing is
 * the third case — a stored value that no longer makes sense.
 *
 * Every one of these is reachable in the wild: downgrade the app and the stored
 * order names a panel this build does not have; upgrade it and the order is
 * missing the panel that was just added; hand-edit localStorage and anything at
 * all can be in there. Left alone, each ends the same way — a panel that exists
 * in the markup and can never be reached from the UI, which is exactly the kind
 * of failure nobody reports because it looks like the feature was never built.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

interface Prefs {
  panelOrder: string[];
  panelOpen: Record<string, boolean>;
  overlay: { on: boolean; fields: string[] };
  sidebars: { left: boolean; right: boolean };
}

// Evaluated rather than imported, because the app loads it as a bare <script>
// with no module system at all — same file, same way, no scaffolding that only
// a test would ever use.
const source = readFileSync(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../desktop/app/ui/prefs.js'),
  'utf8'
);
const {
  PANELS,
  OVERLAY_FIELDS,
  RECENT_LIMIT,
  defaultPrefs,
  normalize,
  loadPrefs,
  savePrefs,
  loadRecent,
  rememberLibrary,
  forgetLibrary,
} = new Function(
  `${source}\nreturn { PANELS, OVERLAY_FIELDS, RECENT_LIMIT, defaultPrefs, normalize, loadPrefs, savePrefs, loadRecent, rememberLibrary, forgetLibrary };`
)() as {
  PANELS: string[];
  OVERLAY_FIELDS: string[];
  RECENT_LIMIT: number;
  defaultPrefs: () => Prefs;
  normalize: (raw: unknown) => Prefs;
  loadPrefs: (storage: Storage, key?: string) => Prefs;
  savePrefs: (storage: Storage, prefs: Prefs, key?: string) => boolean;
  loadRecent: (storage: Storage, key?: string) => string[];
  rememberLibrary: (storage: Storage, path: string, key?: string) => string[];
  forgetLibrary: (storage: Storage, path: string, key?: string) => string[];
};

/** localStorage's surface, with the failures a real one has. */
function fakeStorage(initial: Record<string, string> = {}, opts: { readOnly?: boolean } = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      if (opts.readOnly) throw new DOMException('quota', 'QuotaExceededError');
      map.set(k, v);
    },
    seen: map,
  } as unknown as Storage & { seen: Map<string, string> };
}

describe('normalize', () => {
  it('gives a fresh install info before develop', () => {
    // The photograph comes before what has been done to it.
    expect(defaultPrefs().panelOrder).toEqual(['info', 'develop']);
  });

  it('keeps a stored order the photographer chose', () => {
    expect(normalize({ panelOrder: ['develop', 'info'] }).panelOrder).toEqual([
      'develop',
      'info',
    ]);
  });

  it('appends a panel the stored order never heard of', () => {
    // What an upgrade looks like: the order was written by a build that had
    // only one panel. The new one must appear, not be waited for forever.
    expect(normalize({ panelOrder: ['develop'] }).panelOrder).toEqual(['develop', 'info']);
  });

  it('drops a panel this build no longer has', () => {
    // And what a downgrade looks like.
    const order = normalize({ panelOrder: ['histogram', 'develop', 'info'] }).panelOrder;
    expect(order).toEqual(['develop', 'info']);
  });

  it('never lists a panel twice', () => {
    // A duplicate would render the same panel in two places, and the second
    // would be the live one — every click on the first doing nothing.
    expect(normalize({ panelOrder: ['info', 'info', 'develop'] }).panelOrder).toEqual([
      'info',
      'develop',
    ]);
  });

  it('always ends up with every panel exactly once, whatever went in', () => {
    for (const stored of [null, undefined, 42, 'info', [], {}, { panelOrder: 'info' }]) {
      const order = normalize(stored as unknown).panelOrder;
      expect([...order].sort()).toEqual([...PANELS].sort());
    }
  });

  it('treats a missing open-state as the default, not as closed', () => {
    // An absent key is a version that never wrote it. Reading it as `false`
    // would collapse every panel on the first launch after an upgrade.
    expect(normalize({ panelOpen: {} }).panelOpen.develop).toBe(true);
    expect(normalize({ panelOpen: { develop: null } }).panelOpen.develop).toBe(true);
  });

  it('honours a panel the photographer actually closed', () => {
    expect(normalize({ panelOpen: { develop: false } }).panelOpen.develop).toBe(false);
  });

  it('starts the overlay off — it covers the photograph', () => {
    expect(defaultPrefs().overlay.on).toBe(false);
  });

  it('orders overlay fields by what this build draws, not by what was stored', () => {
    // Which fields appear is the photographer's choice; where they appear is
    // the code's. A stored order would otherwise freeze a layout decision.
    const fields = normalize({
      overlay: { fields: ['size', 'filename', 'camera'] },
    }).overlay.fields;
    expect(fields).toEqual(OVERLAY_FIELDS.filter((f) => fields.includes(f)));
    expect(fields).toContain('filename');
    expect(fields).not.toContain('lens');
  });

  it('ignores an overlay field this build cannot draw', () => {
    expect(normalize({ overlay: { fields: ['filename', 'gps'] } }).overlay.fields).toEqual([
      'filename',
    ]);
  });

  it('lets the overlay be emptied without falling back to every field', () => {
    // An empty list is a choice. Treating it as "unset" would put every row
    // back the next time the app started.
    expect(normalize({ overlay: { fields: [] } }).overlay.fields).toEqual([]);
  });

  it('keeps both sidebars unless told otherwise', () => {
    expect(normalize({ sidebars: { left: false } })).toMatchObject({
      sidebars: { left: false, right: true },
    });
  });
});

describe('recent libraries', () => {
  it('puts the newest first — that is the one wanted next', () => {
    const storage = fakeStorage();
    rememberLibrary(storage, '/a');
    rememberLibrary(storage, '/b');
    expect(loadRecent(storage)).toEqual(['/b', '/a']);
  });

  it('moves a re-opened library to the front instead of listing it twice', () => {
    // Opening the same library every morning must not fill the switcher with
    // eight copies of it and push every other one off the end.
    const storage = fakeStorage();
    rememberLibrary(storage, '/a');
    rememberLibrary(storage, '/b');
    rememberLibrary(storage, '/a');
    expect(loadRecent(storage)).toEqual(['/a', '/b']);
  });

  it('keeps the list a list, not a history', () => {
    const storage = fakeStorage();
    for (let i = 0; i < RECENT_LIMIT + 5; i++) rememberLibrary(storage, `/lib-${i}`);
    const list = loadRecent(storage);
    expect(list).toHaveLength(RECENT_LIMIT);
    expect(list[0]).toBe(`/lib-${RECENT_LIMIT + 4}`);
  });

  it('drops entries that are not paths', () => {
    const storage = fakeStorage({
      'gpp.recentLibraries': JSON.stringify(['/good', 42, null, '', '   ', { path: '/x' }]),
    });
    expect(loadRecent(storage)).toEqual(['/good']);
  });

  it('reads unusable storage as an empty list rather than throwing', () => {
    // The switcher has to open even when this key is nonsense.
    expect(loadRecent(fakeStorage({ 'gpp.recentLibraries': 'not json' }))).toEqual([]);
    expect(loadRecent(fakeStorage({ 'gpp.recentLibraries': '{"a":1}' }))).toEqual([]);
    expect(loadRecent(fakeStorage())).toEqual([]);
  });

  it('forgets one without disturbing the rest', () => {
    const storage = fakeStorage();
    rememberLibrary(storage, '/a');
    rememberLibrary(storage, '/b');
    rememberLibrary(storage, '/c');
    expect(forgetLibrary(storage, '/b')).toEqual(['/c', '/a']);
    expect(loadRecent(storage)).toEqual(['/c', '/a']);
  });

  it('survives storage that refuses to be written', () => {
    // Losing the history must not take down the library that just opened.
    const storage = fakeStorage({}, { readOnly: true });
    expect(() => rememberLibrary(storage, '/a')).not.toThrow();
    expect(() => forgetLibrary(storage, '/a')).not.toThrow();
  });
});

describe('loadPrefs / savePrefs', () => {
  it('round-trips an arrangement', () => {
    const storage = fakeStorage();
    const prefs = defaultPrefs();
    prefs.panelOrder = ['develop', 'info'];
    prefs.panelOpen.info = false;
    prefs.overlay.on = true;
    prefs.sidebars.left = false;

    expect(savePrefs(storage, prefs)).toBe(true);
    expect(loadPrefs(storage)).toEqual(prefs);
  });

  it('falls back to defaults on unparsable storage rather than throwing', () => {
    // A half-written value, or a key another tool put there. The window still
    // has to come up.
    expect(loadPrefs(fakeStorage({ 'gpp.prefs': '{not json' }))).toEqual(defaultPrefs());
  });

  it('reads an empty profile as defaults', () => {
    expect(loadPrefs(fakeStorage())).toEqual(defaultPrefs());
  });

  it('survives storage that refuses to be written', () => {
    // A blocked or full profile must not take down the click that changed the
    // setting — the arrangement simply does not outlive the session.
    expect(savePrefs(fakeStorage({}, { readOnly: true }), defaultPrefs())).toBe(false);
  });
});
