use fn_console::Key;
use fn_ui::screens::frontend_cooked::{SignInHit, SignInState};

pub const MAX_NAME: usize = 16;

pub fn valid_name(s: &str) -> bool {
    let t = s.trim();
    let n = t.chars().count();
    (3..=MAX_NAME).contains(&n) && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_'))
}

#[derive(Default)]
pub struct NameEntry { pub st: SignInState, pub done: Option<String>, pressed: bool }

impl NameEntry {
    pub fn new() -> NameEntry { NameEntry::default() }

    fn field(&mut self) -> &mut String { if self.st.focus == 0 { &mut self.st.email } else { &mut self.st.password } }

    pub fn key(&mut self, k: Key) {
        if self.done.is_some() { return; }
        let caret = self.st.caret;
        match k {
            Key::Char(c) => {
                let limit = if self.st.focus == 0 { MAX_NAME } else { 64 };
                let f = self.field();
                if f.chars().count() >= limit { return; }
                let mut v: Vec<char> = f.chars().collect();
                v.insert(caret.min(v.len()), c);
                *f = v.into_iter().collect();
                self.st.caret = caret + 1;
            }
            Key::Backspace => {
                if caret == 0 { return; }
                let f = self.field();
                let mut v: Vec<char> = f.chars().collect();
                if caret <= v.len() { v.remove(caret - 1); }
                *f = v.into_iter().collect();
                self.st.caret = caret - 1;
            }
            Key::Delete => {
                let f = self.field();
                let mut v: Vec<char> = f.chars().collect();
                if caret < v.len() { v.remove(caret); }
                *f = v.into_iter().collect();
            }
            Key::Left => self.st.caret = caret.saturating_sub(1),
            Key::Right => { let n = self.field().chars().count(); self.st.caret = (caret + 1).min(n); }
            Key::Home => self.st.caret = 0,
            Key::End => self.st.caret = self.field().chars().count(),
            Key::Tab | Key::ShiftTab | Key::Up | Key::Down => self.focus(1 - self.st.focus.min(1)),
            Key::Enter => self.commit(),
            _ => {}
        }
        self.refresh();
    }

    fn focus(&mut self, f: u8) { self.st.focus = f; self.st.caret = self.field().chars().count(); }

    fn refresh(&mut self) { self.st.button_enabled = valid_name(&self.st.email); }

    pub fn commit(&mut self) {
        self.refresh();
        if self.st.button_enabled { self.done = Some(self.st.email.trim().to_owned()); }
    }

    pub fn pointer(&mut self, hit: Option<SignInHit>, down: bool) {
        self.st.button_hovered = hit == Some(SignInHit::SignIn);
        if down {
            match hit { Some(SignInHit::Email) => self.focus(0), Some(SignInHit::Password) => self.focus(1), _ => {} }
            self.pressed = hit == Some(SignInHit::SignIn);
        } else {
            if self.pressed && hit == Some(SignInHit::SignIn) { self.commit(); }
            self.pressed = false;
        }
        self.st.button_pressed = self.pressed && self.st.button_hovered;
        self.refresh();
    }

    pub fn hover(&mut self, hit: Option<SignInHit>) { self.st.button_hovered = hit == Some(SignInHit::SignIn); self.st.button_pressed = self.pressed && self.st.button_hovered; }
}
