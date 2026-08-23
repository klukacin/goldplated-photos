/* Feature switches for the desktop UI.
 *
 * A plain const object, loaded before app.js so everything can read it at
 * boot. Flipping a value to false removes the feature's controls from the
 * window and makes its entry point refuse. The wiring underneath stays
 * compiled in — a switch here is a UI decision; the build-level ones are
 * cargo features on gpp-core (e.g. `heif`).
 *
 * To add a switch: name it here; hide the feature's buttons at boot where
 * app.js already handles the existing switches (search for FEATURES.); and
 * make the feature's entry function return early when it is off. Both halves
 * matter — hiding alone is not enough, because a keyboard shortcut or a stray
 * event listener can still reach the entry point, and a guard alone leaves a
 * button on screen that does nothing when pressed.
 */
const FEATURES = {
  /// The crop tool: the ⛶ button, the framing overlay and its action row.
  crop: true,
};
