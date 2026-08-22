//! The boundary, driven the way a Swift or Kotlin caller drives it.
//!
//! Nothing here reaches into Rust behind the door: every call goes through
//! `gpp_call` with NUL-terminated byte strings, every reply is read back out of
//! a `char *` and then handed to `gpp_string_free`, and every session is
//! released. If a foreign caller can hit it, these tests can hit it too — and
//! if they leak, so does the caller.

use std::ffi::{CStr, CString};
use std::ptr;

use serde_json::{json, Value};

use gpp_ffi::{gpp_call, gpp_session_free, gpp_session_new, gpp_string_free, GppSession, METHODS};

// ------------------------------------------------------------------ harness

/// A session held the way C holds one, freed on drop so no test can leak it.
struct ForeignCaller(*mut GppSession);

impl ForeignCaller {
    fn new() -> Self {
        let ptr = gpp_session_new();
        assert!(!ptr.is_null(), "gpp_session_new returned NULL");
        Self(ptr)
    }

    /// Open a library on a fresh temporary directory and return both, because
    /// dropping the directory would take the catalog with it.
    fn with_library() -> (Self, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let caller = Self::new();
        let reply = caller.call("open_library", &json!({ "path": path_of(&dir) }).to_string());
        assert!(reply.get("ok").is_some(), "could not open a library: {reply}");
        (caller, dir)
    }

    fn call(&self, method: &str, args: &str) -> Value {
        raw_call(self.0, Some(method.as_bytes()), Some(args.as_bytes()))
    }

    /// The reply's payload, insisting it was a success.
    fn ok(&self, method: &str, args: &str) -> Value {
        let reply = self.call(method, args);
        match reply.get("ok") {
            Some(v) => v.clone(),
            None => panic!("{method} failed: {reply}"),
        }
    }
}

impl Drop for ForeignCaller {
    fn drop(&mut self) {
        unsafe { gpp_session_free(self.0) }
    }
}

/// One trip across the boundary, with the pointers a C caller would pass —
/// including `NULL`, and including bytes that are not UTF-8.
fn raw_call(session: *mut GppSession, method: Option<&[u8]>, args: Option<&[u8]>) -> Value {
    let method = method.map(|b| CString::new(b).unwrap());
    let args = args.map(|b| CString::new(b).unwrap());
    unsafe {
        let reply = gpp_call(
            session,
            method.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            args.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
        );
        assert!(!reply.is_null(), "gpp_call promises a non-NULL reply");
        let text = CStr::from_ptr(reply)
            .to_str()
            .expect("gpp_call promises valid UTF-8")
            .to_owned();
        gpp_string_free(reply);
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("reply was not JSON ({e}): {text}"))
    }
}

/// The stable error tag, insisting the reply was a failure.
fn error_kind(reply: &Value) -> String {
    assert!(
        reply.get("ok").is_none(),
        "expected a failure, got {reply}"
    );
    assert!(
        reply.get("error").and_then(Value::as_str).is_some(),
        "an error envelope must carry a message: {reply}"
    );
    reply
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("an error envelope must carry a kind: {reply}"))
        .to_owned()
}

fn path_of(dir: &tempfile::TempDir) -> String {
    dir.path().display().to_string()
}

