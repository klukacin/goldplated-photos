/**
 * The develop panel coalesces adjustments so that holding an arrow key on a
 * slider does not hand the core a dozen full renders of the whole selection.
 * Coalescing is where work gets dropped, and dropped work here is silent: the
 * photo simply keeps the value it had, and the photographer finds out later.
 *
 * These drive the queue directly, with a render that takes as long as the test
 * says, so the interleavings that need seconds of real timing land instantly.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

interface DevelopQueue {
  queue(kind: string, ids: string[], run: () => Promise<void>): void;
  queueRelative(
    kind: string,
    ids: string[],
    send: (ids: string[], presses: number) => Promise<void>
  ): void;
  forgetFor(ids: string[]): void;
  pendingCount(): number;
}

type MakeQueue = (deps: {
  onError: (err: unknown) => void;
  onSettled: () => void | Promise<void>;
}) => DevelopQueue;

// Evaluated rather than imported, because the app loads it as a bare <script>
// with no module system at all — so this exercises the same file the same way,
// and the file needs no export scaffolding that only a test would ever use.
const source = readFileSync(
  path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    '../desktop/app/ui/develop-queue.js'
  ),
  'utf8'
);
const createDevelopQueue: MakeQueue = new Function(
  `${source}\nreturn createDevelopQueue;`
)();

/** A queue whose failures and repaints are recorded rather than displayed. */
function harness() {
  const errors: unknown[] = [];
  let settled = 0;
  const q = createDevelopQueue({
    onError: (e) => errors.push(e),
    onSettled: () => {
      settled += 1;
    },
  });
  return { q, errors, settledCount: () => settled };
}

/** Let the queue drain: it is driven by promises, not timers. */
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('develop queue', () => {
  it('applies an adjustment to each photo when two are adjusted in a burst', async () => {
    // The bug this pins: keyed by slider alone, exposure on photo A and then
    // exposure on photo B collapsed into one entry, B replaced A, and A never
    // got its adjustment. The window was however long the core takes to render
    // a selection — for a shoot, seconds.
    const { q } = harness();
    const applied: Array<[string, number]> = [];
    let release!: () => void;
    const firstRenderRunning = new Promise<void>((r) => {
      release = r;
    });

    // The render in flight, holding the queue open the way a real one does.
    q.queue('exposure', ['a'], async () => {
      applied.push(['a-blocking', 0]);
      await firstRenderRunning;
    });
    await settle();

    q.queue('exposure', ['a'], async () => {
      applied.push(['a', 0.4]);
    });
    q.queue('exposure', ['b'], async () => {
      applied.push(['b', -0.2]);
    });

    release();
    await settle();

    expect(applied).toContainEqual(['a', 0.4]);
    expect(applied).toContainEqual(['b', -0.2]);
  });

  it('keeps only the newest value for the same slider on the same photos', async () => {
    // The point of coalescing: dragging a slider must not queue every value it
    // passed through on the way.
    const { q } = harness();
    const applied: number[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['a'], async () => {
      await blocked;
    });
    await settle();

    for (const v of [0.1, 0.2, 0.3]) {
      q.queue('exposure', ['a'], async () => {
        applied.push(v);
      });
    }

    release();
    await settle();
    expect(applied).toEqual([0.3]);
  });

  it('gives a different slider its own entry', async () => {
    const { q } = harness();
    const applied: string[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['a'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['a'], async () => {
      applied.push('exposure');
    });
    q.queue('contrast', ['a'], async () => {
      applied.push('contrast');
    });

    release();
    await settle();
    expect(applied.sort()).toEqual(['contrast', 'exposure']);
  });

  it('adds up rotate presses instead of dropping them', async () => {
    // Sliders carry a value, so the newest wins. Rotate is relative: drop a
    // press and the photo ends up at an angle nobody asked for.
    const { q } = harness();
    const sent: number[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['a'], async () => {
      await blocked;
    });
    await settle();

    for (let i = 0; i < 3; i++) {
      q.queueRelative('rotate1', ['a'], async (_ids, presses) => {
        sent.push(presses);
      });
    }

    release();
    await settle();
    expect(sent).toEqual([3]);
  });

  it('counts rotate presses per selection, not across selections', async () => {
    const { q } = harness();
    const sent: Array<[string, number]> = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queueRelative('rotate1', ['a'], async (ids, presses) => {
      sent.push([ids.join(','), presses]);
    });
    q.queueRelative('rotate1', ['a'], async (ids, presses) => {
      sent.push([ids.join(','), presses]);
    });
    q.queueRelative('rotate1', ['b'], async (ids, presses) => {
      sent.push([ids.join(','), presses]);
    });

    release();
    await settle();
    expect(sent.sort()).toEqual([
      ['a', 2],
      ['b', 1],
    ]);
  });

  it('a reset drops queued work for its own photos', async () => {
    const { q } = harness();
    const applied: string[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['a'], async () => {
      applied.push('exposure-a');
    });
    q.forgetFor(['a']);
    q.queue('reset', ['a'], async () => {
      applied.push('reset-a');
    });

    release();
    await settle();
    expect(applied).toEqual(['reset-a']);
  });

  it('a reset leaves another selection’s queued work alone', async () => {
    // Clearing the whole queue used to throw away an adjustment someone had
    // just made to a different photo.
    const { q } = harness();
    const applied: string[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['b'], async () => {
      applied.push('exposure-b');
    });
    q.forgetFor(['a']);
    q.queue('reset', ['a'], async () => {
      applied.push('reset-a');
    });

    release();
    await settle();
    expect(applied.sort()).toEqual(['exposure-b', 'reset-a']);
  });

  it('a reset also discards rotate presses it supersedes', async () => {
    // The tally lives beside the job. Drop one without the other and the next
    // rotate replays presses the reset was supposed to have undone.
    const { q } = harness();
    const sent: number[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queueRelative('rotate1', ['a'], async (_ids, presses) => {
      sent.push(presses);
    });
    q.queueRelative('rotate1', ['a'], async (_ids, presses) => {
      sent.push(presses);
    });
    q.forgetFor(['a']);

    // A later rotate must start from zero, not inherit the two discarded ones.
    q.queueRelative('rotate1', ['a'], async (_ids, presses) => {
      sent.push(presses);
    });

    release();
    await settle();
    expect(sent).toEqual([1]);
  });

  it('drops queued work for a multi-photo selection that overlaps the reset', async () => {
    // A pending edit on [a, b] is meaningless once a is reset, even though the
    // selections are not equal.
    const { q } = harness();
    const applied: string[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['a', 'b'], async () => {
      applied.push('exposure-ab');
    });
    q.forgetFor(['b']);

    release();
    await settle();
    expect(applied).toEqual([]);
  });

  it('a failing job does not stall the ones behind it', async () => {
    const { q, errors } = harness();
    const applied: string[] = [];
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['a'], async () => {
      throw new Error('core said no');
    });
    q.queue('contrast', ['a'], async () => {
      applied.push('contrast');
    });

    release();
    await settle();
    expect(applied).toEqual(['contrast']);
    expect(errors).toHaveLength(1);
  });

  it('repaints once for a whole burst, not once per adjustment', async () => {
    const { q, settledCount } = harness();
    let release!: () => void;
    const blocked = new Promise<void>((r) => {
      release = r;
    });

    q.queue('exposure', ['z'], async () => {
      await blocked;
    });
    await settle();

    q.queue('exposure', ['a'], async () => {});
    q.queue('contrast', ['a'], async () => {});
    q.queue('saturation', ['a'], async () => {});

    release();
    await settle();
    expect(settledCount()).toBe(1);
  });
});
