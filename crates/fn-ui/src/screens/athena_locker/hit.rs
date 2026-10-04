//! UMG hit testing over the arranged widget tree (what `SWidget::OnMouseButtonDown` routing sees): visibility rules of `ESlateVisibility`, only the active
//! child of a widget switcher, later siblings above earlier ones, list views clip their entries, disabled buttons are not interactive.
use crate::layout::Rect;
use crate::screens::athena_lobby::umg::{Ui, Vis};

/// Widgets that clip the hit testing of their subtree to their own rectangle (Slate clipping zones: list views, blurs).
fn clips(class: &str) -> bool { matches!(class, "FortItemTileView" | "BackgroundBlur") }

/// Whether a widget that is `Visible` takes part in hit testing itself. UMG panels and text widgets construct with `SelfHitTestInvisible` (or `HitTestInvisible`) and Blueprint
/// user widgets that are not buttons do not capture the pointer, so only an explicitly serialized `Visibility` (or a widget that handles the pointer: buttons, images,
/// borders that draw) makes a widget a hit target. (Class defaults of the `UWidget` constructors of 4.16.)
pub fn self_hit_testable(ui: &Ui, id: usize) -> bool {
    let w = &ui.w[id];
    if w.vis != Vis::Visible { return false; }
    if w.props.get("Visibility").is_some() { return true; }
    if w.user.is_some() { return ui.is_common_button(id); }
    match w.class.as_str() {
        "CanvasPanel" | "Overlay" | "VerticalBox" | "HorizontalBox" | "GridPanel" | "UniformGridPanel" | "WrapBox" | "SizeBox" | "ScaleBox" | "SafeZone" | "WidgetSwitcher" | "CommonWidgetSwitcher"
        | "CommonWidgetStack" | "NamedSlot" | "Spacer" | "TextBlock" | "CommonTextBlock" | "CommonNumericTextBlock" | "RichTextBlock" | "BackgroundBlur" | "CommonLoadGuard"
        | "CommonVisibilityWidget" | "MenuAnchor" | "CommonActionWidget" | "FortItemCountTextBlock" | "FortMultiSizeItemCard" | "FortItemTileView" => false,
        _ => true,
    }
}

fn walk(ui: &Ui, id: usize, rects: &[Rect], x: f32, y: f32, out: &mut Vec<usize>) {
    let w = &ui.w[id];
    // Collapsed / Hidden widgets and everything below them are not hit-testable; `HitTestInvisible` also hides the children
    if matches!(w.vis, Vis::Collapsed | Vis::Hidden | Vis::HitTestInvisible) { return; }
    let Some(r) = rects.get(id).copied() else { return };
    if clips(&w.class) && !r.contains(x, y) { return; }
    let kids: Vec<usize> = match w.class.as_str() {
        "WidgetSwitcher" | "CommonWidgetSwitcher" => {
            let idx = w.props.get("ActiveWidgetIndex").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
            w.kids.get(idx).copied().into_iter().collect()
        }
        _ => w.kids.clone(),
    };
    // topmost first: later children are drawn above earlier ones
    for &k in kids.iter().rev() { walk(ui, k, rects, x, y, out); }
    if self_hit_testable(ui, id) && r.contains(x, y) && r.w > 0.0 && r.h > 0.0 { out.push(id); }
}

/// Every widget under the point, topmost first. `rects` = pixel rectangle of each widget by id (`Frame::id_rects`).
pub fn hit_stack(ui: &Ui, root: usize, rects: &[Rect], x: f32, y: f32) -> Vec<usize> {
    let mut out = vec![];
    walk(ui, root, rects, x, y, &mut out);
    out
}

/// A widget that takes pointer input: `UCommonButton` / `UCommonRotator` (the Blueprint buttons of the screens are subclasses).
pub fn is_interactive(ui: &Ui, id: usize) -> bool { ui.is_common_button(id) }

/// A widget and all its ancestors are enabled (`UWidget::bIsEnabled`; Slate's `ShouldBeEnabled`).
pub fn effectively_enabled(ui: &Ui, mut id: usize) -> bool {
    loop {
        if !ui.w[id].enabled { return false; }
        match ui.w[id].parent { Some(p) => id = p, None => return true }
    }
}

/// The interactive widget the pointer is over: the nearest interactive ancestor-or-self of the topmost hit widget (a disabled button blocks the pointer
/// but is not returned).
pub fn button_under(ui: &Ui, root: usize, rects: &[Rect], x: f32, y: f32) -> Option<usize> {
    for id in hit_stack(ui, root, rects, x, y) {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if is_interactive(ui, c) { return if effectively_enabled(ui, c) { Some(c) } else { None }; }
            cur = ui.w[c].parent;
        }
    }
    None
}

/// `id` is `anc` or lies below it.
pub fn within(ui: &Ui, id: usize, anc: usize) -> bool { ui.is_descendant(id, anc) }