fn write_jpeg(path: &std::path::Path, w: u32, h: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

// ------------------------------------------------------------ the happy path

#[test]
fn a_library_can_be_opened_and_reports_its_status() {
    let (c, dir) = ForeignCaller::with_library();

    let status = c.ok("status", "{}");
    assert_eq!(status["photo_count"], json!(0));
    assert_eq!(status["root"], json!(path_of(&dir)));
    assert_eq!(c.ok("is_open", "{}"), json!(true));
}

#[test]
fn a_method_that_takes_no_arguments_accepts_a_null_args_pointer() {
    let (c, _dir) = ForeignCaller::with_library();

    // A C caller with nothing to say should not have to spell `"{}"`.
    let reply = raw_call(c.0, Some(b"is_open"), None);
    assert_eq!(reply["ok"], json!(true));
}

#[test]
fn photos_are_listed_and_narrowed_by_a_filter() {
    let (c, dir) = ForeignCaller::with_library();
    write_jpeg(&dir.path().join("one.jpg"), 60, 40);
    write_jpeg(&dir.path().join("two.jpg"), 60, 40);
    assert_eq!(c.ok("import", "{}")["imported"], json!(2));

    let all = c.ok("photos", "{}");
    assert_eq!(all.as_array().unwrap().len(), 2);

    let first = all[0]["id"].as_i64().unwrap();
    c.ok("set_rating", &json!({ "ids": [first], "rating": 5 }).to_string());
    c.ok("set_flag", &json!({ "ids": [first], "flag": "pick" }).to_string());

    // A partial filter is the normal case: name the one thing you care about.
    let starred = c.ok("photos", r#"{"filter":{"minRating":4}}"#);
    let starred = starred.as_array().unwrap();
    assert_eq!(starred.len(), 1, "the filter must reach the query");
    assert_eq!(starred[0]["id"], json!(first));
    assert_eq!(starred[0]["flag"], json!("pick"));
}

#[test]
fn an_album_can_be_created_filled_and_read_back() {
    let (c, dir) = ForeignCaller::with_library();
    write_jpeg(&dir.path().join("one.jpg"), 60, 40);
    c.ok("import", "{}");
    let id = c.ok("photos", "{}")[0]["id"].as_i64().unwrap();

    let album = c.ok(
        "create_album",
        &json!({"album": {"path": "2026/test", "title": "Test"}}).to_string(),
    );
    assert_eq!(album["path"], json!("2026/test"));

    let added = c.ok(
        "add_to_album",
        &json!({"path": "2026/test", "photo_ids": [id]}).to_string(),
    );
    assert_eq!(added, json!(1));

    // The album, plus the `2026` folder created so the path stays reachable.
    let albums = c.ok("albums", "{}");
    let albums = albums.as_array().unwrap();
    assert_eq!(albums.len(), 2);
    let test = albums.iter().find(|a| a["path"] == json!("2026/test")).unwrap();
    assert_eq!(test["photo_count"], json!(1));
    assert_eq!(test["is_locked"], json!(false));

    let link = c.ok("generate_share_link", r#"{"path":"2026/test"}"#);
    assert!(!link.as_str().unwrap().is_empty());
}

#[test]
fn a_develop_adjustment_is_recorded_and_can_be_reset() {
    let (c, dir) = ForeignCaller::with_library();
    write_jpeg(&dir.path().join("one.jpg"), 80, 60);
    c.ok("import", "{}");
    let id = c.ok("photos", "{}")[0]["id"].as_i64().unwrap();

    // An untouched photo has an empty stack rather than no stack, so the
    // develop panel has one shape to render.
    assert_eq!(c.ok("photo_edits", &json!({ "id": id }).to_string())["ops"], json!([]));

    let changed = c.ok(
        "set_photo_edit",
        &json!({"ids": [id], "op": {"op": "exposure", "ev": 0.5}}).to_string(),
    );
    assert_eq!(changed, json!(1));
    let stack = c.ok("photo_edits", &json!({ "id": id }).to_string());
    assert_eq!(stack["ops"], json!([{"op": "exposure", "ev": 0.5}]));

    c.ok("clear_photo_edit", &json!({"ids": [id], "kind": "exposure"}).to_string());
    assert_eq!(c.ok("photo_edits", &json!({ "id": id }).to_string())["ops"], json!([]));

    c.ok(
        "set_photo_edit",
        &json!({"ids": [id], "op": {"op": "black-and-white"}}).to_string(),
    );
    c.ok("reset_photo_edits", &json!({ "ids": [id] }).to_string());
    assert_eq!(c.ok("photo_edits", &json!({ "id": id }).to_string())["ops"], json!([]));
}

#[test]
fn publishing_writes_the_gallery_tree() {
    let (c, dir) = ForeignCaller::with_library();
    let dest = tempfile::tempdir().unwrap();
    write_jpeg(&dir.path().join("one.jpg"), 60, 40);
    c.ok("import", "{}");
    let id = c.ok("photos", "{}")[0]["id"].as_i64().unwrap();
    c.ok("create_album", r#"{"album":{"path":"2026/test","title":"Test"}}"#);
    c.ok(
        "add_to_album",
        &json!({"path": "2026/test", "photo_ids": [id]}).to_string(),
    );

    c.ok(
        "set_publish_target",
        &json!({"target": {"dest": path_of(&dest), "min_rating": null}}).to_string(),
    );
    assert_eq!(
        c.ok("publish_target", "{}")["dest"],
        json!(path_of(&dest))
    );

    let results = c.ok("publish", r#"{"album_path":"2026/test"}"#);
    assert_eq!(results[0]["photos_copied"], json!(1));
    assert!(dest.path().join("2026/test/index.md").exists());
}

#[test]
fn sync_subscriptions_are_readable_and_writable_across_the_boundary() {
    let (c, _dir) = ForeignCaller::with_library();
    let remote = tempfile::tempdir().unwrap();
    c.ok("set_remote_dir", &json!({ "dir": path_of(&remote) }).to_string());
    assert_eq!(c.ok("remote_dir", "{}"), json!(path_of(&remote)));

    // The token is settable but never readable back — only whether one exists.
    assert_eq!(c.ok("has_remote_token", "{}"), json!(false));
    c.ok("set_remote_token", r#"{"token":"s3cret"}"#);
    assert_eq!(c.ok("has_remote_token", "{}"), json!(true));
    assert_eq!(error_kind(&c.call("remote_token", "{}")), "unknown-method");

    c.ok("track_album", r#"{"album_path":"2026/x","direction":"pull"}"#);
    let subs = c.ok("album_subscriptions", "{}");
    assert_eq!(subs.as_array().unwrap().len(), 1);
    assert_eq!(subs[0]["direction"], json!("pull"));

    c.ok("untrack_album", r#"{"album_path":"2026/x"}"#);
    assert_eq!(c.ok("album_subscriptions", "{}"), json!([]));
}

#[test]
fn an_album_pushed_by_one_session_is_pulled_by_another() {
    let remote = tempfile::tempdir().unwrap();

    // Machine A authors and contributes.
    let (a, a_dir) = ForeignCaller::with_library();
    let a_pub = tempfile::tempdir().unwrap();
    a.ok("set_remote_dir", &json!({ "dir": path_of(&remote) }).to_string());
    a.ok(
        "set_publish_target",
        &json!({"target": {"dest": path_of(&a_pub)}}).to_string(),
    );
    write_jpeg(&a_dir.path().join("2026/x/one.jpg"), 90, 60);
    a.ok("import", "{}");
    a.ok("create_album", r#"{"album":{"path":"2026/x","title":"Ex"}}"#);
    let id = a.ok("photos", "{}")[0]["id"].as_i64().unwrap();
    a.ok("add_to_album", &json!({"path": "2026/x", "photo_ids": [id]}).to_string());
    let pushed = a.ok("push_album", r#"{"album_path":"2026/x"}"#);
    assert!(pushed["files_pushed"].as_u64().unwrap() >= 2);

    // Machine B discovers and adopts.
    let (b, _b_dir) = ForeignCaller::with_library();
    let b_pub = tempfile::tempdir().unwrap();
    b.ok("set_remote_dir", &json!({ "dir": path_of(&remote) }).to_string());
    b.ok(
        "set_publish_target",
        &json!({"target": {"dest": path_of(&b_pub)}}).to_string(),
    );
    let found = b.ok("remote_albums", "{}");
    let x = found
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["path"] == json!("2026/x"))
        .unwrap();
    assert_eq!(x["local"], json!(false));
    assert_eq!(x["title"], json!("Ex"));

    let pulled = b.ok("pull_album", r#"{"album_path":"2026/x"}"#);
    assert!(pulled["files_pulled"].as_u64().unwrap() >= 2);
    assert_eq!(
        b.ok("album_photos", r#"{"path":"2026/x"}"#).as_array().unwrap().len(),
        1
    );
}

#[test]
fn one_session_serves_many_threads_at_once() {
    let (c, dir) = ForeignCaller::with_library();
    write_jpeg(&dir.path().join("one.jpg"), 60, 40);
    c.ok("import", "{}");

    // The header promises this; a data race here would be the promise being
    // wrong rather than the test being flaky.
    struct Shared(*mut GppSession);
    unsafe impl Send for Shared {}
    unsafe impl Sync for Shared {}
    let shared = Shared(c.0);

    std::thread::scope(|scope| {
        for _ in 0..8 {
            let shared = &shared;
            scope.spawn(move || {
                for _ in 0..20 {
                    let reply = raw_call(shared.0, Some(b"photos"), Some(b"{}"));
                    assert_eq!(reply["ok"].as_array().unwrap().len(), 1);
                }
            });
        }
    });
}

// ------------------------------------------------------------ the ugly path

#[test]
fn a_null_session_pointer_is_an_error_not_a_crash() {
    let reply = raw_call(ptr::null_mut(), Some(b"status"), Some(b"{}"));
    assert_eq!(error_kind(&reply), "null-pointer");
}

#[test]
fn a_null_method_pointer_is_an_error_not_a_crash() {
    let c = ForeignCaller::new();
    let reply = raw_call(c.0, None, Some(b"{}"));
    assert_eq!(error_kind(&reply), "null-pointer");
}

#[test]
fn input_that_is_not_utf8_is_an_error_not_a_crash() {
    let c = ForeignCaller::new();

    // A lone 0xFF byte cannot begin a UTF-8 sequence.
    assert_eq!(
        error_kind(&raw_call(c.0, Some(&[0xff, b'x']), Some(b"{}"))),
        "invalid-utf8"
    );
    assert_eq!(
        error_kind(&raw_call(c.0, Some(b"is_open"), Some(&[0xff, b'x']))),
        "invalid-utf8"
    );
}

#[test]
fn an_unknown_method_name_is_an_error_that_names_it() {
    let c = ForeignCaller::new();
    let reply = c.call("set_ratings", "{}");
    assert_eq!(error_kind(&reply), "unknown-method");
    assert!(
        reply["error"].as_str().unwrap().contains("set_ratings"),
        "the message should name what was asked for: {reply}"
    );
}

#[test]
fn malformed_json_is_reported_rather_than_parsed_loosely() {
    let c = ForeignCaller::new();
    assert_eq!(error_kind(&c.call("open_library", "{not json")), "bad-arguments");
    // A JSON array is well-formed but is not an argument object.
    assert_eq!(error_kind(&c.call("open_library", "[1,2]")), "bad-arguments");
    // A required argument that is missing is the same class of mistake.
    assert_eq!(error_kind(&c.call("open_library", "{}")), "bad-arguments");
}

#[test]
fn a_misspelled_argument_is_refused_rather_than_silently_ignored() {
    let (c, _dir) = ForeignCaller::with_library();

    // `min_rating` instead of `minRating` used to mean "no filter at all",
    // which looks exactly like a filter that matched everything.
    let reply = c.call("photos", r#"{"filter":{"min_rating":4}}"#);
    assert_eq!(error_kind(&reply), "bad-arguments");
    assert!(reply["error"].as_str().unwrap().contains("unknown field"));

    // And a stray key on the call itself, not just on a nested object.
    assert_eq!(error_kind(&c.call("photos", r#"{"filtre":{}}"#)), "bad-arguments");
}

#[test]
fn a_failure_inside_the_core_comes_back_as_an_error_envelope() {
    // No library open: the core refuses, and the refusal is legible.
    let c = ForeignCaller::new();
    let reply = c.call("status", "{}");
    assert_eq!(error_kind(&reply), "other");
    assert!(reply["error"].as_str().unwrap().contains("no library is open"));

    // With one open, a name that is not in the catalog gets its own kind, so a
    // caller can tell "you asked for the wrong thing" from "the disk failed".
    let (c, _dir) = ForeignCaller::with_library();
    assert_eq!(error_kind(&c.call("photo", r#"{"id":9999}"#)), "photo-not-found");
    assert_eq!(
        error_kind(&c.call("delete_album", r#"{"path":"nope"}"#)),
        "album-not-found"
    );
    // Publishing with nowhere to publish to is refused before anything moves.
    assert_eq!(error_kind(&c.call("publish", "{}")), "other");
}

#[test]
fn freeing_a_null_pointer_is_a_no_op() {
    unsafe {
        gpp_string_free(ptr::null_mut());
        gpp_session_free(ptr::null_mut());
    }
}

#[test]
fn a_session_survives_being_used_after_an_error() {
    let (c, _dir) = ForeignCaller::with_library();
    assert_eq!(error_kind(&c.call("no_such_method", "{}")), "unknown-method");
    assert_eq!(error_kind(&c.call("photo", r#"{"id":1}"#)), "photo-not-found");
    // A rejected call must not have disturbed the session behind it.
    assert_eq!(c.ok("is_open", "{}"), json!(true));
}

// ----------------------------------------------------------- the method list

#[test]
fn every_documented_method_name_resolves_to_something() {
    let c = ForeignCaller::new();
    for method in METHODS {
        let reply = c.call(method, "{}");
        // Most of these fail for want of a library or of arguments — that is
        // fine. What must not happen is a listed name that nothing answers to.
        assert_ne!(
            reply.get("kind").and_then(Value::as_str),
            Some("unknown-method"),
            "{method} is listed in METHODS but nothing dispatches it"
        );
    }
}

#[test]
fn the_method_list_has_no_duplicates() {
    let mut seen = std::collections::BTreeSet::new();
    for method in METHODS {
        assert!(seen.insert(*method), "{method} is listed twice");
    }
}
