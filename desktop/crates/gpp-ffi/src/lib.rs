//! C ABI for the Goldplated Photos core.
//!
//! `gpp-core` is portable, `Send + Sync` Rust with no GUI and no async
//! runtime, and it already compiles for `aarch64-apple-ios`. What it did not
//! have was a door: a Swift iPad app or a Kotlin Android app had nothing to
//! hold on to. This crate is that door, and nothing else — it contains no
//! behaviour of its own.
//!
//! # Shape
//!
//! Four functions, and one of them does all the work:
//!
//! ```c
//! GppSession *gpp_session_new(void);
//! char       *gpp_call(GppSession *, const char *method, const char *args_json);
//! void        gpp_string_free(char *);
//! void        gpp_session_free(GppSession *);
//! ```
//!
//! [`gpp_core::Session`] has around forty-five methods and every one of them
//! already serialises — that is what the Tauri shell has been relying on. So
//! rather than hand-writing forty-five C signatures that would drift the first
//! time a field was added, the whole surface goes through one JSON call.
//! The checked-in header is then four declarations that never change, and
//! adding a method to the core is a match arm rather than an ABI break.
//!
//! The price is that the type checking happens at run time, inside serde,
//! instead of at compile time in the caller's language. Every argument struct
//! sets `deny_unknown_fields`, so a misspelled key is a returned error and not
//! a value that silently did nothing — the same reasoning that put
//! `deny_unknown_fields` on [`gpp_core::model::PhotoFilter`] after it swallowed
//! `minRating`.
//!
//! # Naming
//!
//! Method names are [`gpp_core::Session`]'s method names and argument keys are
//! its parameter names, both unchanged. Nested objects and every return value
//! are the core's own serde shapes, which means `snake_case` throughout —
//! except [`gpp_core::model::PhotoFilter`], [`gpp_core::albums::NewAlbum`] and
//! [`gpp_core::albums::AlbumUpdate`], which the web UI made `camelCase` long
//! before this crate existed. That seam is the core's, and renaming it here
//! would only move the surprise onto the Tauri shell.
//!
//! # Result envelope
//!
//! `gpp_call` always returns a JSON object, and it is always one of:
//!
//! ```json
//! {"ok": <the method's return value, or null for a method that returns nothing>}
//! {"error": "album not found: 2026/x", "kind": "album-not-found"}
//! ```
//!
//! There is no third shape and no bare value, so a caller never has to guess
//! whether it is looking at a result or a failure. `kind` is a stable tag —
//! branch on that; `error` is prose for a human.
//!
//! # Panics never cross the boundary
//!
//! Unwinding out of an `extern "C"` function is undefined behaviour. Every
//! entry point wraps its work in [`std::panic::catch_unwind`]; a panic in the
//! core comes back as `{"error": ..., "kind": "panic"}`.
//!
//! One honest consequence: [`gpp_core::Session`] holds its library behind an
//! `RwLock`, and a panic while that lock is held poisons it. The session is
//! then still safe to touch — every later call returns an error rather than
//! misbehaving — but it is no longer useful. Treat `"kind": "panic"` as fatal
//! to that session: free it and open the library again.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::Value;

use gpp_core::Session;

mod dispatch;

pub use dispatch::METHODS;

/// An open (or not yet opened) library session.
///
/// Opaque to C: its size and contents are deliberately not in the header, so
/// the Rust side can change freely without recompiling callers. Allocate with
/// [`gpp_session_new`], release with [`gpp_session_free`].
///
/// The underlying [`Session`] is `Send + Sync`, so it is safe to call
/// [`gpp_call`] on the same session from several threads at once. Freeing is
/// the exception: no call may be in flight when [`gpp_session_free`] runs.
pub struct GppSession {
    session: Session,
}

/// Returned when even the error envelope could not be built. Unreachable in
/// practice — kept so that the "never returns NULL" promise has no hole in it.
const FALLBACK_JSON: &str = r#"{"error":"the result could not be encoded","kind":"other"}"#;

