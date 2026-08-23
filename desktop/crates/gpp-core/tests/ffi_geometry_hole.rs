//! Orientation reached through the generic edit setter — the road the C ABI
//! gives a foreign client.
//!
//! `set_photo_edit` takes any `EditOp`, geometry included, and it is the only
//! way a non-Rust caller could turn a photograph: `rotate_photos` and
//! `toggle_photo_edit` are not in the dispatch table. Left going straight to
//! `EditStack::set`, it edited the op where it lay — the model that turned a
//! mirrored frame the wrong way — so the same three instructions gave the
//! buttons a mirror and a foreign client a half turn and a mirror.

use gpp_core::develop::{EditOp, EditStack};

/// Canonical: at most one mirror, at most one turn, mirror first, crop last,
/// and never a stored `flip-vertical`.
fn assert_canonical(stack: &EditStack, what: &str) {
    let kinds: Vec<&str> = stack.ops.iter().map(|o| o.kind()).collect();
    assert!(
        !kinds.contains(&"flip-vertical"),
        "{what}: a vertical mirror was stored rather than folded — {kinds:?}"
    );
    assert!(
        kinds.iter().filter(|k| **k == "flip-horizontal").count() <= 1
            && kinds.iter().filter(|k| **k == "rotate").count() <= 1,
        "{what}: geometry stacked up instead of folding — {kinds:?}"
    );
    let geometry: Vec<&str> = kinds
        .iter()
        .copied()
        .filter(|k| *k == "flip-horizontal" || *k == "rotate")
        .collect();
    if geometry.len() == 2 {
        assert_eq!(
            (geometry[0], geometry[1]),
            ("flip-horizontal", "rotate"),
            "{what}: mirror must come before the turn — {kinds:?}"
        );
    }
    // Only among the geometry: tone runs in its own per-pixel pass, so where a
    // tone op sits relative to the crop cannot change a thing.
    if let Some(at) = kinds.iter().position(|k| *k == "crop") {
        let geometry_after = kinds[at + 1..]
            .iter()
            .any(|k| matches!(*k, "flip-horizontal" | "flip-vertical" | "rotate"));
        assert!(
            !geometry_after,
            "{what}: a turn or mirror sits under the crop, so the frame moves beneath it — {kinds:?}"
        );
    }
}

#[test]
fn geometry_sent_absolutely_still_lands_canonical() {
    let sends: Vec<EditOp> = vec![
        EditOp::Rotate { quarter_turns: 1 },
        EditOp::FlipHorizontal,
        EditOp::Rotate { quarter_turns: 2 },
        EditOp::FlipVertical,
        EditOp::Crop { x: 0.1, y: 0.2, w: 0.5, h: 0.5 },
        EditOp::Rotate { quarter_turns: 3 },
        EditOp::FlipHorizontal,
        EditOp::Rotate { quarter_turns: 0 },
        EditOp::Exposure { ev: 0.5 },
        EditOp::FlipVertical,
    ];

    let mut stack = EditStack::new();
    for (i, op) in sends.iter().enumerate() {
        stack.set(op.clone());
        assert_canonical(&stack, &format!("after send {i} ({})", op.kind()));
    }
}

/// An absolute turn means what it says: ask for two quarters and the
/// photograph sits at two quarters, mirror or no mirror.
#[test]
fn an_absolute_turn_is_the_angle_it_names() {
    for mirrored in [false, true] {
        for turns in 0..4u8 {
            let mut stack = EditStack::new();
            if mirrored {
                stack.toggle(EditOp::FlipHorizontal);
            }
            stack.set(EditOp::Rotate { quarter_turns: turns });

            let stored = stack.ops.iter().find_map(|o| match o {
                EditOp::Rotate { quarter_turns } => Some(*quarter_turns),
                _ => None,
            });
            assert_eq!(
                stored.unwrap_or(0),
                turns,
                "mirrored={mirrored}: asked for {turns} quarter turns"
            );
            assert_canonical(&stack, "absolute turn");
        }
    }
}
