//! The Esc menu: the 1.8 `/Game/UI/Frontend/MainMenu/MainMenu` overlay (the same widget the lobby hamburger menu opens).
//!
//! HOST (reports/menu-hosts.json): Escape -> `AthenaHUD.HandleCursorModeChanged` -> `TopBar.ForceMenuOpen` -> TopBar, whose first overlay
//! layer is BackgroundBlur 16 > `MainMenuBorder` (Border_ShellUberBG = the ShellUberBG material) > SafeZone > VBox { Spacer [0,92],
//! MainMenu (fill, top) }. AthenaHUDMenu / AthenaSpectatorHUD do not exist in 1.8. `MainMenu.Border_0` is NoDrawType, so there is NO 50%
//! dim (the 1.10 notes misread its BrushColor): the backdrop is the ShellUberBG panel (`outline::PanelKind::Backdrop`, per-pixel port of
//! its compiled shader: opaque dark navy with a bluer, 60%-opaque radial glow towards the bottom right).
//!
//! RECOVERED (original data; reports/menu-layout-pause.json + menu-widgets.json + menu-strings.json + menu-shaders/cbuffers.json, cross-checked
//! by the tests at the bottom of this file):
//! * Content hugs the RIGHT edge below the 92-unit TopBar spacer: spacer 32 | text column (SizeBox min 288 / max 512) | spacer 14 | 4x4
//!   LineSeparator (white 0.2, padding top 8) | spacer 14 | icon column (SizeBox: 90 wide with a gamepad, 48 otherwise per
//!   `Set Icon Button List Column Width`) | spacer 16.
//! * Entries, their ORDER (icon column in 1.8: Messages, Settings, Legal, Logout, News, Credits, Exit) and labels, the original
//!   hover-description strings (now paired by the bound-event bytecode, reports `eventWiring`), the icon textures (52x52 `T-Icon-*-XL/-52`),
//!   the per-state materials (ButtonStyle-Outline / OutlineLeftShade / Outline-Red) drawn per pixel by `outline.rs`, and the text styles (NotoSans
//!   Bold 16, disabled = white 0.8 alpha), button min height 48, slot padding top 8, icon 32x32 (default brush size).
//! * The `ContextualHelpText` box: left-aligned with the text column (padding 32), 360 wide, min height 64, padding top
//!   8, TextStyle-Base-XS-BlueGray (NotoSans Regular 16, colour 0.521/0.552/0.745 linear); `UpdateDescriptionText` is
//!   driven by OnHovered / OnUnhovered of every button (empty when nothing is hovered). `LeaveGameHovered` shows the Leave button's own
//!   label; `AthenaPartyPrivacyButton`'s events jump into PrivacyButton's handlers (hover = its "Change your game privacy setting..." text).
//! * Which entries exist in a Battle Royale match (exact bytecode): `InitializeMainMenu` (Exit = !IsInZone && AllowQuit; Credits, Daily Rewards,
//!   Change Subgame collapse when IsInZone; Logout = AllowLogout) and `ConfigureSubGameWidgets` (GetSubGame() == 1: AthenaPartyPrivacyButton +
//!   CodeOfConduct visible, SocialButtonsVB collapsed); `Messages` and `News` start Collapsed and no function re-shows them.
//!   ASSUMED natives: IsInZone() = true in a match, AllowLogout() = true; SubGame 1 = Battle Royale (names are not serialized).
//! * Leave button label ("Leave Match" while IsConsideredInGame = IsInLobby || IsInZone, otherwise "Leave Party"), visibility, and the "Leave Match?"
//!   confirmation (title/message/Leave/Cancel strings, Primary-M buttons, Leave first and focused, Cancel = GenericBack = Esc, Lightbox dim 0.5).
//! * Escape is the `MainMenu` input action (closes) and `GenericBack`.
//!
//! RUNTIME CHOICES / NOT RECOVERED: BackgroundBlur 16 (native post effect; the backdrop is >= 60% opaque everywhere so the scene is barely
//! visible); the 80-unit TopBar strip (wordmark, social count, hamburger; its M-Shell-TopBar* materials and online widgets) is not drawn; the
//! open/close fade (no animation exists in the widget: 0.12 s fade-in/out is the runtime's, labelled in `MenuSystem`); text placement inside the
//! buttons (inferred from the IconTextButton tree); sounds (names only); the party bar, Messages/News widgets, Feedback form, Code of Conduct
//! (opens a native web URL) and Legal screen (LegalInfo: EULA + third-party tabs), Privacy popup and Logout do not exist in this runtime, so those
//! entries are shown DISABLED (greyed, no message of any kind: the original has none) and never activate. "Leave Match" has no lobby to return
//! to in this standalone runtime (the original confirms with `ExecuteClientEvent(UserInterface, LeaveZone)`): it quits to the desktop.
use super::outline::*;
use super::pause::SettingsScreen;
use super::theme::*;
use super::*;