/// Create a session. No library is open yet — call `open_library` first.
///
/// # Ownership
///
/// The caller owns the returned pointer and must release it with exactly one
/// call to [`gpp_session_free`]. Freeing it twice, or passing it to anything
/// other than `gpp_session_free`, is undefined behaviour.
///
/// Returns `NULL` if the session could not be created; a `NULL` return is the
/// only case in which the caller has nothing to free.
#[no_mangle]
pub extern "C" fn gpp_session_new() -> *mut GppSession {
    catch_unwind(|| {
        Box::into_raw(Box::new(GppSession {
            session: Session::new(),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Call one core method by name.
///
/// `method` is a NUL-terminated UTF-8 name — the list is in [`METHODS`] and the
/// README. `args_json` is a NUL-terminated UTF-8 JSON **object** whose keys are
/// that method's parameter names in `snake_case`; it may be `NULL` or empty for
/// a method that takes no arguments.
///
/// # Ownership
///
/// `session`, `method` and `args_json` stay owned by the caller and are only
/// borrowed for the duration of the call — this function keeps no reference to
/// them and does not free them.
///
/// The **return value is owned by the caller** and must be released with
/// exactly one call to [`gpp_string_free`]. It is never `NULL`, always
/// NUL-terminated, always valid UTF-8, and always the JSON envelope described
/// at the top of this module — including when the arguments were nonsense or
/// the session pointer was `NULL`. Releasing it with the host language's own
/// `free` is undefined behaviour on platforms where the library and the caller
/// do not share an allocator, which is why `gpp_string_free` exists at all.
///
/// # Safety
///
/// `session` must be either `NULL` or a pointer returned by
/// [`gpp_session_new`] that has not yet been freed. `method` and `args_json`
/// must each be either `NULL` or a valid pointer to a NUL-terminated byte
/// string that stays alive and unmodified for the duration of the call.
/// A dangling or already-freed pointer cannot be detected here and is
/// undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn gpp_call(
    session: *mut GppSession,
    method: *const c_char,
    args_json: *const c_char,
) -> *mut c_char {
    guarded(|| call_json(session, method, args_json))
}

/// Run the body of a call, turning a panic into an error envelope and the
/// result into a string C owns.
///
/// Separated from [`gpp_call`] so the guard itself can be tested: the only way
/// to prove a panic is caught is to cause one, and there is no method in the
/// table whose job is to fail.
fn guarded(body: impl FnOnce() -> String) -> *mut c_char {
    // `AssertUnwindSafe` because the thing being asserted is true for a
    // different reason than the auto-trait can see: the session's lock poisons
    // on panic, and every later call then fails loudly at the lock rather than
    // reading a half-written value. Broken, but not unsound — which is exactly
    // the distinction `UnwindSafe` is about.
    let json = catch_unwind(AssertUnwindSafe(body))
        .unwrap_or_else(|payload| error_json("panic", &panic_message(&*payload)));
    into_owned_c_string(json)
}

/// Release a string returned by [`gpp_call`].
///
/// # Ownership
///
/// Takes ownership of the pointer: after this returns, the memory is gone and
/// the pointer is dangling. Call it exactly once per string that `gpp_call`
/// handed out. Passing `NULL` is a no-op and always safe. Calling it twice on
/// the same pointer, or on a pointer this library did not return, is undefined
/// behaviour — most often a heap corruption that surfaces somewhere else
/// entirely.
///
/// # Safety
///
/// `ptr` must be `NULL` or a pointer returned by [`gpp_call`] that has not
/// already been passed to this function.
#[no_mangle]
pub unsafe extern "C" fn gpp_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // Reclaiming the allocation cannot panic, but dropping a `CString` runs
    // the allocator, and an allocator that aborts is better contained here
    // than allowed to unwind into C.
    let _ = catch_unwind(AssertUnwindSafe(|| drop(CString::from_raw(ptr))));
}

/// Release a session created by [`gpp_session_new`].
///
/// # Ownership
///
/// Takes ownership: after this returns the pointer is dangling. Call it exactly
/// once per session. Passing `NULL` is a no-op and always safe. A second call
/// on the same pointer is undefined behaviour, as is any [`gpp_call`] that
/// starts after — or is still running during — this one.
///
/// Freeing a session does not free strings it produced; those are independent
/// allocations and each still needs its own [`gpp_string_free`].
///
/// # Safety
///
/// `ptr` must be `NULL` or a pointer returned by [`gpp_session_new`] that has
/// not already been freed, and no other thread may be inside [`gpp_call`] with
/// that same pointer.
#[no_mangle]
pub unsafe extern "C" fn gpp_session_free(ptr: *mut GppSession) {
    if ptr.is_null() {
        return;
    }
    // Closing the catalog runs SQLite's teardown; keep any unwind from it on
    // this side of the boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(ptr))));
}

// ---------------------------------------------------------------- internals

/// The body of [`gpp_call`], separated so the panic guard wraps everything
/// including argument decoding.
///
/// # Safety
///
/// Same requirements as [`gpp_call`].
unsafe fn call_json(
    session: *mut GppSession,
    method: *const c_char,
    args_json: *const c_char,
) -> String {
    let session = match session.as_ref() {
        Some(s) => s,
        None => return error_json("null-pointer", "the session pointer was NULL"),
    };

    let method = match borrow_str(method, "method") {
        Ok(Some(m)) => m,
        // A missing method name is a different mistake from a mangled one, and
        // a caller staring at a log deserves to be told which.
        Ok(None) => return error_json("null-pointer", "the method pointer was NULL"),
        Err(msg) => return error_json("invalid-utf8", &msg),
    };

    // No arguments at all is a legitimate call, so NULL here means `{}`.
    let args = match borrow_str(args_json, "args_json") {
        Ok(Some(a)) => a,
        Ok(None) => "",
        Err(msg) => return error_json("invalid-utf8", &msg),
    };

    match dispatch::dispatch(&session.session, method, args) {
        Ok(value) => ok_json(value),
        Err(e) => error_json(e.kind, &e.message),
    }
}

/// Borrow a C string as `&str`, distinguishing "absent" from "not UTF-8".
///
/// # Safety
///
/// `ptr` must be `NULL` or point to a NUL-terminated string that outlives the
/// returned borrow.
unsafe fn borrow_str<'a>(ptr: *const c_char, name: &str) -> Result<Option<&'a str>, String> {
    if ptr.is_null() {
        return Ok(None);
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(Some)
        .map_err(|e| format!("{name} was not valid UTF-8: {e}"))
}

fn ok_json(value: Value) -> String {
    // Building `{"ok": v}` by hand rather than through a wrapper struct keeps
    // this total: `Value` is already known-encodable, so there is no error path
    // left to handle at the point where handling one is most awkward.
    let mut out = String::from(r#"{"ok":"#);
    match serde_json::to_string(&value) {
        Ok(s) => out.push_str(&s),
        Err(e) => return error_json("serde", &format!("could not encode the result: {e}")),
    }
    out.push('}');
    out
}

fn error_json(kind: &str, message: &str) -> String {
    let mut out = String::from(r#"{"error":"#);
    match serde_json::to_string(message) {
        Ok(s) => out.push_str(&s),
        Err(_) => return FALLBACK_JSON.to_string(),
    }
    out.push_str(r#","kind":"#);
    match serde_json::to_string(kind) {
        Ok(s) => out.push_str(&s),
        Err(_) => return FALLBACK_JSON.to_string(),
    }
    out.push('}');
    out
}

/// Recover whatever a panic carried, so the caller gets the message rather than
/// a bare "something panicked".
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        format!("the core panicked: {s}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("the core panicked: {s}")
    } else {
        "the core panicked".to_string()
    }
}

/// Hand a `String` to C as an owned NUL-terminated buffer.
///
/// Never returns `NULL`: [`gpp_call`] promises a readable envelope in every
/// case, and a caller that has to null-check a documented-non-null return is
/// a caller that will forget to.
fn into_owned_c_string(json: String) -> *mut c_char {
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        // Unreachable: serde_json escapes a NUL byte into a six-character
        // escape sequence, so its output never contains one. If that ever
        // changed, a terse envelope is still better than a NULL the caller was
        // promised could not happen.
        Err(_) => CString::new(FALLBACK_JSON).unwrap_or_default().into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a reply back the way C would, and give the allocation up again.
    fn take(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        unsafe {
            let text = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
            gpp_string_free(ptr);
            text
        }
    }

    /// Unwinding out of an `extern "C"` function is undefined behaviour, so
    /// this is the one property in the crate that must not regress quietly.
    #[test]
    fn a_panic_becomes_an_error_envelope_instead_of_crossing_the_boundary() {
        // A `&'static str` payload — `panic!("literal")`.
        let reply = take(guarded(|| panic!("boom")));
        assert_eq!(reply, r#"{"error":"the core panicked: boom","kind":"panic"}"#);

        // A `String` payload — `panic!("{}", value)`, and what `.expect()` on a
        // poisoned lock produces.
        let reply = take(guarded(|| panic!("{}", String::from("lock poisoned"))));
        assert_eq!(
            reply,
            r#"{"error":"the core panicked: lock poisoned","kind":"panic"}"#
        );

        // A payload of some other type still has to produce a valid envelope.
        let reply = take(guarded(|| std::panic::panic_any(7u8)));
        assert_eq!(reply, r#"{"error":"the core panicked","kind":"panic"}"#);
    }

    /// The envelope is JSON, so a message full of quotes and newlines has to
    /// come out escaped rather than breaking the document around it.
    #[test]
    fn an_error_message_is_escaped_into_the_envelope() {
        let json = error_json("other", "he said \"no\"\nand left\ttabbed");
        let parsed: Value = serde_json::from_str(&json).expect("the envelope must parse");
        assert_eq!(parsed["error"], "he said \"no\"\nand left\ttabbed");
        assert_eq!(parsed["kind"], "other");
    }

    /// `()` serializes to `null`, so a method that returns nothing still gets
    /// an `ok` key rather than an empty object the caller has to special-case.
    #[test]
    fn a_method_that_returns_nothing_still_reports_success() {
        assert_eq!(ok_json(Value::Null), r#"{"ok":null}"#);
    }
}
