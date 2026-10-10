use fn_console::Key;
use fn_net::chat::{clean_chat, clean_name, ChatReject, CHAT_MAX};
use std::collections::VecDeque;
use std::sync::Mutex;

#[cfg(target_arch = "wasm32")]
#[path = "../../../tools/web/fn-web/src/chat_web.rs"]
mod chat_web;

pub const PROMPT: &str = "Say: ";
pub const HELP: &str = "Commands: /setusername NAME, /join CODE, /code, /leave, /help";
const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

#[derive(Default)]
struct State { open: bool, buf: String, caret: usize, renames: Vec<String>, inbox: VecDeque<String> }
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn is_open() -> bool { with(|s| s.open) }
pub fn open() { with(|s| { s.open = true; s.buf.clear(); s.caret = 0; }); }
pub fn close() { with(|s| { s.open = false; s.buf.clear(); s.caret = 0; }); }
pub fn take_renames() -> Vec<String> { with(|s| std::mem::take(&mut s.renames)) }
pub fn push_inbound(json: String) { with(|s| { if s.inbox.len() >= 256 { s.inbox.pop_front(); } s.inbox.push_back(json); }); }

fn line(markup: String) { crate::kill_feed_hook::push_markup(&markup); }
fn info(text: &str) { line(text.to_owned()); }
fn error(text: &str) { line(format!("<Victim>{text}</>")); }
fn said(name: &str, text: &str, me: bool) { line(format!("<{}>{name}:</> {text}", if me { "LocalKiller" } else { "TeamKiller" })); }

fn local_name() -> String { crate::kill_feed_hook::local().name }

pub fn reject_text(r: ChatReject) -> &'static str {
    match r {
        ChatReject::Empty => "Nothing to send",
        ChatReject::TooLong => "Messages are at most 120 characters",
        ChatReject::Url => "Links are not allowed in chat",
        ChatReject::Filter => "That message is not allowed",
    }
}
fn reject_code_text(code: &str) -> &'static str {
    match code {
        "rate" => "You are sending messages too fast",
        "long" => reject_text(ChatReject::TooLong),
        "url" => reject_text(ChatReject::Url),
        "filter" => reject_text(ChatReject::Filter),
        "bad_name" => "That name is not allowed",
        _ => "The message was not sent",
    }
}

fn lobby_code() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    { return chat_web::lobby_code(); }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

fn send_control(json: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    { return chat_web::send_text(json); }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = json; false }
}

pub fn on_name_applied(name: &str) {
    if lobby_code().is_some() { send_control(&serde_json::json!({ "t": "name", "name": name }).to_string()); }
}

pub fn valid_code(code: &str) -> Option<String> {
    let c = code.trim().to_ascii_uppercase();
    (c.len() == 6 && c.chars().all(|x| CODE_ALPHABET.contains(x))).then_some(c)
}

pub fn submit(input: &str) {
    let t = input.trim();
    if t.is_empty() { return; }
    if let Some(cmd) = t.strip_prefix('/') { command(cmd); return; }
    match clean_chat(t) {
        Err(r) => error(reject_text(r)),
        Ok(text) => {
            if lobby_code().is_some() {
                if !send_control(&serde_json::json!({ "t": "chat", "text": text }).to_string()) { error("Not connected to the lobby"); }
            } else { said(&local_name(), &text, true); }
        }
    }
}