// ---- layout constants (Slate units) -----------------------------------------------------------------------------------
pub const TOP_INSET: f32 = 92.; // TopBar VerticalBox_1 / Spacer_1 Size [0,92] above MainMenu (RECOVERED, reports/menu-hosts.json topBar.topSpacer)
pub const SPACER_LEAD: f32 = 32.;
pub const LEFT_W: f32 = 288.; // SizeBox_1 MinDesiredWidth (content is narrower, so the min applies)
pub const GAP: f32 = 14.;
pub const SEPARATOR_W: f32 = 4.;
pub const ICON_COL_W_MOUSE: f32 = 48.;
pub const ICON_COL_W_PAD: f32 = 90.;
pub const SPACER_RIGHT: f32 = 16.;
pub const BUTTON_H: f32 = 48.; // ButtonStyle-Outline MinHeight
pub const SLOT_PAD_TOP: f32 = 8.;
pub const ICON_SIZE: f32 = 32.;
pub const HELP_W: f32 = 360.;
pub const HELP_MIN_H: f32 = 64.;
pub const HELP_PAD_TOP: f32 = 8.;
pub const FONT_HELP: f32 = 16.; // TextStyle-Base-XS-BlueGray
pub const FONT_BUTTON: f32 = 16.; // TextStyle-Base-XS-B
/// Text inset inside a text button: button padding 4 + action widget 2 + left image 8 + text padding 5 (inferred from the IconTextButton tree).
pub const TEXT_INSET: f32 = 19.;
pub const SEPARATOR_COLOR: [f32; 4] = [1., 1., 1., 0.2];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuContext {
    /// In a match (`IsInZone() == true`): what Esc shows in-game.
    Match,
    /// Front end / lobby (`IsInZone() == false`): reference table only, not reachable in this runtime.
    Lobby,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column { Text, Icon }
/// What an entry does in THIS runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuntimeAction { Settings, LeaveMatch, Unavailable }

/// One widget of the original MainMenu tree.
#[derive(Clone, Copy, Debug)]
pub struct EntrySpec {
    /// Widget variable name in MainMenu_C (also the id key).
    pub widget: &'static str,
    pub column: Column,
    /// Original ButtonText (empty for icon buttons).
    pub label: &'static str,
    pub icon: Option<IconKind>,
    /// Red style (ButtonStyle-Outline-Red / Outline-LeftShade-Red).
    pub red: bool,
    /// Original hover description (ContextualHelpText), if the widget has one.
    pub hover: Option<&'static str>,
    pub in_match: bool,
    pub in_lobby: bool,
    pub runtime: RuntimeAction,
}
const fn e(widget: &'static str, column: Column, label: &'static str, icon: Option<IconKind>, red: bool, hover: Option<&'static str>, in_match: bool, in_lobby: bool, runtime: RuntimeAction) -> EntrySpec {
    EntrySpec { widget, column, label, icon, red, hover, in_match, in_lobby, runtime }
}
use Column::{Icon as CI, Text as CT};
use RuntimeAction::{LeaveMatch as LM, Settings as ST, Unavailable as NA};

/// Every widget of the original menu in tree order (text column first, then icon column), with its visibility per context.
pub const ENTRIES: &[EntrySpec] = &[
    e("Feedback", CT, "Feedback", None, false, Some("Feedback. Send feedback about bugs or game experiences."), true, true, NA),
    e("ButtonOutpostPermissions", CT, "Edit Permissions", None, false, Some("Edit Storm Shield permissions."), false, false, NA),
    e("BoostsButton", CT, "BOOSTS!", None, false, Some("Use XP Boosts on yourself or on your teammates."), false, false, NA),
    e("DailyRewardsButton", CT, "Daily Rewards", None, false, Some("Earn a reward for logging in every day."), false, false, NA),
    e("PartyInvites", CT, "Party Finder", None, false, Some("Invite your friends"), false, false, NA),
    e("PrivacyButton", CT, "Privacy: Public", None, false, Some("Change your game privacy setting to allow others to join your game."), false, false, NA),
    e("FriendsListButton", CT, "Friends  List", None, false, None, false, false, NA),
    e("InviteCodesButton", CT, "Friend Codes!", None, false, Some("Fortnite invite codes to give to friends."), false, false, NA),
    // AthenaPartyPrivacyButton: its bound OnHovered event jumps into PrivacyButton's handler (event 600 -> 7338: hover text), OnUnhovered clears it.
    e("AthenaPartyPrivacyButton", CT, "Privacy: Public", None, false, Some("Change your game privacy setting to allow others to join your game."), true, true, NA),
    e("CodeOfConduct", CT, "Code of Conduct", None, false, None, true, true, NA),
    e("LeaveButton", CT, "Leave Match", None, true, None, true, true, LM),
    e("ChangeSubgameButton", CT, "Select Game Mode", None, false, Some("Switch between Save the World mode and Battle Royale mode."), false, true, NA),
    e("MessagesButton", CI, "Messages", None, false, Some("View message history"), false, false, NA),
    e("SettingsButton", CI, "", Some(IconKind::Config), false, Some("Settings. Adjust game settings such as video, audio, and controls"), true, true, ST),
    e("Legal", CI, "", Some(IconKind::Legal), false, Some("Legal Information. View EULA and 3rd party software usage"), true, true, NA),
    // 1.8 tree order: Messages, Settings, Legal, Logout, News, Credits, Exit, TestUI (1.10: ..., News, Credits, Logout, Exit).
    e("LogoutButton", CI, "", Some(IconKind::Logout), false, Some("Logout of this account."), true, true, NA),
    e("ButtonNews", CI, "", Some(IconKind::News), false, Some("View news."), false, false, NA),
    e("ButtonCredits", CI, "", Some(IconKind::Credits), false, Some("View game credits."), false, true, NA),
    e("ExitButton", CI, "", Some(IconKind::Exit), true, Some("Exit the game."), false, true, NA),
    e("TestUIButton", CI, "", None, false, Some("Used for developer testing"), false, false, NA),
];

impl EntrySpec {
    pub fn visible(&self, ctx: MenuContext) -> bool { match ctx { MenuContext::Match => self.in_match, MenuContext::Lobby => self.in_lobby } }
    pub fn available(&self) -> bool { self.runtime != RuntimeAction::Unavailable }
    /// Button text: the Leave button reads "Leave Match" in a match and "Leave Party" otherwise (GetLeaveActionText, Athena branch).
    pub fn label_in(&self, ctx: MenuContext) -> &'static str { if self.widget == "LeaveButton" && ctx == MenuContext::Lobby { "Leave Party" } else { self.label } }
    pub fn style(&self) -> ProcStyle {
        match (self.column, self.red) {
            (Column::Text, false) => ProcStyle::Outline, (Column::Text, true) => ProcStyle::OutlineRed,
            (Column::Icon, false) => ProcStyle::OutlineLeftShade, (Column::Icon, true) => ProcStyle::OutlineLeftShadeRed,
        }
    }
    /// Text for the description box: the original hover description, plus the honest not-available line for entries this build cannot run.
    pub fn description(&self) -> String {
        let s = self.hover.unwrap_or("").to_owned();
        s
    }
    /// Description for a context: `LeaveGameHovered` calls `UpdateDescriptionText(LeaveButton.GetButton().ButtonText)`, so hovering Leave shows
    /// the button's own label ("Leave Match" / "Leave Party"); every other entry shows `description()`.
    pub fn description_in(&self, ctx: MenuContext) -> String { if self.widget == "LeaveButton" { self.label_in(ctx).to_owned() } else { self.description() } }
}
/// Indices into `ENTRIES` of the visible entries of one column, in original order.
pub fn visible_entries(ctx: MenuContext, column: Column) -> Vec<usize> { ENTRIES.iter().enumerate().filter(|(_, x)| x.column == column && x.visible(ctx)).map(|(i, _)| i).collect() }
pub fn entry_index(widget: &str) -> Option<usize> { ENTRIES.iter().position(|x| x.widget == widget) }

// ---- layout (pure) ------------------------------------------------------------------------------------------------------
pub struct Layout {
    /// (index into ENTRIES, pixel rect) in focus order: text column top to bottom, then icon column.
    pub items: Vec<(usize, Rect)>,
    pub sep: Rect,
    pub help: Rect,
    /// Pixel x of the text column / icon column.
    pub left_x: f32,
    pub icon_x: f32,
    pub icon_w: f32,
}
impl Layout { pub fn rect_of(&self, widget: &str) -> Option<Rect> { let i = entry_index(widget)?; self.items.iter().find(|(k, _)| *k == i).map(|(_, r)| *r) } }

/// MainMenu geometry for a `vw` x `vh` (Slate units) viewport at `scale` px/unit. `wide_icons` = gamepad column width (90).
pub fn layout(vw: f32, vh: f32, scale: f32, ctx: MenuContext, wide_icons: bool) -> Layout {
    let wi = if wide_icons { ICON_COL_W_PAD } else { ICON_COL_W_MOUSE };
    let fixed = SPACER_LEAD + GAP + SEPARATOR_W + GAP + wi + SPACER_RIGHT;
    let left_w = if vw < fixed + LEFT_W { (vw - fixed).clamp(40., LEFT_W) } else { LEFT_W };
    let x0 = (vw - (fixed + left_w)).max(0.);
    let left_x = x0 + SPACER_LEAD;
    let sep_x = left_x + left_w + GAP;
    let icon_x = sep_x + SEPARATOR_W + GAP;
    let (lc, ic) = (visible_entries(ctx, Column::Text), visible_entries(ctx, Column::Icon));
    let col_h = |n: usize| n as f32 * (SLOT_PAD_TOP + BUTTON_H);
    let cols_h = col_h(lc.len()).max(col_h(ic.len()));
    let mut inset = TOP_INSET;
    let need = inset + cols_h + HELP_PAD_TOP + HELP_MIN_H + 24.; // trailing Spacer_0 (0,24)
    if need > vh { inset = (inset - (need - vh)).max(0.); }
    let s = |x: f32, y: f32, w: f32, h: f32| Rect::new(x * scale, y * scale, w * scale, h * scale);
    let mut items = Vec::new();
    for (list, x, w) in [(&lc, left_x, left_w), (&ic, icon_x, wi)] {
        let mut y = inset;
        for &i in list { y += SLOT_PAD_TOP; items.push((i, s(x, y, w, BUTTON_H))); y += BUTTON_H; }
    }
    Layout {
        items,
        sep: s(sep_x, inset + SLOT_PAD_TOP, SEPARATOR_W, (cols_h - SLOT_PAD_TOP).max(0.)),
        help: s(left_x, inset + cols_h + HELP_PAD_TOP, HELP_W.min((vw - left_x).max(40.)), HELP_MIN_H),
        left_x: left_x * scale, icon_x: icon_x * scale, icon_w: wi * scale,
    }
}
/// Convenience for tests/scripted captures: pixel rect of a widget in a `w` x `h` client (mouse/keyboard layout).
pub fn entry_rect(w: f32, h: f32, ctx: MenuContext, widget: &str) -> Option<Rect> {
    let vp = Viewport::new(w, h); let (vw, vh) = vp.slate_size(); layout(vw, vh, vp.scale, ctx, false).rect_of(widget)
}

// ---- screen --------------------------------------------------------------------------------------------------------------
/// The Esc menu. Registered in the menu stack under the historical name "pause".
pub struct MainMenuScreen { pub ctx: MenuContext }
impl MainMenuScreen {
    /// The in-game menu (Match context).
    pub fn new() -> Self { Self { ctx: MenuContext::Match } }
    pub fn with_context(ctx: MenuContext) -> Self { Self { ctx } }
}

impl Screen for MainMenuScreen {
    fn name(&self) -> &str { "pause" }
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let (vw, vh) = ui.vp.slate_size();
        let ctx = self.ctx;
        let lay = layout(vw, vh, ui.vp.scale, ctx, ui.st.pad_mode);
        // Initial keyboard/pad focus (the original does not say which button; Settings is the safest first stop: ASSUMED).
        if let Some(i) = entry_index("SettingsButton") { let id = ui.id(ENTRIES[i].widget); ui.default_focus(id); }
        // Backdrop = TopBar.MainMenuBorder (ShellUberBG material over the whole screen); MainMenu's own Border_0 is NoDrawType (draws nothing).
        ui.panel(PanelKind::Backdrop, ui.screen());
        ui.fill(lay.sep, SEPARATOR_COLOR);

        let (mut hot_entry, mut focus_entry, mut action) = (None, None, None);
        for &(idx, r) in &lay.items {
            let spec = &ENTRIES[idx];
            let id = ui.id(spec.widget);
            let avail = spec.available();
            // Unavailable entries stay focusable/hoverable (keyboard users get the description too) but never activate.
            let it = ui.interact(id, r, true, true);
            let vis = if !avail { Vis::Disabled } else if it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal };
            ui.procedural(spec.style(), vis, r);
            if !avail && it.focused { outline_frame(ui, r, [1., 1., 1., 0.55]); }
            match spec.column {
                Column::Text => {
                    let color = if avail { WHITE } else { [1., 1., 1., 0.8] }; // TextStyle-Button-Outline-XS-Disabled
                    let inset = ui.u(TEXT_INSET);
                    ui.text(Rect::new(r.x + inset, r.y, (r.w - inset - ui.u(9.)).max(0.), r.h), spec.label_in(ctx), TextStyle::new(FontId::Bold, FONT_BUTTON, color), HAlign::Left);
                }
                Column::Icon => if let Some(icon) = spec.icon {
                    let sz = ui.u(ICON_SIZE);
                    // Right aligned (ContentAlignment HAlign_Right) inside button padding 4 + slot padding 4.
                    let ir = Rect::new(r.x + r.w - ui.u(8.) - sz, r.y + (r.h - sz) * 0.5, sz, sz);
                    ui.sprite(BrushId::Icon(icon), ir, false, if avail { WHITE } else { [1., 1., 1., 0.5] });
                },
            }
            if it.hot { hot_entry = Some(idx); }
            if it.focused { focus_entry = Some(idx); }
            if it.clicked && avail { action = Some(spec.runtime); }
        }

        // Left/Right hop between the two columns (linear Up/Down focus order is handled by the toolkit).
        let col_ids = |c: Column| -> Vec<Id> { lay.items.iter().filter(|(i, _)| ENTRIES[*i].column == c).map(|(i, _)| ui.id(ENTRIES[*i].widget)).collect() };
        let (left, right) = (col_ids(Column::Text), col_ids(Column::Icon));
        if let (Some(f), false, false) = (ui.st.focused_id(), left.is_empty(), right.is_empty()) {
            if let Some(r) = left.iter().position(|i| *i == f) { let o = right[r.min(right.len() - 1)]; ui.horizontal_focus(&[f, o]); }
            else if let Some(r) = right.iter().position(|i| *i == f) { let o = left[r.min(left.len() - 1)]; ui.horizontal_focus(&[o, f]); }
        }

        // ContextualHelpText: the keyboard/pad-focused entry (focus is only "visible" after keyboard/pad input and is hidden
        // again by any mouse movement) wins over a stationary mouse hover; empty when nothing is hovered or focused.
        if let Some(i) = focus_entry.or(hot_entry) {
            let text = ENTRIES[i].description_in(ctx);
            ui.paragraph(lay.help, &text, TextStyle::new(FontId::Regular, FONT_HELP, srgba(TEXT_BLUE_GRAY, 1.)), HAlign::Left);
        }

        match action {
            Some(RuntimeAction::Settings) => cx.push(Box::new(SettingsScreen::new())),
            Some(RuntimeAction::LeaveMatch) => cx.push(Box::new(ConfirmationWindow::leave_match())),
            _ => {}
        }
        // Escape = InputAction `MainMenu` (toggle) / `GenericBack`; the gamepad's Start/B map to Back as well.
        if ui.back_pressed() { cx.resume(); }
    }
}

