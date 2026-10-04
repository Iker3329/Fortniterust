//! Slate brushes, text styles, border styles and button styles as the cooked data defines them (`FSlateBrush`, `UCommonTextStyle`,
//! `UCommonBorderStyle`, `UCommonButtonStyle` Blueprint class defaults).
use super::store::{jenum, jf, jmargin, jvec, Store};
use crate::text::FontId;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawAs { NoDraw, Box, Border, Image, RoundedBox }

/// `FSlateBrush` (linear tint; `margin` = fractions of `image_size`, as Slate reads it for Box brushes).
#[derive(Clone, Debug)]
pub struct Brush {
    pub draw: DrawAs,
    pub image_size: [f32; 2],
    pub margin: [f32; 4],
    pub tint: [f32; 4],
    /// `/Game/...Name.Name` of the texture or material (None = no resource: a plain tinted rectangle).
    pub resource: Option<String>,
    pub flip_x: bool,
    pub uv_region: Option<[f32; 4]>,
}
impl Brush {
    pub fn from_json(v: &Value) -> Option<Brush> {
        let o = v.as_object()?;
        let draw = match jenum(o.get("DrawAs")) {
            Some("NoDrawType") => DrawAs::NoDraw,
            Some("Box") => DrawAs::Box,
            Some("Border") => DrawAs::Border,
            Some("RoundedBox") => DrawAs::RoundedBox,
            Some("Image") => DrawAs::Image,
            // `DrawAs` is only serialized when it differs from the default of `FSlateBrush` (Image)
            _ => DrawAs::Image,
        };
        let tint = o.get("TintColor").and_then(|t| jvec::<4>(t.get("SpecifiedColor"))).unwrap_or([1.0; 4]);
        let resource = o.get("ResourceObject").and_then(|r| r.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_owned());
        let uv_region = o.get("UVRegion").and_then(|r| {
            if r.get("bIsValid").and_then(|b| b.as_bool()).unwrap_or(false) { let mn = jvec::<2>(r.get("Min"))?; let mx = jvec::<2>(r.get("Max"))?; Some([mn[0], mn[1], mx[0], mx[1]]) } else { None }
        });
        Some(Brush {
            draw,
            image_size: jvec::<2>(o.get("ImageSize")).unwrap_or([32.0, 32.0]),
            margin: jmargin(o.get("Margin")),
            tint,
            resource,
            flip_x: matches!(jenum(o.get("Mirroring")), Some("Horizontal") | Some("Both")),
            uv_region,
        })
    }
}

/// Resolved text style (linear colours, Slate units).
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: FontId,
    pub typeface: String,
    pub size: f32,
    pub color: [f32; 4],
    pub shadow_offset: [f32; 2],
    pub shadow_color: [f32; 4],
    pub uses_shadow: bool,
    pub outline: f32,
    pub outline_color: [f32; 4],
}
impl TextStyle {
    pub fn plain() -> TextStyle {
        // UMG `FSlateFontInfo` default / `STextBlock` default colour: Roboto 24 white (a `CommonTextBlock` with no style at all)
        TextStyle { font: FontId::RobotoBold, typeface: "Bold".into(), size: 24.0, color: [1.0; 4], shadow_offset: [0.0; 2], shadow_color: [0.0, 0.0, 0.0, 0.0], uses_shadow: false, outline: 0.0, outline_color: [0.0; 4] }
    }
    /// Overlay a `FSlateFontInfo` JSON object (`Size`, `TypefaceFontName`, `FontObject`).
    pub fn apply_font(&mut self, f: &Value) {
        if let Some(s) = f.get("Size").and_then(jf) { self.size = s; }
        let tf = f.get("TypefaceFontName").and_then(|x| x.as_str());
        let obj = f.get("FontObject").and_then(|x| x.as_str()).map(|s| s.rsplit('.').next().unwrap_or(s));
        if let Some(t) = tf { self.typeface = t.to_owned(); }
        self.font = font_id(obj.unwrap_or(""), &self.typeface);
        if let Some(of) = f.get("OutlineSettings").and_then(|o| o.get("OutlineSize")).and_then(jf) { self.outline = of; }
    }
    pub fn from_defaults(d: &Value) -> TextStyle {
        let mut t = TextStyle::plain();
        if let Some(f) = d.get("Font") { t.apply_font(f); }
        if let Some(c) = jvec::<4>(d.get("Color")) { t.color = c; }
        if let Some(c) = jvec::<4>(d.get("ColorAndOpacity")).filter(|_| d.get("Color").is_none()) { t.color = c; }
        if let Some(c) = jvec::<2>(d.get("ShadowOffset")) { t.shadow_offset = c; }
        if let Some(c) = jvec::<4>(d.get("ShadowColor")) { t.shadow_color = c; }
        t.uses_shadow = d.get("bUsesDropShadow").and_then(|b| b.as_bool()).unwrap_or(t.shadow_offset != [0.0; 2]);
        t
    }
}

