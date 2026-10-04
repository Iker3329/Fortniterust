//! Widget `RenderTransform` for already emitted quads (glyph runs): Slate applies `Scale / Shear / Angle` about the widget's `RenderTransformPivot` (default
//! 0.5, 0.5 = the widget's own centre) and then the `Translation`. Used by the "Out of Ammo!" text of `AthenaEquippedItemWidget` (`Angle -5, Translation (0, 24)`).
use crate::draw::{Cmd, RenderList};

/// Rotate every quad emitted at `start..` by `angle_deg` (UMG convention: positive = clockwise on screen, y down) about `centre`.
pub fn rotate_quads_since(out: &mut RenderList, start: usize, centre: (f32, f32), angle_deg: f32) {
    if angle_deg == 0. { return; }
    let (s, c) = angle_deg.to_radians().sin_cos();
    for cmd in out.cmds.iter_mut().skip(start) {
        if let Cmd::Quad(q) = cmd {
            for p in q.pos.iter_mut() {
                let (dx, dy) = (p[0] - centre.0, p[1] - centre.1);
                *p = [centre.0 + dx * c - dy * s, centre.1 + dx * s + dy * c];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::TextureId;
    use crate::layout::Rect;
    #[test] fn negative_angle_raises_the_right_end() {
        let mut l = RenderList::new();
        l.quad(TextureId(3), Rect::new(90., 100., 20., 4.), [0., 0., 1., 1.], [1.; 4]); // a glyph right of the centre (100, 100)
        l.quad(TextureId(3), Rect::new(-10., 100., 20., 4.), [0., 0., 1., 1.], [1.; 4]); // left of it... centre at x 0
        rotate_quads_since(&mut l, 0, (0., 100.), -5.);
        let Cmd::Quad(a) = &l.cmds[0] else { panic!() }; let Cmd::Quad(b) = &l.cmds[1] else { panic!() };
        assert!(a.pos[0][1] < 100. - 7., "far right of the pivot moves up: {:?}", a.pos[0]);
        assert!((b.pos[0][0] - -10.).abs() < 0.1 && b.pos[0][1] > 99. && b.pos[0][1] < 102., "near the pivot barely moves: {:?}", b.pos[0]);
    }
    #[test] fn only_quads_from_start_are_rotated_and_zero_is_a_no_op() {
        let mut l = RenderList::new();
        l.quad(TextureId(3), Rect::new(10., 10., 5., 5.), [0., 0., 1., 1.], [1.; 4]);
        l.quad(TextureId(3), Rect::new(10., 10., 5., 5.), [0., 0., 1., 1.], [1.; 4]);
        let before = l.cmds.clone();
        rotate_quads_since(&mut l, 1, (0., 0.), 0.); assert_eq!(l.cmds, before);
        rotate_quads_since(&mut l, 1, (0., 0.), 90.);
        assert_eq!(l.cmds[0], before[0]); assert_ne!(l.cmds[1], before[1]);
        let Cmd::Quad(q) = &l.cmds[1] else { panic!() }; assert!((q.pos[0][0] - -10.).abs() < 1e-4 && (q.pos[0][1] - 10.).abs() < 1e-4, "(10,10) clockwise 90 deg -> (-10,10): {:?}", q.pos[0]);
    }
}