// ---- ConfirmationWindow (/Game/UI/Foundation/Modals/ConfirmationWindow) --------------------------------------------------
// RECOVERED: SizeBox width 1000, min height 600, overlay padding 200 top/bottom, centred; Border-MainModal panel
// (M_UI_GradientPanel RGBA0/RGBA1); 100-unit-high ModalHeader (M-Border-ModalHeader-procedural) with the Title in
// TextStyle-Header-L (Burbank-Black 48, white), title padding 8 after a 20 spacer; description in TextStyle-Base-XS-Black
// (NotoSans Regular 16, black) padded 20, top 20; ButtonBox bottom-right with padding 20, buttons Primary-M (min 160 x
// 48, padding 4 each side, text NotoSans Bold 18, TextStyle-Button-Primary-M), first button (confirm) is centred/focused
// (`CenterFirstButton`). Lightbox dim = Border_0 BrushColor 0,0,0,0.5 (RECOVERED, reports/menu-hosts.json lightbox); panel + header are the
// M_UI_GradientPanel shader evaluated per pixel (`outline::panel_pixel`; gradient direction = Rotation*0.25 radians, INFERRED units). NOT
// reproduced: BackgroundBlur strength 15 (no blur pass).
pub const DLG_W: f32 = 1000.;
pub const DLG_MIN_H: f32 = 600.;
pub const DLG_V_MARGIN: f32 = 200.;
pub const DLG_HEADER_H: f32 = 100.;
pub const DLG_SIDE: f32 = 20.;
pub const DLG_TITLE_X: f32 = 28.; // 20 spacer + 8 title padding (main icon collapsed when empty: ASSUMED)
pub const DLG_DESC_TOP: f32 = 20.;
pub const DLG_BTN_BOX: f32 = 20.;
pub const DLG_BTN_PAD: f32 = 4.;
pub const DLG_BTN_H: f32 = 48.;
pub const DLG_BTN_MIN_W: f32 = 160.;
pub const DLG_NOTE_GAP: f32 = 12.;
pub const FONT_DLG_TITLE: f32 = 48.;
pub const FONT_DLG_BODY: f32 = 16.;
pub const FONT_DLG_BUTTON: f32 = 18.;
/// Lightbox dim behind the dialog: Lightbox.Border_0 BrushColor (0,0,0,0.5) with a default (drawn) brush (RECOVERED).
pub const DLG_DIM: f32 = 0.5;

/// Panel width in Slate units (1000, never wider than the viewport).
pub fn confirm_width(vw: f32) -> f32 { DLG_W.min((vw - 32.).max(vw * 0.8)) }
/// ConfirmationWindow geometry in pixels: (panel, header, body, confirm button, cancel button). Pure for tests.
pub fn confirm_layout(vw: f32, vh: f32, scale: f32, body_h: f32) -> (Rect, Rect, Rect, Rect, Rect) {
    let w = confirm_width(vw);
    let btn_area = DLG_BTN_BOX + DLG_BTN_PAD + DLG_BTN_H + DLG_BTN_PAD + DLG_BTN_BOX;
    let needed = DLG_HEADER_H + DLG_DESC_TOP + body_h + btn_area;
    let max_h = (vh - 2. * DLG_V_MARGIN).max(0.);
    let h = needed.max(DLG_MIN_H).min(max_h.max(needed)).min(vh - 16.).max(1.);
    let (x, y) = ((vw - w) * 0.5, (vh - h) * 0.5);
    let s = |x: f32, y: f32, w: f32, h: f32| Rect::new(x * scale, y * scale, w * scale, h * scale);
    let panel = s(x, y, w, h);
    let header = s(x, y, w, DLG_HEADER_H.min(h * 0.5));
    let body = s(x + DLG_SIDE, y + DLG_HEADER_H + DLG_DESC_TOP, w - 2. * DLG_SIDE, body_h);
    let bw = DLG_BTN_MIN_W.min(((w - 2. * DLG_BTN_BOX - 4. * DLG_BTN_PAD) * 0.5).max(40.));
    let by = y + h - DLG_BTN_BOX - DLG_BTN_PAD - DLG_BTN_H;
    let cancel_x = x + w - DLG_BTN_BOX - DLG_BTN_PAD - bw;
    let confirm_x = cancel_x - 2. * DLG_BTN_PAD - bw;
    (panel, header, body, s(confirm_x, by, bw, DLG_BTN_H), s(cancel_x, by, bw, DLG_BTN_H))
}

type Callback = Box<dyn FnMut(&mut Ctx)>;
/// Modal confirmation in the style of the original ConfirmationWindow. Confirm button first (left) and initially focused,
/// Cancel second; Esc / pad B = the original `Cancel` input action (GenericBack).
pub struct ConfirmationWindow { pub title: String, pub body: String, pub note: Option<String>, pub confirm: String, pub cancel: String, on_confirm: Option<Callback> }
impl ConfirmationWindow {
    pub fn new(title: &str, body: &str, confirm: &str, cancel: &str, on_confirm: impl FnMut(&mut Ctx) + 'static) -> Self {
        Self { title: title.into(), body: body.into(), note: None, confirm: confirm.into(), cancel: cancel.into(), on_confirm: Some(Box::new(on_confirm)) }
    }
    /// Runtime-added second paragraph (not original text), shown under the original message.
    pub fn with_note(mut self, note: &str) -> Self { self.note = Some(note.into()); self }
    /// LeaveButton's `ShowSimpleConfirmationDialog_CustomInput`: title "Leave Match?", message (no party) "Leave the match
    /// return to the lobby?" (sic, the original string), actions "Leave" / "Cancel". This runtime has no lobby, so the
    /// confirmed action quits to the desktop and a runtime note says so.
    pub fn leave_match() -> Self {
        Self::new("Leave Match?", "Leave the match return to the lobby?", "Leave", "Cancel", |cx| cx.quit_to_desktop())
            .with_note("This standalone build has no lobby: leaving closes the game and returns to the desktop.")
    }
}
impl Screen for ConfirmationWindow {
    fn name(&self) -> &str { "confirm" }
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let (vw, vh) = ui.vp.slate_size();
        let sc = ui.vp.scale;
        ui.fill(ui.screen(), [0., 0., 0., DLG_DIM]);
        let px = ui.text_px(FONT_DLG_BODY) as f32;
        let wrap_w = ((confirm_width(vw) - 2. * DLG_SIDE) * sc).max(1.);
        let lh = (ui.fonts.line_height(FontId::Regular, px) * 1.15).ceil() / sc;
        let n1 = wrap_text(ui.fonts, FontId::Regular, &self.body, px, wrap_w).len() as f32;
        let n2 = self.note.as_ref().map_or(0., |n| wrap_text(ui.fonts, FontId::Regular, n, px, wrap_w).len() as f32);
        let body_h = (n1 * lh + if n2 > 0. { DLG_NOTE_GAP + n2 * lh } else { 0. }).max(lh);
        let (panel, header, body, confirm, cancel) = confirm_layout(vw, vh, sc, body_h);
        // Panel (M-Border-MainModal) and header (ModalHeader): the M_UI_GradientPanel shader, per pixel (outline::panel_pixel).
        ui.panel(PanelKind::ModalPanel, panel);
        ui.panel(PanelKind::ModalHeader, header);
        ui.text(Rect::new(panel.x + ui.u(DLG_TITLE_X), header.y, (panel.w - ui.u(2. * DLG_TITLE_X)).max(0.), header.h), &self.title, TextStyle::new(FontId::Title, FONT_DLG_TITLE, WHITE), HAlign::Left);
        let used = ui.paragraph(body, &self.body, TextStyle::new(FontId::Regular, FONT_DLG_BODY, srgba(TEXT_BLACK, 1.)), HAlign::Left);
        if let Some(note) = &self.note {
            let r = Rect::new(body.x, body.y + used + ui.u(DLG_NOTE_GAP), body.w, (body.h - used).max(0.));
            ui.paragraph(r, note, TextStyle::new(FontId::Regular, FONT_DLG_BODY, [0.16, 0.2, 0.28, 1.]), HAlign::Left);
        }
        let (cid, xid) = (ui.id("confirm_yes"), ui.id("confirm_no"));
        ui.default_focus(cid); // CenterFirstButton: the first button of the ButtonBox (the confirm action)
        let (mut confirmed, mut cancelled) = (false, false);
        for (id, r, label, is_confirm) in [(cid, confirm, self.confirm.as_str(), true), (xid, cancel, self.cancel.as_str(), false)] {
            let it = ui.interact(id, r, true, true);
            let vis = if it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal };
            ui.procedural(ProcStyle::PrimaryM, vis, r);
            let pad = ui.u(6.);
            ui.text(Rect::new(r.x + pad, r.y, (r.w - 2. * pad).max(0.), r.h), label, TextStyle::new(FontId::Bold, FONT_DLG_BUTTON, srgba(PRIMARY_M_TEXT, 1.)), HAlign::Center);
            if it.clicked { if is_confirm { confirmed = true } else { cancelled = true } }
        }
        ui.horizontal_focus(&[cid, xid]);
        if confirmed { if let Some(mut f) = self.on_confirm.take() { f(cx); } }
        if cancelled || ui.back_pressed() { cx.pop(); }
    }
}

/// 1.5-unit square frame (keyboard focus marker for entries that are disabled and therefore have no hover look).
fn outline_frame(ui: &mut Ui, r: Rect, color: [f32; 4]) {
    let t = ui.u(1.5).round().max(1.);
    ui.fill(Rect::new(r.x, r.y, r.w, t), color); ui.fill(Rect::new(r.x, r.y + r.h - t, r.w, t), color);
    ui.fill(Rect::new(r.x, r.y + t, t, (r.h - 2. * t).max(0.)), color); ui.fill(Rect::new(r.x + r.w - t, r.y + t, t, (r.h - 2. * t).max(0.)), color);
}

// ======================================================================================================================
#[cfg(test)]
mod tests {
    //! Conformance tests: the constants and tables above are asserted against the evidence JSON written by
    //! tools/menu-evidence.cjs (reports/menu-*.json), plus behaviour/layout/draw-list checks of the real screens.
    use super::*;
    use serde_json::Value;

    fn json(name: &str) -> Value { crate::testutil::report_json(name) }
    fn fonts() -> Fonts { crate::testutil::fonts() }

    struct H { m: MenuSystem, w: f32, h: f32 }
    impl H {
        fn new(w: f32, h: f32) -> Self { let mut m = MenuSystem::new(fonts()); m.fade_time = 0.; m.open(); let mut s = H { m, w, h }; s.frame(); s.frame(); s }
        fn frame(&mut self) -> Vec<Cmd> { self.m.frame(self.w, self.h, 1. / 60.).cmds.clone() }
        fn ev(&mut self, e: Event) { self.m.event(e); self.frame(); }
        fn mv(&mut self, p: (f32, f32)) { self.ev(Event::MouseMove(p.0, p.1)); }
        fn click(&mut self, p: (f32, f32)) { self.mv(p); self.ev(Event::MouseDown(MouseButton::Left, p.0, p.1)); self.ev(Event::MouseUp(MouseButton::Left, p.0, p.1)); self.frame(); }
        fn at(&self, widget: &str) -> (f32, f32) { let r = entry_rect(self.w, self.h, MenuContext::Match, widget).unwrap_or_else(|| panic!("{widget}")); (r.x + r.w * 0.5, r.y + r.h * 0.5) }
        fn texts(&mut self) -> Vec<String> { self.frame().iter().filter_map(|c| if let Cmd::Text { text, .. } = c { Some(text.clone()) } else { None }).collect() }
    }
    const SIZES: [(f32, f32); 6] = [(1920., 1080.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.)];

