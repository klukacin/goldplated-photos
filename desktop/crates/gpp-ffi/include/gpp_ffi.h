/*
 * gpp_ffi.h — C ABI for the Goldplated Photos core.
 *
 * Hand-written and checked in, deliberately. The usual generator, cbindgen, is
 * MPL-2.0 (mozilla/cbindgen), and this project keeps every Rust dependency —
 * build-time ones included — under MIT / Apache-2.0 / BSD / CC0. Four
 * declarations that have no reason to change are cheaper to maintain than a
 * licence exception, and `tests/header_matches_the_code.rs` fails the build if
 * this file and the Rust source ever disagree about a symbol.
 *
 * ---------------------------------------------------------------------------
 * The whole API
 * ---------------------------------------------------------------------------
 *
 * `gpp_core::Session` is the application-level API — roughly forty-five
 * methods, one per UI command. All of them are serde-serialisable, so instead
 * of forty-five C signatures there is one JSON call:
 *
 *     char *r = gpp_call(s, "set_rating", "{\"ids\":[1,2],\"rating\":5}");
 *
 * `args_json` is a JSON object keyed by the method's Rust parameter names,
 * unchanged; it may be NULL for a method that takes none. The reply is always
 * one of these two shapes, never a bare value:
 *
 *     {"ok": <return value, or null for a method that returns nothing>}
 *     {"error": "album not found: 2026/x", "kind": "album-not-found"}
 *
 * Branch on "kind" (a stable tag), show "error" (prose). The tags are:
 * database, io, image, serde, invalid-path, album-not-found, photo-not-found,
 * album-exists, unsupported, sync-conflict, other, plus the boundary's own
 * null-pointer, invalid-utf8, bad-arguments, unknown-method and panic.
 *
 * Method names, and the arguments each takes, are listed in README.md.
 *
 * ---------------------------------------------------------------------------
 * Ownership, in one place
 * ---------------------------------------------------------------------------
 *
 *   gpp_session_new   -> you own it; free with exactly one gpp_session_free.
 *                        Returns NULL on failure — that is the only case with
 *                        nothing to free.
 *   gpp_call          -> you own the returned string; free with exactly one
 *                        gpp_string_free. NEVER returns NULL. Do not call the
 *                        host's free() on it: the library may not share your
 *                        allocator.
 *   gpp_string_free   -> takes ownership. NULL is a no-op. Twice is undefined
 *                        behaviour (heap corruption, usually noticed later and
 *                        elsewhere).
 *   gpp_session_free  -> takes ownership. NULL is a no-op. Twice is undefined
 *                        behaviour. Strings the session produced are separate
 *                        allocations and still need their own frees.
 *
 * Nothing passed in is retained: `method` and `args_json` are borrowed for the
 * duration of the call and may be freed by you as soon as it returns.
 *
 * ---------------------------------------------------------------------------
 * Threads and panics
 * ---------------------------------------------------------------------------
 *
 * The session is Send + Sync: gpp_call may run concurrently on one session from
 * several threads. gpp_session_free is the exception — no call may be in flight
 * when it runs.
 *
 * That promise is what makes a long call stoppable. "cancel_import" raises a
 * flag the running import reads between files and never touches the library
 * itself, so send it from another thread while "import" is still blocking; a
 * client with only one thread has no way to abandon a 2000-frame card.
 *
 * A Rust panic never unwinds into C; it comes back as {"error":...,
 * "kind":"panic"}. Treat that as fatal to the session (its internal lock is
 * poisoned): free it and open the library again.
 */

#ifndef GPP_FFI_H
#define GPP_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque. Size and layout are intentionally not published. */
typedef struct GppSession GppSession;

GppSession *gpp_session_new(void);

char *gpp_call(GppSession *session, const char *method, const char *args_json);

void gpp_string_free(char *ptr);

void gpp_session_free(GppSession *ptr);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* GPP_FFI_H */
