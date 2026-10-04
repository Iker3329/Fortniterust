//! Plain enums shared by the draw list and the CPU/GPU material evaluators (kept out of `menu` so backends do not depend on the toolkit).

/// Which material family a procedural box imitates.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ProcStyle {
    /// `ButtonStyle-Outline` (text buttons of the left column).
    Outline,
    /// `ButtonStyle-Outline-Red` (Leave button, Exit icon button's text twin).
    OutlineRed,
    /// `ButtonStyle-OutlineLeftShade` (right-column icon buttons).
    OutlineLeftShade,
    /// `ButtonStyle-Outline-LeftShade-Red` (Exit icon button).
    OutlineLeftShadeRed,
    /// `ButtonStyle-Primary-M` (confirmation window buttons).
    PrimaryM,
}
impl ProcStyle {
    pub fn left_shade(self) -> bool { matches!(self, ProcStyle::OutlineLeftShade | ProcStyle::OutlineLeftShadeRed) }
    pub fn red(self) -> bool { matches!(self, ProcStyle::OutlineRed | ProcStyle::OutlineLeftShadeRed) }
}

/// Which gradient panel material a `Cmd::Panel` draws.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PanelKind {
    /// `M-Border-ShellUberBG` (TopBar.MainMenuBorder: the Esc menu backdrop, drawn over the whole screen).
    Backdrop,
    /// `M-Border-MainModal` (ConfirmationWindow body).
    ModalPanel,
    /// `M-Border-ModalHeader-procedural` (ConfirmationWindow title bar).
    ModalHeader,
}

/// Button/brush visual state of a procedural widget.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum Vis { Normal, Hover, Pressed, Disabled }