fn command(cmd: &str) {
    let (name, arg) = match cmd.split_once(char::is_whitespace) { Some((n, a)) => (n, a.trim()), None => (cmd, "") };
    match name.to_ascii_lowercase().as_str() {
        "help" => info(HELP),
        "setusername" => {
            if arg.is_empty() { error("Usage: /setusername NAME"); return; }
            match clean_name(arg) {
                Some(n) => { with(|s| s.renames.push(n.clone())); info(&format!("Display name set to {n}")); }
                None => error("Names are 3-16 letters, numbers, spaces, dots, dashes or underscores, and must be clean"),
            }
        }
        "code" => match lobby_code() { Some(c) => info(&format!("Lobby code: {c}")), None => error("You are not in a lobby") },
        "join" => {
            let Some(code) = valid_code(arg) else { error("Usage: /join CODE (6 characters)"); return };
            #[cfg(target_arch = "wasm32")]
            { chat_web::join(&code, &local_name()); info(&format!("Joining lobby {code}...")); }
            #[cfg(not(target_arch = "wasm32"))]
            { let _ = code; error("Joining a lobby needs the browser build"); }
        }
        "leave" => {
            if lobby_code().is_none() { error("You are not in a lobby"); return; }
            #[cfg(target_arch = "wasm32")]
            chat_web::leave();
            crate::mp_hook::push_text(r#"{"t":"end","reason":"left"}"#.to_owned());
            info("You left the lobby");
        }
        "" => error("Type /help for the commands"),
        other => error(&format!("Unknown command /{other}. Type /help")),
    }
}

fn inbound(json: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    let me = v.get("me").and_then(|x| x.as_bool()).unwrap_or(false);
    match s("t").as_str() {
        "chat" => {
            let name = s("name").replace(['<', '>'], "");
            let Ok(text) = clean_chat(&s("text")) else { return };
            if !name.is_empty() { said(&name, &text, me); }
        }
        "renamed" => if !me { info(&format!("{} is now {}", s("old").replace(['<', '>'], ""), s("name").replace(['<', '>'], ""))); },
        "chat_reject" => error(reject_code_text(&s("code"))),
        "sys" => { let t = s("text").replace(['<', '>'], ""); if v.get("err").and_then(|x| x.as_bool()).unwrap_or(false) { error(&t) } else { info(&t) } }
        _ => {}
    }
}

pub fn key(k: Key) {
    let submit_text = with(|s| {
        let caret = s.caret.min(s.buf.chars().count());
        match k {
            Key::Char(c) => {
                if s.buf.chars().count() >= CHAT_MAX + 40 { return None; }
                let mut v: Vec<char> = s.buf.chars().collect();
                v.insert(caret, c);
                s.buf = v.into_iter().collect();
                s.caret = caret + 1;
            }
            Key::Backspace => if caret > 0 { let mut v: Vec<char> = s.buf.chars().collect(); v.remove(caret - 1); s.buf = v.into_iter().collect(); s.caret = caret - 1; },
            Key::Delete => { let mut v: Vec<char> = s.buf.chars().collect(); if caret < v.len() { v.remove(caret); s.buf = v.into_iter().collect(); } }
            Key::Left => s.caret = caret.saturating_sub(1),
            Key::Right => s.caret = (caret + 1).min(s.buf.chars().count()),
            Key::Home => s.caret = 0,
            Key::End => s.caret = s.buf.chars().count(),
            Key::Escape => { s.open = false; s.buf.clear(); s.caret = 0; }
            Key::Enter => { s.open = false; s.caret = 0; return Some(std::mem::take(&mut s.buf)); }
            _ => {}
        }
        None
    });
    if let Some(t) = submit_text { submit(&t); }
}

pub fn decorate(lines: &mut Vec<String>, t: f32) {
    let inbox: Vec<String> = with(|s| s.inbox.drain(..).collect());
    for m in inbox { inbound(&m); }
    if let Some((buf, caret)) = with(|s| s.open.then(|| (s.buf.clone(), s.caret))) {
        let shown: String = buf.chars().filter(|c| *c != '<' && *c != '>').collect();
        let caret = caret.min(shown.chars().count());
        let mut v: Vec<char> = shown.chars().collect();
        v.insert(caret, if (t * 2.).fract() < 0.5 { '|' } else { ' ' });
        lines.push(format!("{PROMPT}{}", v.into_iter().collect::<String>()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn codes_follow_the_relay_alphabet() {
        assert_eq!(valid_code(" abc234 "), Some("ABC234".into()));
        assert_eq!(valid_code("ABCDE1"), None);
        assert_eq!(valid_code("ABCDEFG"), None);
    }
    #[test] fn commands_and_offline_chat_go_to_the_feed() {
        crate::kill_feed_hook::set_local_name("Tester");
        submit("/help");
        submit("/nope");
        let l = crate::kill_feed_hook::frame(0., Some(100.));
        assert!(l.iter().any(|m| m == HELP));
        assert!(l.iter().any(|m| m.contains("Unknown command /nope")));
        submit("/setusername ok_name");
        submit("/setusername sh1t");
        submit("hello there");
        submit("see evil.com");
        let l = crate::kill_feed_hook::frame(0., Some(100.));
        assert!(l.iter().any(|m| m == "<LocalKiller>Tester:</> hello there"));
        assert!(l.iter().any(|m| m.contains("Links are not allowed")));
        assert_eq!(take_renames(), vec!["ok_name".to_string()]);
    }
    #[test] fn typing_edits_and_enter_sends() {
        open();
        for c in "hi!".chars() { key(Key::Char(c)); }
        key(Key::Backspace);
        let mut l = vec![];
        decorate(&mut l, 0.);
        assert_eq!(l, vec!["Say: hi|".to_string()]);
        key(Key::Escape);
        assert!(!is_open());
    }
}
