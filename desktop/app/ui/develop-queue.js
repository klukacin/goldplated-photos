// Coalescing queue for develop adjustments.
//
// Every applied adjustment re-renders each selected photo, so the core must not
// be handed one job per keystroke: holding an arrow key on a slider would queue
// a dozen full renders of the whole selection and leave the grid showing pixels
// from several values ago. Work is queued by adjustment instead, so a newer
// value replaces the pending one and a different slider gets its own entry.
//
// The key includes *which photos*, not only which slider. Keyed by slider
// alone, adjusting exposure on one photo and then immediately on another
// replaces the first job with the second and the first photo never gets its
// adjustment — and the window for that is however long the core takes to render
// a selection, which for a shoot is seconds, not milliseconds.
//
// This lives apart from app.js because it is the one piece of the panel that is
// pure scheduling: no DOM, no IPC, so it can be driven by a test at speeds and
// interleavings a hand on a mouse could never reproduce. app.js passes in what
// to do on failure and what to repaint once the burst has drained.

function createDevelopQueue({ onError, onSettled }) {
  /** Jobs waiting to run, newest value per (adjustment, selection). */
  const pending = new Map();
  /**
   * Presses counted but not yet sent, under the same key as the job that will
   * send them. Only relative adjustments — rotate and the flips — use this;
   * see `queueRelative`.
   */
  const presses = new Map();
  let draining = false;

  const keyFor = (kind, ids) => `${kind}:${ids.join(',')}`;

  async function drain() {
    if (draining) return;
    draining = true;
    try {
      while (pending.size) {
        const [key, job] = pending.entries().next().value;
        pending.delete(key);
        try {
          await job.run();
        } catch (err) {
          onError(err);
        }
      }
    } finally {
      draining = false;
    }
    // One repaint for the whole burst, once the core has caught up.
    await onSettled();
  }

  return {
    /** Queue an absolute adjustment: the newest value wins outright. */
    queue(kind, ids, run) {
      pending.set(keyFor(kind, ids), { ids, run });
      drain();
    },

    /**
     * Queue a relative adjustment. Sliders send a value, so the newest may
     * simply replace the pending one; rotate and the flips are relative — drop
     * a press and the photo ends up at an angle nobody asked for. Presses on
     * the same photos therefore add up into a single call once the core
     * catches up, and `send` is handed the total.
     */
    queueRelative(kind, ids, send) {
      const key = keyFor(kind, ids);
      presses.set(key, (presses.get(key) || 0) + 1);
      this.queue(kind, ids, async () => {
        const count = presses.get(key) || 0;
        presses.delete(key);
        if (count) await send(ids, count);
      });
    },

    /**
     * Drop queued work for photos that are about to be overruled — a reset
     * makes any adjustment still waiting for those photos meaningless. Work
     * queued for *other* photos is somebody's edit and stays.
     */
    forgetFor(ids) {
      const targets = new Set(ids);
      for (const [key, job] of pending) {
        if (!job.ids.some((id) => targets.has(id))) continue;
        pending.delete(key);
        // A relative adjustment keeps its tally under the same key, so dropping
        // the job has to drop the presses counted for it too — otherwise the
        // next rotate on those photos would replay the ones this just discarded.
        presses.delete(key);
      }
    },

    /** For tests and diagnostics only. */
    pendingCount: () => pending.size,
  };
}