    fn find_mat<'a>(w: &'a Value, name: &str) -> &'a Value {
        let m = w["materials"].as_object().unwrap();
        let k = m.keys().find(|k| k.ends_with(&format!("/{name}"))).unwrap_or_else(|| panic!("material {name}"));
        &m[k]["effective"]
    }
    fn f3(v: &Value) -> [f32; 3] { let a = v.as_array().unwrap(); [a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32, a[2].as_f64().unwrap() as f32] }
    fn sc(v: &Value, k: &str) -> f32 { v["scalars"][k].as_f64().unwrap_or_else(|| panic!("scalar {k}")) as f32 }
    fn close3(a: [f32; 3], b: [f32; 3]) -> bool { (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5) }

    #[test] fn entry_table_matches_menu_layout_pause_json() {
        let j = json("menu-layout-pause.json");
        let strings = json("menu-strings.json");
        let known: std::collections::HashSet<String> = strings["strings"].as_array().unwrap().iter().filter_map(|s| s["en"].as_str().map(str::to_owned)).collect();
        for (col, key) in [(Column::Text, "leftColumn"), (Column::Icon, "iconColumn")] {
            let items = j[key]["items"].as_array().unwrap();
            let ours: Vec<&EntrySpec> = ENTRIES.iter().filter(|e| e.column == col).collect();
            assert_eq!(items.len(), ours.len(), "{key}: same number of widgets");
            for (i, (it, e)) in items.iter().zip(&ours).enumerate() {
                assert_eq!(it["widget"].as_str().unwrap(), e.widget, "{key} order {i}");
                assert_eq!(it["order"].as_u64().unwrap() as usize, i);
                match it["label"]["text"].as_str() { Some(t) if !t.is_empty() && e.widget != "LeaveButton" => { assert_eq!(t, e.label, "{} label", e.widget); assert!(known.contains(t), "{t:?} is a shipped string"); } _ => {} }
                match (it["hoverDescription"]["text"].as_str(), e.hover) {
                    (Some(a), Some(b)) => { assert_eq!(a, b, "{} hover description", e.widget); assert!(known.contains(a)); }
                    (None, None) => {}
                    (a, b) => panic!("{} hover mismatch {a:?} vs {b:?}", e.widget),
                }
                if it["initialVisibility"].as_str() == Some("ESlateVisibility::Collapsed") && e.in_match { assert!(false, "{} starts Collapsed and is never re-shown by decoded code", e.widget); }
                if let (Some(path), Some(icon)) = (it["rightExtraContent"][0]["brush"]["resource"]["path"].as_str(), e.icon) {
                    let stem = path.rsplit('/').next().unwrap().split('.').next().unwrap();
                    assert_eq!(format!("{stem}.png"), icon.file_name(), "{} icon texture", e.widget);
                }
                let style = it["style"].as_str().or(it["controllerStyle"].as_str()).unwrap_or("");
                if e.widget != "LeaveButton" { assert_eq!(style.contains("Red"), e.red, "{} red style ({style})", e.widget); }
            }
        }
        // LeaveButton: red outline style, label logic (Athena branch) and the dialog strings.
        assert!(ENTRIES[entry_index("LeaveButton").unwrap()].red);
        assert!(j["leaveButton"]["tree"]["props"]["Style"]["path"].as_str().unwrap().contains("Outline-Red"));
        let texts: Vec<&str> = j["leaveButton"]["labelTexts"].as_array().unwrap().iter().map(|t| t["text"].as_str().unwrap()).collect();
        assert!(texts.contains(&"Leave Match") && texts.contains(&"Leave Party"));
        assert_eq!(ENTRIES[entry_index("LeaveButton").unwrap()].label_in(MenuContext::Match), "Leave Match");
        assert_eq!(ENTRIES[entry_index("LeaveButton").unwrap()].label_in(MenuContext::Lobby), "Leave Party");
        assert_eq!(ENTRIES[entry_index("ChangeSubgameButton").unwrap()].label, j["changeSubgameButton"]["tree"]["children"][0]["children"][0]["props"]["ButtonText"]["text"].as_str().unwrap());
        // 1.8 bytecode evidence: Leave hover = its own label (LeaveGameHovered), Athena privacy hover = PrivacyButton's text (event wiring), both exact.
        assert_eq!(j["leaveButton"]["hoverDescription"]["fn"], "LeaveGameHovered");
        assert_eq!(ENTRIES[entry_index("LeaveButton").unwrap()].description_in(MenuContext::Match), "Leave Match");
        let ap = &j["eventWiring"]["AthenaPartyPrivacyButton"]["hover"]["text"]; assert_eq!(ap.as_str(), ENTRIES[entry_index("AthenaPartyPrivacyButton").unwrap()].hover, "AthenaPartyPrivacyButton hover text");
        assert_eq!(j["eventWiring"]["PrivacyButton"]["hover"]["text"], *ap, "the Athena button's events jump into PrivacyButton's handlers");
        // every hover description in the table is the one wired to that widget's OnHovered event (exact pairing, no longer inferred)
        for e in ENTRIES { if let Some(h) = e.hover { if let Some(w) = j["eventWiring"].get(e.widget) { if let Some(t) = w["hover"]["text"].as_str() { assert_eq!(t, h, "{} hover wired by its bound event", e.widget); } } } }
    }
    #[test] fn match_context_has_exactly_the_recovered_entries() {
        let names = |c| visible_entries(MenuContext::Match, c).iter().map(|i| ENTRIES[*i].widget).collect::<Vec<_>>();
        assert_eq!(names(Column::Text), ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "LeaveButton"]);
        assert_eq!(names(Column::Icon), ["SettingsButton", "Legal", "LogoutButton"]);
        let lobby = |c| visible_entries(MenuContext::Lobby, c).iter().map(|i| ENTRIES[*i].widget).collect::<Vec<_>>();
        assert_eq!(lobby(Column::Text), ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "LeaveButton", "ChangeSubgameButton"]);
        assert_eq!(lobby(Column::Icon), ["SettingsButton", "Legal", "LogoutButton", "ButtonCredits", "ExitButton"], "1.8 tree order: Logout comes before News/Credits/Exit");
        // inGameVariant (report): hidden in a match = Exit, Credits, Daily Rewards, Change Subgame
        for w in ["ExitButton", "ButtonCredits", "DailyRewardsButton", "ChangeSubgameButton"] { assert!(!ENTRIES[entry_index(w).unwrap()].visible(MenuContext::Match), "{w} is lobby-only"); }
        // no "Resume": neither a widget nor a shipped string
        assert!(ENTRIES.iter().all(|e| !e.label.to_ascii_lowercase().contains("resume")));
        let strings = json("menu-strings.json");
        assert!(strings["strings"].as_array().unwrap().iter().all(|s| !s["en"].as_str().unwrap_or("").to_ascii_lowercase().contains("resume")), "1.8 has no Resume string either (Game.locres, MainMenu texts)");
        // inGameVariant (bytecode) lists exactly the entries this table shows in a match
        let iv = &json("menu-layout-pause.json")["inGameVariant"]["battleRoyaleMatch"];
        let widgets = |k: &str| iv[k].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(widgets("textColumn"), names(Column::Text)); assert_eq!(widgets("iconColumn"), names(Column::Icon));
        // exactly two entries do something here
        let live: Vec<_> = ENTRIES.iter().filter(|e| e.visible(MenuContext::Match) && e.available()).map(|e| e.widget).collect();
        assert_eq!(live, ["LeaveButton", "SettingsButton"]);
    }
    #[test] fn style_constants_match_menu_widgets_json() {
        use super::super::outline::*;
        let w = json("menu-widgets.json");
        let table: [(ProcStyle, [&str; 4]); 4] = [
            (ProcStyle::Outline, ["M-Button-Outline-Base_Fill", "M-Button-Outline-Hovered", "M-Button-Outline-Pressed", "M-Button-Outline-Disabled"]),
            (ProcStyle::OutlineLeftShade, ["M-Button-OutlineLeftShade-Base", "M-Button-OutlineLeftShade-Hovered", "M-Button-OutlineLeftShade-Pressed", "M-Button-OutlineLeftShade-Disabled"]),
            (ProcStyle::OutlineRed, ["M-Button-Outline-Base_Red", "M-Button-Outline-Hovered", "M-Button-Outline-Pressed_Red", "M-Button-Outline-Disabled"]),
            (ProcStyle::OutlineLeftShadeRed, ["M-Button-Outline-LeftShade-Base_Red", "M-Button-OutlineLeftShade-Hovered", "M-Button-Outline-LeftShade-Pressed_Red", "M-Button-OutlineLeftShade-Disabled"]),
        ];
        for (style, mats) in table {
            for (vis, m) in [Vis::Normal, Vis::Hover, Vis::Pressed, Vis::Disabled].into_iter().zip(mats) {
                let e = find_mat(&w, m); let l = look(style, vis);
                assert!(close3(l.primary, f3(&e["vectors"]["Primary Color"])), "{style:?} {vis:?} primary");
                assert!(close3(l.base, f3(&e["vectors"]["Base Color"])), "{style:?} {vis:?} base");
                for (k, v) in [("Dark Base Alpha", l.dark_base_alpha), ("Gradient Global Alpha", l.gradient_alpha), ("Gradient Min Alpha", l.gradient_min_alpha), ("Outline Alpha", l.outline_alpha), ("Inner Fill Offset", l.inner_fill_offset), ("Switch Offset", l.switch_offset)] {
                    assert!((sc(e, k) - v).abs() < 1e-5, "{style:?} {vis:?} {k}: {} vs {v}", sc(e, k));
                }
            }
        }
        for (vis, m) in [Vis::Normal, Vis::Hover, Vis::Pressed, Vis::Disabled].into_iter().zip(["M-Button-Primary-M-Base", "M-Button-Primary-M-Hovered", "M-Button-Primary-M-Pressed", "M-Button-Primary-M-Disabled"]) {
            let e = find_mat(&w, m); let (c0, c1, sh, a) = primary_m_look(vis);
            assert!(close3(c0, f3(&e["vectors"]["RGBA0"])) && close3(c1, f3(&e["vectors"]["RGBA1"])) && close3(sh, f3(&e["vectors"]["Drop Shadow"])), "{vis:?}");
            assert!((sc(e, "Alpha") - a).abs() < 1e-5);
        }
        let m = find_mat(&w, "M-Border-MainModal"); assert!(close3(MODAL_RGBA0, f3(&m["vectors"]["RGBA0"])) && close3(MODAL_RGBA1, f3(&m["vectors"]["RGBA1"])));
        let m = find_mat(&w, "M-Border-ModalHeader-procedural"); assert!(close3(HEADER_RGBA0, f3(&m["vectors"]["RGBA0"])) && close3(HEADER_RGBA1, f3(&m["vectors"]["RGBA1"])));
        let st = |n: &str| w["styles"].as_object().unwrap().iter().find(|(k, _)| k.ends_with(&format!("/{n}"))).map(|(_, v)| v["props"].clone()).unwrap();
        assert!(close3(PRIMARY_M_TEXT, f3(&st("TextStyle-Button-Primary-M")["Color"])));
        assert!(close3(TEXT_BLUE_GRAY, f3(&st("TextStyle-Base-XS-BlueGray")["Color"])));
        assert_eq!(st("TextStyle-Base-XS-B")["Font"]["size"].as_f64().unwrap() as f32, FONT_BUTTON, "button text size");
        assert_eq!(st("TextStyle-Base-XS-BlueGray")["Font"]["size"].as_f64().unwrap() as f32, FONT_HELP);
        assert_eq!(st("TextStyle-Header-L")["Font"]["size"].as_f64().unwrap() as f32, FONT_DLG_TITLE);
        assert_eq!(st("TextStyle-Button-Primary-M")["Font"]["size"].as_f64().unwrap() as f32, FONT_DLG_BUTTON);
        assert_eq!(st("TextStyle-Base-XS-Black")["Font"]["size"].as_f64().unwrap() as f32, FONT_DLG_BODY);
    }
    fn cb() -> Value { crate::testutil::report_json("menu-shaders/cbuffers.json") }
    fn slot(c: &Value, mat: &str, name: &str) -> f32 {
        let v = &c["materials"][mat]["slots"][name]["value"]; v.as_f64().unwrap_or_else(|| panic!("{mat}.{name} {v}")) as f32
    }
    #[test] fn material_constants_match_the_disassembled_shaders() {
        use super::super::outline::*;
        let c = cb();
        // Outline family (M_MenuButton_Master, outline.plain.asm): every state/style, constants read from the evaluated constant buffer.
        for (style, vis, m) in [
            (ProcStyle::Outline, Vis::Normal, "M-Button-Outline-Base_Fill"), (ProcStyle::Outline, Vis::Hover, "M-Button-Outline-Hovered"), (ProcStyle::Outline, Vis::Pressed, "M-Button-Outline-Pressed"), (ProcStyle::Outline, Vis::Disabled, "M-Button-Outline-Disabled"),
            (ProcStyle::OutlineRed, Vis::Normal, "M-Button-Outline-Base_Red"), (ProcStyle::OutlineRed, Vis::Pressed, "M-Button-Outline-Pressed_Red"),
            (ProcStyle::OutlineLeftShade, Vis::Normal, "M-Button-OutlineLeftShade-Base"), (ProcStyle::OutlineLeftShade, Vis::Hover, "M-Button-OutlineLeftShade-Hovered"), (ProcStyle::OutlineLeftShade, Vis::Pressed, "M-Button-OutlineLeftShade-Pressed"), (ProcStyle::OutlineLeftShade, Vis::Disabled, "M-Button-OutlineLeftShade-Disabled"),
            (ProcStyle::OutlineLeftShadeRed, Vis::Normal, "M-Button-Outline-LeftShade-Base_Red"), (ProcStyle::OutlineLeftShadeRed, Vis::Pressed, "M-Button-Outline-LeftShade-Pressed_Red"),
        ] {
            let l = look(style, vis);
            for (k, v) in [("Box Size", l.box_size), ("Dark Base Alpha", l.dark_base_alpha), ("Inner Fill Offset", l.inner_fill_offset), ("Switch Offset", l.switch_offset), ("Gradient Min Alpha", l.gradient_min_alpha), ("Gradient Global Alpha", l.gradient_alpha), ("Outline Alpha", l.outline_alpha)] {
                assert!((slot(&c, m, k) - v).abs() < 1e-5, "{m} {k}: {} vs {v}", slot(&c, m, k));
            }
            assert_eq!(slot(&c, m, "s_UseShineSwipe"), 0., "{m}: the shine swipe is off, so the ports may ignore it");
            let prim = &c["materials"][m]["slots"]["Primary Color"]["value"]; for k in 0..3 { assert!((prim[k].as_f64().unwrap() as f32 - l.primary[k]).abs() < 1e-5, "{m} Primary Color"); }
            // derived registers of the shader that the port hard-codes: 1 - Switch Offset (cb[15].z)
            assert!((c["materials"][m]["cb"][15][2].as_f64().unwrap() as f32 - (1. - l.switch_offset)).abs() < 1e-6, "{m} cb[15].z = 1 - SwitchOffset");
        }
        // Primary-M (M-Button-Master, primary.plain.asm)
        for (vis, m) in [(Vis::Normal, "M-Button-Primary-M-Base"), (Vis::Hover, "M-Button-Primary-M-Hovered"), (Vis::Pressed, "M-Button-Primary-M-Pressed"), (Vis::Disabled, "M-Button-Primary-M-Disabled")] {
            for (k, v) in [("Size", PM_SIZE), ("Drop Shadow Distance", PM_SHADOW_DIST), ("SS Min", PM_SS_MIN), ("SS Max", PM_SS_MAX), ("DropShadow Alpha", PM_SHADOW_ALPHA), ("LG SS Min_1", PM_LG.0), ("LG SS Max_1", PM_LG.1), ("Alpha", primary_m_look(vis).3)] {
                assert!((slot(&c, m, k) - v).abs() < 1e-5, "{m} {k}: {} vs {v}", slot(&c, m, k));
            }
            assert_eq!(slot(&c, m, "Shine"), 0.); assert_eq!(slot(&c, m, "Rotation_1"), 0.);
            let cbm = &c["materials"][m]["cb"]; // cb[5].xy = (sin, cos)(Rotation_1 * 0.25) = (0, 1): the face gradient runs along TexCoord1.y
            assert!(cbm[5][0].as_f64().unwrap().abs() < 1e-9 && (cbm[5][1].as_f64().unwrap() - 1.).abs() < 1e-9, "{m} gradient direction");
            assert!((cbm[9][0].as_f64().unwrap() as f32 - PM_SIZE).abs() < 1e-6 && (cbm[9][1].as_f64().unwrap() as f32 - PM_SHADOW_DIST).abs() < 1e-6, "cb[9].xy = (Size, Drop Shadow Distance)");
        }
        // Gradient panels (M_UI_GradientPanel, gradientPanel.plain.asm)
        for (m, kind) in [("M-Border-MainModal", PanelKind::ModalPanel), ("M-Border-ModalHeader-procedural", PanelKind::ModalHeader)] {
            assert_eq!(slot(&c, m, "Rotation"), PANEL_ROTATION); assert_eq!(slot(&c, m, "Scale"), 1.); assert_eq!(slot(&c, m, "Position"), 0.);
            assert_eq!((slot(&c, m, "LG SS Min"), slot(&c, m, "LG SS Max")), PANEL_LG); assert_eq!(slot(&c, m, "Global Alpha"), PANEL_GLOBAL_ALPHA); assert_eq!(slot(&c, m, "Multiply Base"), 1.);
            let d = panel_direction(PANEL_ROTATION); let cb3 = &c["materials"][m]["cb"][3];
            assert!((cb3[0].as_f64().unwrap() as f32 - d[0]).abs() < 1e-6 && (cb3[1].as_f64().unwrap() as f32 - d[1]).abs() < 1e-6, "{m} cb[3].xy direction");
            let (c0, c1) = if kind == PanelKind::ModalPanel { (MODAL_RGBA0, MODAL_RGBA1) } else { (HEADER_RGBA0, HEADER_RGBA1) };
            for k in 0..3 { assert!((c["materials"][m]["slots"]["RGBA0"]["value"][k].as_f64().unwrap() as f32 - c0[k]).abs() < 1e-5 && (c["materials"][m]["slots"]["RGBA1"]["value"][k].as_f64().unwrap() as f32 - c1[k]).abs() < 1e-5); }
        }
        // ShellUberBG sphere permutation (shellUberBG.plain.asm)
        let m = "M-Border-ShellUberBG";
        for (k, v) in [("Sphere Gradient Pos U", UBER_POS.0), ("Sphere Gradient Pos V", UBER_POS.1), ("Sphere Gradient Radius", UBER_RADIUS), ("Sphere Gradient Density", UBER_DENSITY), ("Gradient Contrast", UBER_CONTRAST), ("Gradient Mult", UBER_MULT), ("Global Alpha", UBER_GLOBAL_ALPHA)] {
            assert!((slot(&c, m, k) - v).abs() < 1e-5, "{m} {k}");
        }
        for k in 0..4 { assert!((c["materials"][m]["slots"]["RGBA0"]["value"][k].as_f64().unwrap() as f32 - UBER_RGBA0[k]).abs() < 1e-5 && (c["materials"][m]["slots"]["RGBA1"]["value"][k].as_f64().unwrap() as f32 - UBER_RGBA1[k]).abs() < 1e-5, "{m} RGBA"); }
    }
    #[test] fn outline_buttons_are_drawn_like_the_compiled_material() {
        use super::super::outline::*;
        // Slate box brush margin 0.1 x ImageSize 48 = 4.8 px bands carry UV 0..0.1; the outline band is 0.5 - 0.433 = 0.067 of UV = 3.2 px, so
        // pixel centres 0.5, 1.5, 2.5 are band and 3.5 is not (the old approximation drew a 1.5 unit outline).
        let r = Rect::new(100., 40., 288., 48.);
        let fills = expand(ProcStyle::Outline, Vis::Normal, r, 1., 1.);
        let at = |x: f32, y: f32| fills.iter().rev().find_map(|c| if let Cmd::Fill { rect, color } = c { if x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h { Some(*color) } else { None } } else { None });
        let l = look(ProcStyle::Outline, Vis::Normal); let prim = lin_to_srgb(l.primary);
        for (x, band) in [(100.5, true), (102.5, true), (103.5, false), (200.5, false), (385.5, true), (387.5, true), (384.5, false)] {
            let c = at(x, 60.5).unwrap(); assert_eq!(c[3] > 0.7, band, "x {x}: {c:?}");
            if band { assert!((0..3).all(|k| (c[k] - prim[k]).abs() < 0.01) && (c[3] - l.outline_alpha).abs() < 0.01, "band = Primary at Outline Alpha: {c:?}"); }
        }
        for y in [40.5, 41.5, 42.5, 85.5, 86.5, 87.5] { assert!(at(200.5, y).unwrap()[3] > 0.7, "top/bottom band rows {y}"); }
        assert!(at(200.5, 43.5).unwrap()[3] < 0.5 && at(200.5, 84.5).unwrap()[3] < 0.5, "interior rows are the 0.4 * gradient fill");
        // interior alpha = 1 * (1 - s*(1-MinAlpha)) * GlobalAlpha: just below the top band (v = 3.5/48) ~ 0.4, bottom row (v = 44.5/48) ~ 0.3
        let top = at(200.5, 43.5).unwrap()[3]; assert!((top - 0.4 * (1. - smooth01_t(3.5 / 48.) * 0.25)).abs() < 0.01, "{top}");
        let bot = at(200.5, 84.5).unwrap()[3]; assert!((bot - 0.4 * (1. - smooth01_t(44.5 / 48.) * 0.25)).abs() < 0.01 && bot < top, "{bot} {top}");
        // colour runs from Primary at the top to 60% of the way to Base at the bottom (Dark Base Alpha 0.6 * smoothstep)
        let ct = at(200.5, 43.5).unwrap(); let cbm = at(200.5, 84.5).unwrap(); assert!(ct[0] > cbm[0] + 0.1 && ct[2] > cbm[2], "{ct:?} {cbm:?}");
        // LeftShade (icon column): left half filled, right half only the outline band (u > Inner Fill Offset 0.5 -> fill = 1 - Switch Offset 1 = 0)
        let r2 = Rect::new(500., 40., 48., 48.);
        let f2 = expand(ProcStyle::OutlineLeftShade, Vis::Normal, r2, 1., 1.);
        let at2 = |x: f32, y: f32| f2.iter().rev().find_map(|c| if let Cmd::Fill { rect, color } = c { if x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h { Some(*color) } else { None } } else { None });
        assert!(at2(510.5, 64.5).is_some_and(|c| c[3] > 0.1), "left half filled"); assert!(at2(535.5, 64.5).is_none(), "right half interior is transparent (no fill command)");
        assert!(at2(547.5, 64.5).is_some_and(|c| c[3] > 0.7), "right outline band still drawn");
        // hover = yellow Primary, 100% outline alpha; pressed doubles the fill alpha (Gradient Global Alpha 0.8, Min Alpha 1)
        let hv = expand(ProcStyle::Outline, Vis::Hover, r, 1., 1.); let Cmd::Fill { color, .. } = hv[0] else { panic!() }; assert!(color[0] > 0.95 && color[1] > 0.95 && color[3] > 0.95, "{color:?}");
    }
    fn smooth01_t(x: f32) -> f32 { x * x * (3. - 2. * x) }
    #[test] fn primary_m_button_is_a_rounded_pill_with_a_shadow_lip() {
        use super::super::outline::*;
        // 160 x 48 (Primary-M MinWidth/MinHeight), brush margin 0.25 / 0.5 of 48: 12 px side bands and 24 px top/bottom bands.
        let r = Rect::new(0., 0., 160., 48.);
        let f = expand(ProcStyle::PrimaryM, Vis::Normal, r, 1., 1.);
        let at = |x: f32, y: f32| f.iter().rev().find_map(|c| if let Cmd::Fill { rect, color } = c { if x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h { Some(*color) } else { None } } else { None });
        assert!(at(0.5, 0.5).is_none(), "the corner pixel is outside the rounded face (alpha 0)"); assert!(at(80.5, 24.5).unwrap()[3] > 0.99, "centre opaque");
        let (rgba0, rgba1, shadow, _) = primary_m_look(Vis::Normal);
        // mid-height: colour = lerp(RGBA0, RGBA1, smoothstep(0.5) = 0.5); bottom lip (v >= 0.9) = Drop Shadow colour; top rows = (almost) RGBA0
        let mid = at(80.5, 24.5).unwrap(); let want: [f32; 3] = lin_to_srgb([0, 1, 2].map(|k| (rgba0[k] + rgba1[k]) * 0.5)); assert!((0..3).all(|k| (mid[k] - want[k]).abs() < 0.02), "{mid:?} vs {want:?}");
        let lip = at(80.5, 46.5).unwrap(); let ws = lin_to_srgb(shadow); assert!((0..3).all(|k| (lip[k] - ws[k]).abs() < 0.02), "bottom lip is the Drop Shadow colour: {lip:?} vs {ws:?}");
        let top = at(80.5, 3.5).unwrap(); let w0 = lin_to_srgb(rgba0); assert!((0..3).all(|k| (top[k] - w0[k]).abs() < 0.05), "{top:?} vs {w0:?}");
        assert!(at(80.5, 14.).unwrap()[3] > 0.99 && (at(80.5, 40.5).unwrap()[0] - at(80.5, 14.5).unwrap()[0]).abs() > 0.02, "gradient continues below the middle");
        // disabled: material Alpha 0.6
        let d = expand(ProcStyle::PrimaryM, Vis::Disabled, r, 1., 1.);
        let color = d.iter().find_map(|c| if let Cmd::Fill { rect, color } = c { if 80.5 >= rect.x && 80.5 < rect.x + rect.w && 24.5 >= rect.y && 24.5 < rect.y + rect.h { Some(*color) } else { None } } else { None }).unwrap();
        assert!((color[3] - 0.6).abs() < 0.01, "{color:?}");
    }
    #[test] fn layout_constants_match_the_widget_tree() {
        let w = json("menu-widgets.json");
        let tree = &w["widgets"]["/Game/UI/Frontend/MainMenu/MainMenu"]["tree"];
        fn find<'a>(n: &'a Value, name: &str) -> Option<&'a Value> { if n["name"] == name { return Some(n); } n["children"].as_array()?.iter().find_map(|c| find(c, name)) }
        let num = |n: &str, k: &str| find(tree, n).unwrap()["props"][k].as_f64().unwrap() as f32;
        // 1.8: MainMenu's root Border_0 has BrushColor 0,0,0,0.5 but its Background is NoDrawType, so NOTHING is drawn (no dim); the backdrop is
        // TopBar.MainMenuBorder (Border_ShellUberBG) behind a 92-unit spacer, hosted by AthenaHUD (reports/menu-hosts.json).
        let b0 = find(tree, "Border_0").unwrap(); assert_eq!(b0["props"]["Background"]["drawAs"], "NoDrawType"); assert_eq!(b0["props"]["BrushColor"][3].as_f64().unwrap() as f32, 0.5);
        let hosts = json("menu-hosts.json");
        assert_eq!(hosts["topBar"]["topSpacer"]["props"]["Size"][1].as_f64().unwrap() as f32, TOP_INSET, "TopBar Spacer_1 above MainMenu");
        assert!(hosts["topBar"]["backdrop"]["props"]["Style"]["path"].as_str().unwrap().contains("Border_ShellUberBG"));
        assert_eq!(hosts["topBar"]["blur"]["props"]["BlurStrength"].as_f64().unwrap(), 16.);
        assert!(hosts["referencedBy"].as_array().unwrap().iter().any(|h| h == "/Game/Athena/HUD/AthenaHUD") && !hosts["referencedBy"].as_array().unwrap().iter().any(|h| h.as_str().unwrap().contains("AthenaHUDMenu")), "1.8 host is AthenaHUD -> TopBar, AthenaHUDMenu does not exist");
        assert_eq!(hosts["lightbox"]["dimBorder"]["props"]["BrushColor"][3].as_f64().unwrap() as f32, DLG_DIM);
        assert!(hosts["lightbox"]["dimBorder"]["props"].get("Background").is_none(), "Lightbox.Border_0 keeps the default drawn brush");
        let sp = |n: &str| find(tree, n).unwrap()["props"]["Size"][0].as_f64().unwrap() as f32;
        assert_eq!((sp("Spacer_1"), sp("Spacer_5"), sp("Spacer_8"), sp("Spacer_2")), (SPACER_LEAD, GAP, GAP, SPACER_RIGHT));
        assert_eq!(num("SizeBox_1", "MinDesiredWidth"), LEFT_W);
        assert_eq!(num("SizeBox_IconButtonList", "WidthOverride"), ICON_COL_W_PAD, "tree default; mouse/keyboard use 48 per `Set Icon Button List Column Width`");
        assert_eq!(find(tree, "LineSeparator").unwrap()["props"]["Brush"]["size"][0].as_f64().unwrap() as f32, SEPARATOR_W);
        assert_eq!(find(tree, "SizeBox_0").unwrap()["props"]["MinDesiredHeight"].as_f64().unwrap() as f32, HELP_MIN_H);
        assert_eq!(find(tree, "ContextualHelpText").unwrap()["props"]["WrapTextAt"].as_f64().unwrap() as f32, HELP_W);
        assert_eq!(find(tree, "SizeBox_0").unwrap()["slot"]["Padding"][0].as_f64().unwrap() as f32, 32.);
        assert_eq!(find(tree, "SizeBox_0").unwrap()["slot"]["Padding"][1].as_f64().unwrap() as f32, HELP_PAD_TOP);
        let itb = &w["widgets"]["/Game/UI/Foundation/Buttons/IconTextButton"]["tree"]; // MinHeight 48 comes from the style
        let _ = itb;
        let style = &w["styles"]["/Game/UI/Foundation/Buttons/ButtonStyles/TopBar/ButtonStyle-Outline"]["props"];
        assert_eq!(style["MinHeight"].as_f64().unwrap() as f32, BUTTON_H);
        let cd = json("menu-layout-pause.json");
        let c = &cd["confirmationDialogWidget"]; assert!(c["package"].as_str().unwrap().ends_with("ConfirmationWindow"));
        let cw = &w["widgets"]["/Game/UI/Foundation/Modals/ConfirmationWindow/ConfirmationWindow"]["tree"];
        assert_eq!(find(cw, "SizeBox_0").unwrap()["props"]["WidthOverride"].as_f64().unwrap() as f32, DLG_W);
        assert_eq!(find(cw, "SizeBox_0").unwrap()["props"]["MinDesiredHeight"].as_f64().unwrap() as f32, DLG_MIN_H);
        assert_eq!(find(cw, "SizeBox_0").unwrap()["slot"]["Padding"][1].as_f64().unwrap() as f32, DLG_V_MARGIN);
        assert_eq!(find(cw, "Spacer_0").unwrap()["props"]["Size"][1].as_f64().unwrap() as f32, DLG_HEADER_H);
        assert_eq!(find(cw, "ButtonBox").unwrap()["slot"]["Padding"][0].as_f64().unwrap() as f32, DLG_BTN_BOX);
        assert_eq!(find(cw, "TextItemContent").unwrap()["slot"]["Padding"][0].as_f64().unwrap() as f32, DLG_SIDE);
        assert_eq!(find(cw, "DescriptionScroll").unwrap()["slot"]["Padding"][1].as_f64().unwrap() as f32, DLG_DESC_TOP);
        assert_eq!(find(cw, "Title").unwrap()["slot"]["Padding"][0].as_f64().unwrap() as f32 + 20., DLG_TITLE_X);
        let pm = &w["styles"]["/Game/UI/Foundation/Buttons/ButtonStyles/Primary/ButtonStyle-Primary-M"]["props"];
        assert_eq!((pm["MinWidth"].as_f64().unwrap() as f32, pm["MinHeight"].as_f64().unwrap() as f32), (DLG_BTN_MIN_W, DLG_BTN_H));
        assert_eq!(w["widgets"]["/Game/UI/Foundation/Modals/ConfirmationWindow/ConfirmationWindow"]["classDefaults"]["ButtonBoxPadding"][0].as_f64().unwrap() as f32, DLG_BTN_PAD);
    }
    #[test] fn geometry_at_1080p_follows_the_recovered_columns() {
        let vp = Viewport::new(1920., 1080.); assert!((vp.scale - 1.).abs() < 1e-6);
        let l = layout(1920., 1080., 1., MenuContext::Match, false);
        let fb = l.rect_of("Feedback").unwrap(); let st = l.rect_of("SettingsButton").unwrap();
        assert_eq!((fb.w, fb.h), (288., 48.)); assert_eq!((st.w, st.h), (48., 48.));
        assert_eq!(st.x + st.w + 16., 1920., "16-unit right spacer"); assert_eq!(fb.x, 1920. - 16. - 48. - 14. - 4. - 14. - 288.);
        assert_eq!(l.sep.x, fb.x + fb.w + 14.); assert_eq!(st.x, l.sep.x + 4. + 14.); assert_eq!(l.sep.w, 4.);
        assert_eq!(fb.y, TOP_INSET + 8.); assert_eq!(st.y, fb.y, "both columns start at the same y");
        let rows: Vec<f32> = ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "LeaveButton"].iter().map(|n| l.rect_of(n).unwrap().y).collect();
        assert_eq!(rows, [fb.y, fb.y + 56., fb.y + 112., fb.y + 168.], "48 high, 8 slot padding");
        assert_eq!(l.sep.y, TOP_INSET + 8.); assert_eq!(l.sep.h, 4. * 56. - 8., "separator spans the taller (text) column");
        assert_eq!((l.help.x, l.help.w, l.help.h), (fb.x, 360., 64.)); assert_eq!(l.help.y, TOP_INSET + 4. * 56. + 8.);
        let w = layout(1920., 1080., 1., MenuContext::Match, true);
        assert_eq!(w.rect_of("SettingsButton").unwrap().w, 90.); assert_eq!(w.rect_of("Feedback").unwrap().x, 1920. - 16. - 90. - 14. - 4. - 14. - 288.);
    }
    #[test] fn the_backdrop_is_the_shell_uber_bg_panel_and_there_is_no_dim() {
        // Spec change vs 1.10: MainMenu.Border_0 is NoDrawType (draws nothing), so the old "50% black dim" Fill is gone; the first command is the
        // ShellUberBG backdrop (TopBar.MainMenuBorder) over the whole client, scaled by the menu fade.
        let mut h = H::new(1280., 720.);
        let c = h.frame();
        match &c[0] { Cmd::Panel { kind, rect, alpha } => { assert_eq!(*kind, PanelKind::Backdrop); assert_eq!((rect.x, rect.y, rect.w, rect.h), (0., 0., 1280., 720.)); assert_eq!(*alpha, 1.); } o => panic!("first command is the backdrop, got {o:?}") }
        assert!(!c.iter().any(|c| matches!(c, Cmd::Fill { color, rect } if color[0] == 0. && color[1] == 0. && color[2] == 0. && (color[3] - 0.5).abs() < 1e-6 && rect.w >= 1280.)), "no full-screen 50% black dim");
        let mut m = MenuSystem::new(fonts()); m.open(); m.fade_time = 1.; let cmds = m.frame(1280., 720., 0.5).cmds.clone();
        assert!(matches!(cmds[0], Cmd::Panel { alpha, .. } if (alpha - 0.5).abs() < 1e-5), "the backdrop fades with the menu");
    }
    #[test] fn backdrop_pixels_follow_the_compiled_shell_uber_bg_shader() {
        // Hand evaluation of shellUberBG.plain.asm: far from the glow centre (1, 2.2) (top-left, d = 2.42 > Radius 2) t = ~0 -> RGBA0 at alpha 1;
        // bottom-right corner (d = 1.2): n = 0.4, v = 1 - exp(-1.4427 * 0.16) = 0.2061, t = v^0.54317 * 1.886526 = 0.800.
        let tl = panel_pixel(PanelKind::Backdrop, 0.001, 0.001);
        let rgba0 = lin_to_srgb([UBER_RGBA0[0], UBER_RGBA0[1], UBER_RGBA0[2]]);
        assert!((0..3).all(|k| (tl[k] - rgba0[k]).abs() < 1e-3) && (tl[3] - 1.).abs() < 1e-3, "{tl:?}");
        let br = panel_pixel(PanelKind::Backdrop, 0.999, 0.999);
        let t = (1. - (-1.442695f32 * 0.16).exp()).powf(0.54317) * 1.886526; assert!((t - 0.800).abs() < 3e-3, "{t}");
        assert!((br[3] - (1. + (0.6 - 1.) * t)).abs() < 5e-3, "alpha lerps RGBA0.a 1 -> RGBA1.a 0.6 with t: {br:?}");
        assert!(br[2] > tl[2] + 0.05, "the glow is bluer than the base navy");
        // the whole panel is at least 60% opaque: the scene is barely visible behind the Esc menu
        let bm = panel_bitmap(PanelKind::Backdrop, 64, 36); assert!(bm.chunks(4).all(|p| p[3] >= 152), "min alpha {}", bm.chunks(4).map(|p| p[3]).min().unwrap());
    }
    #[test] fn gamepad_input_widens_the_icon_column_and_mouse_restores_it() {
        let mut h = H::new(1920., 1080.);
        let w0 = entry_rect(1920., 1080., MenuContext::Match, "SettingsButton").unwrap().w; assert_eq!(w0, 48.);
        let icon_w = |h: &mut H| { let cmds = h.frame(); let r = entry_rect(1920., 1080., MenuContext::Match, "SettingsButton").unwrap(); let _ = r; cmds.iter().filter_map(|c| if let Cmd::Procedural { style, rect, .. } = c { if *style == ProcStyle::OutlineLeftShade { Some(rect.w) } else { None } } else { None }).next().unwrap() };
        assert_eq!(icon_w(&mut h), 48.);
        h.ev(Event::Pad(PadButton::DPadDown)); assert_eq!(icon_w(&mut h), 90., "gamepad: SizeBox width override 90");
        h.mv((5., 5.)); h.mv((9., 9.)); assert_eq!(icon_w(&mut h), 48., "mouse movement returns to 48");
    }
    #[test] fn description_box_follows_hover_focus_and_clears_when_unhovered() {
        let mut h = H::new(1920., 1080.);
        assert!(!h.texts().iter().any(|t| t.contains("Adjust game settings")), "empty while nothing is hovered");
        let p = h.at("SettingsButton"); h.mv(p);
        let t = h.texts(); let joined = t.join(" ");
        assert!(joined.contains("Settings. Adjust game settings such as video, audio, and controls"), "{t:?}");
        assert!(joined.contains("Settings. Adjust game settings"), "Settings has its original description");
        let p = h.at("Feedback"); h.mv(p); let t = h.texts(); let joined = t.join(" ");
        assert!(joined.contains("Feedback. Send feedback about bugs or game experiences.") && !joined.contains("Not available"), "disabled entry: original description only: {t:?}");
        let p = h.at("CodeOfConduct"); h.mv(p); let t = h.texts(); assert!(!t.iter().any(|x| x.contains("Not available")), "no custom text: {t:?}");
        let p = h.at("LeaveButton"); h.mv(p); let t = h.texts();
        assert_eq!(t.iter().filter(|x| *x == "Leave Match").count(), 2, "LeaveGameHovered shows the button's own label in the description box (plus the button text itself): {t:?}");
        let p = h.at("AthenaPartyPrivacyButton"); h.mv(p); let t = h.texts().join(" "); assert!(t.contains("Change your game privacy setting to allow others to join your game."), "Athena privacy hover = PrivacyButton's text: {t}");
        h.mv((5., 5.)); assert!(!h.texts().iter().any(|t| t.contains("Adjust")), "OnUnhovered clears it");
        // keyboard focus drives it as well (CommonUI fires hover on focus)
        h.ev(Event::Nav(NavKey::Down)); let t = h.texts().join(" "); assert!(t.contains("Legal Information. View EULA and 3rd party software usage"), "{t}");
    }
    #[test] fn description_text_uses_the_blue_gray_style_wrapped_at_360() {
        let mut h = H::new(1920., 1080.); let p = h.at("Feedback"); h.mv(p);
        let help = layout(1920., 1080., 1., MenuContext::Match, false).help;
        let f = fonts();
        let lines: Vec<(f32, f32, [f32; 4], FontId, u32)> = h.frame().iter().filter_map(|c| if let Cmd::Text { x, top, text, color, font, px, .. } = c { if *x >= help.x - 0.5 && *top >= help.y - 0.5 { Some((*x + f.measure(*font, text, *px as f32), *top, *color, *font, *px)) } else { None } } else { None }).collect();
        assert!(lines.len() >= 2, "description wraps over several lines: {lines:?}");
        for (right, _, color, font, px) in &lines { assert!(*right <= help.x + 360. + 0.5, "wrapped at 360 units"); assert_eq!((*font, *px), (FontId::Regular, 16)); assert!((color[0] - super::super::outline::srgb(0.520996)).abs() < 1e-4 && (color[3] - 1.).abs() < 1e-6, "{color:?}"); }
    }
    #[test] fn unavailable_entries_do_nothing_and_are_marked_disabled() {
        let mut h = H::new(1920., 1080.);
        for w in ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "Legal", "LogoutButton"] {
            let p = h.at(w); h.click(p);
            assert_eq!(h.m.stack_names(), ["pause"], "{w} must not open anything"); assert!(h.m.take_requests().is_empty(), "{w}");
            let r = entry_rect(1920., 1080., MenuContext::Match, w).unwrap();
            let v = h.frame().iter().find_map(|c| if let Cmd::Procedural { vis, rect, .. } = c { if (rect.x - r.x).abs() < 0.5 && (rect.y - r.y).abs() < 0.5 { Some(*vis) } else { None } } else { None }).unwrap();
            assert_eq!(v, Vis::Disabled, "{w} drawn with the original Disabled material");
        }
        assert!(h.m.is_open());
    }
    #[test] fn leave_match_dialog_uses_the_original_strings_and_burbank_title() {
        let s = json("menu-strings.json");
        let get = |id: &str| s["strings"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap()["en"].as_str().unwrap().to_owned();
        let (title, msg) = (get("pause.dialog.LeaveButton.5.title"), get("pause.dialog.LeaveButton.5.message.false"));
        let mut h = H::new(1920., 1080.); let p = h.at("LeaveButton"); h.click(p);
        let c = h.frame();
        let texts: Vec<&str> = c.iter().filter_map(|c| if let Cmd::Text { text, .. } = c { Some(text.as_str()) } else { None }).collect();
        assert!(texts.contains(&title.as_str()) && texts.iter().any(|t| msg.starts_with(t) || t.starts_with("Leave the match return")) , "{texts:?}");
        assert!(texts.contains(&"Leave") && texts.contains(&"Cancel"), "{texts:?}");
        assert!(c.iter().any(|c| matches!(c, Cmd::Text { text, font: FontId::Title, px: 48, .. } if *text == title)), "TextStyle-Header-L: Burbank 48");
        assert!(c.iter().any(|c| matches!(c, Cmd::Text { text, font: FontId::Bold, px: 18, .. } if text == "Leave")), "Primary-M text: NotoSans Bold 18");
        assert!(texts.iter().any(|t| t.contains("no lobby")), "the runtime note states what Leave does here");
        let p = procs_of(&c); assert_eq!(p.iter().filter(|x| **x == ProcStyle::PrimaryM).count(), 2, "two Primary-M buttons");
    }
    fn procs_of(c: &[Cmd]) -> Vec<ProcStyle> { c.iter().filter_map(|c| if let Cmd::Procedural { style, .. } = c { Some(*style) } else { None }).collect() }
    #[test] fn confirmation_window_geometry_matches_the_recovered_sizes() {
        let (panel, header, body, yes, no) = confirm_layout(1920., 1080., 1., 60.);
        assert_eq!((panel.w, panel.h, header.h), (1000., 600., 100.)); assert_eq!((panel.x, panel.y), (460., 240.), "centred");
        assert_eq!((yes.w, yes.h, no.w, no.h), (160., 48., 160., 48.));
        assert_eq!(no.x + no.w, panel.x + panel.w - 20. - 4., "ButtonBox padding 20 + button padding 4"); assert_eq!(yes.x + yes.w + 8., no.x, "4+4 between buttons");
        assert_eq!(no.y + no.h, panel.y + panel.h - 24.); assert_eq!((body.x, body.y), (panel.x + 20., panel.y + 120.));
        // viewport padding 200 top/bottom caps tall dialogs; tiny viewports still fit
        let (p, ..) = confirm_layout(1920., 1080., 1., 900.); assert!(p.h <= 1080. - 16. + 0.01 && p.h >= 600.);
        for (w, h) in SIZES.into_iter().chain([(300., 200.)]) {
            let vp = Viewport::new(w, h); let (vw, vh) = vp.slate_size();
            let (panel, _, _, yes, no) = confirm_layout(vw, vh, vp.scale, 40.);
            for r in [panel, yes, no] { assert!(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= w + 0.01 && r.y + r.h <= h + 0.01, "{w}x{h} {r:?}"); }
            assert!(yes.x + yes.w <= no.x + 0.01);
        }
    }
    #[test] fn no_text_leaves_the_client_area_in_the_menu_or_the_dialog() {
        let f = fonts();
        for (w, h_) in SIZES.into_iter().chain([(300., 200.), (200., 600.)]) {
            let mut h = H::new(w, h_);
            for step in 0..2 {
                if step == 1 { let p = h.at("LeaveButton"); h.click(p); }
                let hover = h.at("Feedback"); if step == 0 { h.mv(hover); }
                for c in h.frame() { if let Cmd::Text { x, top, box_h, text, font, px, .. } = c {
                    let tw = f.measure(font, &text, px as f32);
                    assert!(x >= -0.5 && x + tw <= w + 0.5 && top >= -0.5 && top + box_h <= h_ + 0.5, "{w}x{h_} step {step}: {text:?} x {x} w {tw} top {top} box {box_h}");
                } }
            }
        }
    }
    #[test] fn procedural_expansion_is_contained_and_follows_the_fade() {
        let r = Rect::new(100., 50., 288., 48.);
        for style in [ProcStyle::Outline, ProcStyle::OutlineRed, ProcStyle::OutlineLeftShade, ProcStyle::OutlineLeftShadeRed, ProcStyle::PrimaryM] {
            for vis in [Vis::Normal, Vis::Hover, Vis::Pressed, Vis::Disabled] {
                let full = expand(style, vis, r, 1., 1.); let half = expand(style, vis, r, 1., 0.5);
                assert!(!full.is_empty() && full.len() == half.len());
                for (a, b) in full.iter().zip(&half) {
                    let (Cmd::Fill { rect: ra, color: ca }, Cmd::Fill { color: cb, .. }) = (a, b) else { panic!("only fills") };
                    assert!(ra.x >= r.x - 0.01 && ra.x + ra.w <= r.x + r.w + 0.01 && ra.y >= r.y - 0.01, "{style:?} inside horizontally");
                    assert!(ra.y + ra.h <= r.y + r.h + 0.01 + if style == ProcStyle::PrimaryM { 3. } else { 0. }, "{style:?} {vis:?} vertical extent");
                    assert!((ca[3] * 0.5 - cb[3]).abs() < 1e-5, "alpha scales with the menu fade");
                }
            }
        }
        // hover is the yellow of M-Button-Outline-Hovered: the outline fill is (almost) yellow, not blue
        let e = expand(ProcStyle::Outline, Vis::Hover, r, 1., 1.);
        let Cmd::Fill { color, .. } = e.last().unwrap() else { panic!() }; assert!(color[0] > 0.95 && color[1] > 0.95 && color[2] < 0.5, "{color:?}");
        let e = expand(ProcStyle::Outline, Vis::Normal, r, 1., 1.);
        let Cmd::Fill { color, .. } = e.last().unwrap() else { panic!() }; assert!(color[2] > 0.95 && color[0] < 0.7, "{color:?}");
    }
    #[test] fn clicking_or_activating_a_disabled_entry_shows_nothing_and_opens_nothing() {
        let mut h = H::new(1920., 1080.);
        let before = h.texts().len();
        for wid in ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "Legal", "LogoutButton"] { let p = h.at(wid); h.click(p); assert!(!h.texts().iter().any(|t| t.contains("Not available") || t.contains("not available")), "{wid}: no custom text: {:?}", h.texts()); assert_eq!(h.m.stack_names(), ["pause"], "{wid}: nothing opened"); h.mv((5., 5.)); }
        let _ = before;
        let p = h.at("SettingsButton"); h.click(p);
        assert_eq!(h.m.stack_names(), ["pause", "settings"]);
    }
    #[test] fn closing_the_leave_dialog_returns_focus_and_description_to_leave() {
        // QA: after Esc on the "Leave Match?" dialog the ring was on Leave but the description showed the Settings text.
        let mut h = H::new(1920., 1080.);
        let p = h.at("LeaveButton"); h.click(p);
        assert!(h.m.depth() == 2 && !h.m.stack_names().iter().any(|n| n.contains("settings")), "dialog open: {:?}", h.m.stack_names());
        h.ev(Event::Nav(NavKey::Back));
        assert!(h.m.depth() == 1, "dialog closed");
        let t = h.texts();
        assert!(!t.iter().any(|x| x.contains("Adjust game settings")), "Settings description must not show: {t:?}");
        assert_eq!(t.iter().filter(|x| *x == "Leave Match").count(), 2, "Leave description (its own label) follows the focus: {t:?}");
    }
    #[test] fn the_menu_covers_the_hud_flag_follows_open_and_fade() {
        let mut m = MenuSystem::new(fonts()); m.fade_time = 0.1;
        assert!(!(m.is_open() || m.visibility() > 0.));
        m.open(); m.frame(1280., 720., 1. / 60.);
        assert!(m.is_open() && m.visibility() > 0., "covering from the first frame of the open");
        m.settle(); m.close();
        assert!(m.visibility() > 0. && !m.is_open(), "still fading out: the HUD stays hidden until the dim is gone");
        for _ in 0..20 { m.frame(1280., 720., 1. / 60.); }
        assert!(m.visibility() == 0., "HUD returns after the fade-out");
    }
    #[test] fn tiny_clients_get_a_notice_instead_of_overlapping_widgets() {
        use super::super::{too_small, MIN_UI_SLATE};
        assert!(too_small(184., 81.) && too_small(256., 144.) && too_small(250., 120.));
        for (w, h) in [(640., 360.), (400., 300.), (640., 480.), (1024., 768.), (400., 225.)] { assert!(!too_small(w, h), "{w}x{h} is laid out normally"); }
        assert_eq!(MIN_UI_SLATE, (480., 400.));
        for (w, h_) in [(184., 81.), (256., 144.), (250., 120.), (100., 100.), (50., 20.), (1., 1.)] {
            let mut h = H::new(w, h_);
            let c = h.frame();
            if w >= 150. && h_ >= 60. { assert!(c.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text.contains("too small") || text.contains("Window"))), "{w}x{h_}: notice"); }
            assert!(!c.iter().any(|c| matches!(c, Cmd::Procedural { .. })), "{w}x{h_}: no menu buttons drawn (nothing to overlap)");
            h.ev(Event::Nav(NavKey::Back)); assert!(!h.m.is_open(), "{w}x{h_}: Esc still closes the root menu");
        }
    }
    #[test] fn srgb_conversion_is_the_standard_transfer() {
        use super::super::outline::srgb;
        assert_eq!(srgb(0.), 0.); assert!((srgb(1.) - 1.).abs() < 1e-6); assert!((srgb(0.5) - 0.735_357).abs() < 1e-4); assert!((srgb(0.002) - 0.025_84).abs() < 1e-4);
    }
}