/// Font asset + typeface name -> the face the UI font loader has (`FontObject` NotoSans / Burbank, `TypefaceFontName`).
pub fn font_id(font_object: &str, typeface: &str) -> FontId {
    let t = typeface.to_ascii_lowercase();
    match (font_object, t.as_str()) {
        ("Burbank", "burbank-black") => FontId::Title,
        ("Burbank", "burbank-playerpower") => FontId::BurbankBold,
        (_, "notosans-bold") => FontId::Bold,
        (_, "notosans-regular") | (_, "notosans-italic") => FontId::Regular,
        (_, "burbank-black") => FontId::Title,
        (_, "burbank-playerpower") => FontId::BurbankBold,
        ("Roboto", _) => FontId::RobotoBold,
        _ => if t.contains("bold") { FontId::Bold } else if t.contains("black") { FontId::Title } else { FontId::Regular },
    }
}

/// `UCommonBorderStyle`.
#[derive(Clone, Debug)]
pub struct BorderStyle { pub background: Option<Brush> }

/// `UCommonButtonStyle` (the states Slate's `SButton` / `UCommonButton` pick from).
#[derive(Clone, Debug)]
pub struct ButtonStyle {
    pub normal_base: Option<Brush>,
    pub normal_hovered: Option<Brush>,
    pub normal_pressed: Option<Brush>,
    pub selected_base: Option<Brush>,
    pub selected_hovered: Option<Brush>,
    pub selected_pressed: Option<Brush>,
    pub disabled: Option<Brush>,
    pub padding: [f32; 4],
    pub min_width: f32,
    pub min_height: f32,
    pub normal_text: Option<String>,
    pub normal_hovered_text: Option<String>,
    pub selected_text: Option<String>,
    pub selected_hovered_text: Option<String>,
    pub disabled_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtnState { Normal, Hovered, Pressed, Selected, SelectedHovered, SelectedPressed, Disabled }

impl ButtonStyle {
    pub fn from_defaults(d: &Value) -> ButtonStyle {
        let b = |k: &str| d.get(k).and_then(Brush::from_json);
        let cls = |k: &str| d.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_owned());
        ButtonStyle {
            normal_base: b("NormalBase"), normal_hovered: b("NormalHovered"), normal_pressed: b("NormalPressed"),
            selected_base: b("SelectedBase"), selected_hovered: b("SelectedHovered"), selected_pressed: b("SelectedPressed"), disabled: b("Disabled"),
            padding: jmargin(d.get("ButtonPadding")),
            min_width: d.get("MinWidth").and_then(jf).unwrap_or(0.0),
            min_height: d.get("MinHeight").and_then(jf).unwrap_or(0.0),
            normal_text: cls("NormalTextStyle"), normal_hovered_text: cls("NormalHoveredTextStyle"),
            selected_text: cls("SelectedTextStyle"), selected_hovered_text: cls("SelectedHoveredTextStyle"), disabled_text: cls("DisabledTextStyle"),
        }
    }
    pub fn brush(&self, s: BtnState) -> Option<&Brush> {
        match s {
            BtnState::Normal => self.normal_base.as_ref(),
            BtnState::Hovered => self.normal_hovered.as_ref(),
            BtnState::Pressed => self.normal_pressed.as_ref(),
            BtnState::Selected => self.selected_base.as_ref(),
            BtnState::SelectedHovered => self.selected_hovered.as_ref(),
            BtnState::SelectedPressed => self.selected_pressed.as_ref(),
            BtnState::Disabled => self.disabled.as_ref(),
        }
    }
    /// `GetCurrentTextStyleClass()` of the button in `state`.
    pub fn text_style(&self, s: BtnState) -> Option<&str> {
        match s {
            BtnState::Normal | BtnState::Pressed => self.normal_text.as_deref(),
            BtnState::Hovered => self.normal_hovered_text.as_deref().or(self.normal_text.as_deref()),
            BtnState::Selected | BtnState::SelectedPressed => self.selected_text.as_deref().or(self.normal_text.as_deref()),
            BtnState::SelectedHovered => self.selected_hovered_text.as_deref().or(self.selected_text.as_deref()),
            BtnState::Disabled => self.disabled_text.as_deref().or(self.normal_text.as_deref()),
        }
    }
}

/// Style resolver with caches (class path -> resolved style).
#[derive(Default)]
pub struct StyleCache {
    text: std::collections::HashMap<String, TextStyle>,
    border: std::collections::HashMap<String, BorderStyle>,
    button: std::collections::HashMap<String, ButtonStyle>,
}
impl StyleCache {
    pub fn text(&mut self, store: &Store, class_path: &str) -> TextStyle {
        if let Some(t) = self.text.get(class_path) { return t.clone(); }
        let t = TextStyle::from_defaults(&store.class_defaults(class_path));
        self.text.insert(class_path.to_owned(), t.clone());
        t
    }
    pub fn border(&mut self, store: &Store, class_path: &str) -> BorderStyle {
        if let Some(t) = self.border.get(class_path) { return t.clone(); }
        let d = store.class_defaults(class_path);
        let t = BorderStyle { background: d.get("Background").and_then(Brush::from_json) };
        self.border.insert(class_path.to_owned(), t.clone());
        t
    }
    pub fn button(&mut self, store: &Store, class_path: &str) -> ButtonStyle {
        if let Some(t) = self.button.get(class_path) { return t.clone(); }
        let t = ButtonStyle::from_defaults(&store.class_defaults(class_path));
        self.button.insert(class_path.to_owned(), t.clone());
        t
    }
}
