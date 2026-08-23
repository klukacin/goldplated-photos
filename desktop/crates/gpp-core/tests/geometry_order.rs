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

/// Two flips are a half turn, so two of them plus two right turns is a full
/// circle and the photograph is back exactly as it was shot — and, because an
/// untouched photo must keep its original render key and its existing
/// thumbnails, the stack has to be empty rather than merely equivalent.
#[test]
fn a_full_circle_leaves_no_trace() {
    let base = frame();
    let mut stack = EditStack::default();
    for button in ["flip-h", "flip-v", "rotate-right", "rotate-right"] {
        press(&mut stack, button);
    }
    assert!(same(&apply(&base, &stack), &base), "the photograph moved");
    assert!(stack.is_empty(), "left {:?} behind, so the render key changed", stack.ops);
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

/// Every button, pressed after every reachable framing, must move the
/// photograph exactly the way its icon says — with a crop drawn on it and
/// without.
///
/// Turns and mirrors do not commute, so this cannot be assumed from the code
/// reading sensibly. Two defects lived here: pressing "rotate right" on a
/// flipped frame turned the photograph left, and pressing a flip a second time
/// to undo it mirrored the wrong axis once a turn sat between them. Both were
/// three presses away in the shipping panel.
#[test]
fn every_button_moves_the_photograph_the_way_its_icon_says() {
    let base = frame();
    let buttons = ["rotate-right", "rotate-left", "flip-h", "flip-v"];

    // Every framing reachable in three presses, which covers all eight ways a
    // rectangle can be set down, by more than one route to each.
    let mut histories: Vec<Vec<&str>> = vec![vec![]];
    for _ in 0..3 {
        let mut next = Vec::new();
        for h in &histories {
            for b in buttons {
                let mut longer = h.clone();
                longer.push(b);
                next.push(longer);
            }
        }
        histories.extend(next);
    }

    for history in &histories {
        for cropped in [false, true] {
            for last in buttons {
                let mut stack = EditStack::default();
                for button in history {
                    press(&mut stack, button);
                }
                if cropped {
                    // Off-centre and not square: a centred rectangle is
                    // symmetric enough to survive a wrong transform and report
                    // a pass it did not earn.
                    stack.set(EditOp::Crop { x: 0.125, y: 0.25, w: 0.5, h: 0.5 });
                }
                let on_screen = apply(&base, &stack);

                press(&mut stack, last);
                let got = apply(&base, &stack);

                let mut only_the_press = EditStack::default();
                press(&mut only_the_press, last);
                assert!(
                    same(&got, &apply(&on_screen, &only_the_press)),
                    "after {history:?}{}, pressing {last} did not do what it says",
                    if cropped { " + crop" } else { "" }
                );
            }
        }
    }
}
