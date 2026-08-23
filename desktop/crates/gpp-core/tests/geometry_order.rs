//! What the rotate and flip buttons do, checked against what is on screen.
//!
//! Geometry does not commute, and the edit stack is an order, not a set — so
//! the buttons can only be trusted if pressing one moves the photograph the way
//! the icon says *relative to the frame the photographer is looking at*. That
//! is not automatic: the rotate op keeps its place in the stack, and a mirror
//! applied over it reverses which way the frame appears to turn.

use gpp_core::develop::{apply, EditOp, EditStack};
use image::DynamicImage;

/// A gradient, so any turn or mirror shows up in the pixels rather than being
/// hidden by symmetry. Non-square, so a quarter turn also changes the shape.
fn frame() -> DynamicImage {
    let mut b = image::RgbImage::new(64, 40);
    for (x, y, p) in b.enumerate_pixels_mut() {
        *p = image::Rgb([(x * 4) as u8, (y * 6) as u8, 128]);
    }
    DynamicImage::ImageRgb8(b)
}

fn same(a: &DynamicImage, b: &DynamicImage) -> bool {
    a.width() == b.width()
        && a.height() == b.height()
        && a.to_rgb8().as_raw() == b.to_rgb8().as_raw()
}

fn press(stack: &mut EditStack, button: &str) {
    match button {
        "rotate-right" => stack.rotate_by(1),
        "rotate-left" => stack.rotate_by(-1),
        "flip-h" => stack.toggle(EditOp::FlipHorizontal),
        "flip-v" => stack.toggle(EditOp::FlipVertical),
        other => panic!("no such button: {other}"),
    }
}

/// Press `earlier`, look at the result, then press one more rotate — and hold
/// that last press to turning exactly what was on screen.
fn rotate_turns_the_visible_frame(earlier: &[&str], last: &str) -> bool {
    let base = frame();
    let mut stack = EditStack::default();
    for button in earlier {
        press(&mut stack, button);
    }
    let on_screen = apply(&base, &stack);

    press(&mut stack, last);
    let got = apply(&base, &stack);

    let mut just_the_turn = EditStack::default();
    press(&mut just_the_turn, last);
    same(&got, &apply(&on_screen, &just_the_turn))
}

#[test]
fn a_rotate_always_turns_the_frame_on_screen() {
    // The middle four are the ones that were wrong: a flip sits over the rotate
    // in the stack, so adding a quarter turn moved the photograph backwards and
    // the button did the opposite of its icon.
    for (earlier, last) in [
        (vec![], "rotate-right"),
        (vec!["rotate-right"], "rotate-right"),
        (vec!["rotate-right", "flip-h"], "rotate-right"),
        (vec!["rotate-right", "flip-h"], "rotate-left"),
        (vec!["rotate-right", "flip-v"], "rotate-right"),
        (vec!["rotate-right", "flip-h", "flip-v"], "rotate-right"),
        (vec!["flip-h"], "rotate-right"),
        (vec!["flip-h", "rotate-right"], "rotate-right"),
        (vec!["flip-h", "flip-v"], "rotate-right"),
    ] {
        assert!(
            rotate_turns_the_visible_frame(&earlier, last),
            "after {earlier:?}, pressing {last} did not turn what was on screen"
        );
    }
}

/// Two flips are a 180° turn, not a mirror, so they must not reverse anything.
#[test]
fn two_flips_do_not_reverse_a_turn() {
    let mut both = EditStack::default();
    both.toggle(EditOp::FlipHorizontal);
    both.toggle(EditOp::FlipVertical);
    both.rotate_by(1);
    both.rotate_by(1);

    match both.get("rotate") {
        Some(EditOp::Rotate { quarter_turns }) => assert_eq!(
            *quarter_turns, 2,
            "two right turns under two flips must still be a half turn"
        ),
        other => panic!("expected a rotate op, got {other:?}"),
    }
}

/// Tone is a per-pixel function and runs in its own pass, so where a tone op
/// sits relative to a geometry op cannot change the result. Worth pinning: it
/// is what lets the panel record adjustments in whatever order they are made.
#[test]
fn geometry_and_tone_do_not_interfere() {
    let base = frame();

    let geometry_first = EditStack {
        ops: vec![
            EditOp::Rotate { quarter_turns: 1 },
            EditOp::Exposure { ev: 1.0 },
        ],
        ..Default::default()
    };

    let tone_first = EditStack {
        ops: vec![
            EditOp::Exposure { ev: 1.0 },
            EditOp::Rotate { quarter_turns: 1 },
        ],
        ..Default::default()
    };

    assert!(same(&apply(&base, &geometry_first), &apply(&base, &tone_first)));
}
