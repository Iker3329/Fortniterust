pub const NAME_MIN: usize = 3;
pub const NAME_MAX: usize = 16;
pub const CHAT_MAX: usize = 120;

const SUBSTR: [&str; 33] = [
    "fuck", "shit", "cunt", "bitch", "whore", "slut", "pussy", "penis", "vagina", "porn", "rape",
    "niger", "niga", "nigr", "faget", "fagot", "retard", "trany", "kike", "chink", "wetback",
    "beaner", "raghead", "towelhead", "hitler", "nazi", "pedophil", "molest", "jizz", "twat", "wank", "asshole", "killyourself",
];
const WORD: [&str; 23] = ["fuk", "fck", "fag", "fags", "dick", "cock", "cum", "anal", "anus", "spic", "coon", "gok", "dyke", "paki", "kkk", "heil", "isis", "kys", "rapist", "jihad", "bastard", "pedo", "pedos"];
const TLDS: [&str; 24] = ["com", "net", "org", "gg", "io", "xyz", "ru", "lol", "tv", "me", "co", "app", "dev", "link", "ly", "uk", "de", "info", "biz", "site", "online", "shop", "store", "cc"];

fn leet(c: char) -> char {
    match c { '0' => 'o', '1' => 'i', '3' => 'e', '4' => 'a', '5' => 's', '7' => 't', '8' => 'b', '9' => 'g', '$' => 's', '@' => 'a', '!' => 'i', '|' => 'i', c => c }
}

fn dedupe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() { if !(c.is_ascii_lowercase() && out.ends_with(c)) { out.push(c); } }
    out
}

fn fold(s: &str) -> String {
    let mapped: String = s.chars().map(|c| { let c = leet(c.to_ascii_lowercase()); if c.is_ascii_lowercase() { c } else { ' ' } }).collect();
    dedupe(&mapped)
}

fn word_is_bad(folded_word: &str) -> bool {
    SUBSTR.iter().any(|w| folded_word.contains(&dedupe(w))) || WORD.iter().any(|w| folded_word == dedupe(w))
}

pub fn clean_name(raw: &str) -> Option<String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = name.chars().count();
    if !(NAME_MIN..=NAME_MAX).contains(&n) { return None; }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-')) { return None; }
    if !name.chars().any(|c| c.is_ascii_alphanumeric()) { return None; }
    let folded = fold(&name);
    let words: Vec<&str> = folded.split(' ').filter(|w| !w.is_empty()).collect();
    let joined: String = words.concat();
    if SUBSTR.iter().any(|w| joined.contains(&dedupe(w))) { return None; }
    if WORD.iter().any(|w| { let d = dedupe(w); words.contains(&d.as_str()) || joined == d }) { return None; }
    Some(name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatReject { Empty, TooLong, Url, Filter }

impl ChatReject {
    pub fn code(self) -> &'static str { match self { ChatReject::Empty => "empty", ChatReject::TooLong => "long", ChatReject::Url => "url", ChatReject::Filter => "filter" } }
}

pub fn has_url(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    if l.contains("://") || l.contains("www.") { return true; }
    l.split(|c: char| c.is_whitespace()).any(|w| {
        let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let mut parts = w.split('.');
        let first = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        !first.is_empty() && rest.last().map_or(false, |t| TLDS.contains(&t.trim_end_matches('/').split('/').next().unwrap_or(""))) && rest.iter().all(|p| !p.is_empty())
    })
}

pub fn clean_chat(raw: &str) -> Result<String, ChatReject> {
    let printable: String = raw.chars().filter(|c| (' '..='~').contains(c) && *c != '<' && *c != '>').collect();
    let text = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() { return Err(ChatReject::Empty); }
    if text.chars().count() > CHAT_MAX { return Err(ChatReject::TooLong); }
    if has_url(&text) { return Err(ChatReject::Url); }
    if text.split(' ').any(|w| fold(w).split(' ').any(|f| !f.is_empty() && word_is_bad(f))) { return Err(ChatReject::Filter); }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn names_follow_the_relay_rules() {
        assert_eq!(clean_name("  Brady   Two "), Some("Brady Two".into()));
        assert_eq!(clean_name("ab"), None);
        assert_eq!(clean_name("abcdefghijklmnopq"), None);
        assert_eq!(clean_name("bad<name>"), None);
        assert_eq!(clean_name("..."), None);
        assert_eq!(clean_name("sh1t_lord"), None);
        assert_eq!(clean_name("f u c k"), None);
        assert_eq!(clean_name("kys"), None);
        assert_eq!(clean_name("Dickens"), Some("Dickens".into()));
        assert_eq!(clean_name("Jonny_B.42"), Some("Jonny_B.42".into()));
    }
    #[test] fn chat_is_checked_word_by_word() {
        assert_eq!(clean_chat("this is hit"), Ok("this is hit".into()));
        assert_eq!(clean_chat("  gg   wp  "), Ok("gg wp".into()));
        assert_eq!(clean_chat("you are sh1t"), Err(ChatReject::Filter));
        assert_eq!(clean_chat("kys"), Err(ChatReject::Filter));
        assert_eq!(clean_chat("<Victim>red</>"), Ok("Victimred/".into()));
        assert_eq!(clean_chat("   "), Err(ChatReject::Empty));
        assert_eq!(clean_chat(&"a".repeat(121)), Err(ChatReject::TooLong));
        assert_eq!(clean_chat(&"a".repeat(120)).map(|s| s.len()), Ok(120));
    }
    #[test] fn urls_are_refused() {
        for t in ["go to https://x.y", "www.site", "join discord.gg/abc", "free vbucks at evil.com", "mysite.co.uk now"] { assert_eq!(clean_chat(t), Err(ChatReject::Url), "{t}"); }
        for t in ["ok.", "wait... what", "3.5 damage", "gg."] { assert!(clean_chat(t).is_ok(), "{t}"); }
    }
}
