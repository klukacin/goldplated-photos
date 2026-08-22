//! The header is hand-written, so something has to notice when it drifts.
//!
//! cbindgen would have generated it, but cbindgen is MPL-2.0 and this project
//! keeps every Rust dependency — build-time ones included — permissive. The
//! trade is that `include/gpp_ffi.h` is maintained by hand; this test is the
//! other half of that trade. It reads the header and the source side by side
//! and fails if they stop agreeing about what is exported.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read {rel}: {e}"))
}

/// Every symbol the header promises, with the C spelling it promises it in.
const DECLARATIONS: &[(&str, &str)] = &[
    ("gpp_session_new", "GppSession *gpp_session_new(void);"),
    (
        "gpp_call",
        "char *gpp_call(GppSession *session, const char *method, const char *args_json);",
    ),
    ("gpp_string_free", "void gpp_string_free(char *ptr);"),
    ("gpp_session_free", "void gpp_session_free(GppSession *ptr);"),
];

#[test]
fn the_header_declares_exactly_what_the_library_exports() {
    let header = read("include/gpp_ffi.h");
    let source = read("src/lib.rs");

    for (symbol, declaration) in DECLARATIONS {
        assert!(
            header.contains(declaration),
            "the header should declare `{declaration}`"
        );
        assert!(
            source.contains(&format!("fn {symbol}(")),
            "`{symbol}` is declared in the header but not defined in src/lib.rs"
        );
    }

    // And nothing beyond those four leaves the library under a stable name: an
    // extra `#[no_mangle]` that never reached the header is exactly the drift
    // this test exists to catch.
    let exported = source.matches("#[no_mangle]").count();
    assert_eq!(
        exported,
        DECLARATIONS.len(),
        "src/lib.rs exports {exported} symbols but the header declares {}",
        DECLARATIONS.len()
    );
}

#[test]
fn the_header_is_safe_to_include_more_than_once_and_from_cplusplus() {
    let header = read("include/gpp_ffi.h");
    assert!(header.contains("#ifndef GPP_FFI_H"), "missing include guard");
    assert!(header.contains("#define GPP_FFI_H"), "missing include guard");
    assert!(
        header.contains(r#"extern "C" {"#),
        "a C++ caller needs the linkage block"
    );
}

/// The struct is opaque on purpose: publishing its layout would make every
/// field change an ABI break for callers that never look at one.
#[test]
fn the_session_type_stays_opaque_in_the_header() {
    let header = read("include/gpp_ffi.h");
    assert!(header.contains("typedef struct GppSession GppSession;"));
    assert!(
        !header.contains("struct GppSession {"),
        "the header must not define the struct's contents"
    );
}
