//! Tab-completion: command names (and aliases), then per command the argument kind of the registry (catalog items, cosmetic ids / display names, fixed words).
use crate::catalog::{norm, rank_text, Catalog, ItemKind};
use crate::exec::{CosmeticKind, CosmeticSource};
use crate::registry::{self, Arg, COMMANDS, LIST_TOPICS};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// text that replaces the token under the cursor
    pub insert: String,
    /// shown in the list (bold)
    pub label: String,
    pub hint: String,
}

/// Suggestions for a line: `token_start` = byte offset where the replaced token starts; `hint` = the usage still to type (ghost text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion { pub token_start: usize, pub items: Vec<Suggestion>, pub hint: String }

fn sug(insert: &str, label: &str, hint: &str) -> Suggestion { Suggestion { insert: insert.to_owned(), label: label.to_owned(), hint: hint.to_owned() } }
fn is_number(s: &str) -> bool { !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) }

fn command_items(q: &str) -> Vec<Suggestion> {
    let q = q.to_ascii_lowercase();
    let label = |c: &registry::Command| format!("{} {}", c.name, c.usage).trim_end().to_owned();
    let mut items: Vec<Suggestion> = COMMANDS.iter().filter(|c| c.name.starts_with(&q)).map(|c| sug(c.name, &label(c), c.help)).collect();
    for c in COMMANDS { for a in c.aliases { if a.starts_with(&q) && !a.is_empty() && *a != "?" { items.push(sug(a, &format!("{a}  (= {})", c.name), c.help)); } } }
    if items.is_empty() { items = COMMANDS.iter().filter(|c| c.name.contains(&q)).map(|c| sug(c.name, &label(c), c.help)).collect(); }
    items
}

fn cosmetic_items(cs: &dyn CosmeticSource, kind: CosmeticKind, q: &str) -> Vec<Suggestion> {
    let q = norm(q);
    let mut v: Vec<(u8, Suggestion)> = cs.list(kind).into_iter().filter_map(|e| {
        let r = match (rank_text(&norm(&e.id), &q), rank_text(&norm(&e.display_name), &q)) { (Some(a), Some(b)) => a.min(b), (a, b) => a.or(b)? };
        Some((r, sug(&e.id, &e.display_name, &format!("{}  {}", e.id, e.rarity))))
    }).collect();
    v.sort_by(|a, b| (a.0, &a.1.insert).cmp(&(b.0, &b.1.insert)));
    v.into_iter().map(|(_, s)| s).collect()
}

fn item_items(cat: &Catalog, q: &str, only_guns: bool) -> Vec<Suggestion> {
    cat.search(q, |i| !only_guns || matches!(i.kind, ItemKind::Gun(_))).into_iter().map(|i| sug(&i.id, &i.name, &format!("{}  {}", i.id, Catalog::describe(i).replacen(&format!("{} ", i.name), "", 1)))).collect()
}

/// The runtime weapons matching `q` (those that are also starter items are already suggested by `item_items` when `skip_items`).
fn weapon_items(cat: &Catalog, q: &str, skip_items: bool) -> Vec<Suggestion> {
    cat.weapon_search(q).into_iter().filter(|w| !(skip_items && cat.items.iter().any(|i| i.id == w.id)))
        .map(|w| sug(&w.id, w.name.as_deref().unwrap_or(&w.id), &format!("{}  {}", w.id, w.describe()))).collect()
}

pub fn complete(line: &str, cat: &Catalog, cs: &dyn CosmeticSource) -> Completion {
    let lead = line.len() - line.trim_start().len();
    let body = &line[lead..];
    let Some(sp_at) = body.find(char::is_whitespace) else {
        let items = command_items(body);
        let hint = registry::find(body).filter(|c| c.name.eq_ignore_ascii_case(body) && !c.usage.is_empty()).map(|c| c.usage.to_owned()).unwrap_or_default();
        return Completion { token_start: lead, items, hint };
    };
    let cmd = &body[..sp_at];
    let rest_start = lead + sp_at + body[sp_at..].len() - body[sp_at..].trim_start().len();
    let rest = &line[rest_start..];
    let none = |hint: String| Completion { token_start: rest_start, items: vec![], hint };
    let Some(c) = registry::find(cmd) else { return none(String::new()) };
    let usage_hint = if rest.trim().is_empty() { c.usage.to_owned() } else { String::new() };
    let words: Vec<&str> = rest.split_whitespace().collect();
    // an item / gun query followed by a number: the number is the count, nothing more to complete
    let counting = words.len() >= 2 && is_number(words[words.len() - 1]) || (c.name == "ammo" && words.len() == 1 && is_number(words[0]))
        || (c.name == "give" && words.len() >= 2 && words[words.len() - 1].eq_ignore_ascii_case("equip"));
    match c.arg {
        Arg::None => none(String::new()),
        Arg::Number => none(usage_hint),
        Arg::Command => Completion { token_start: rest_start, items: command_items(rest.trim()), hint: usage_hint },
        Arg::Values(vs) => { let q = rest.trim().to_ascii_lowercase(); Completion { token_start: rest_start, items: vs.iter().filter(|v| v.starts_with(&q)).map(|v| sug(v, v, "")).collect(), hint: usage_hint } }
        Arg::Topic => {
            if words.len() >= 2 || (words.len() == 1 && rest.ends_with(char::is_whitespace)) { return none(String::new()); }
            let q = rest.trim().to_ascii_lowercase();
            Completion { token_start: rest_start, items: LIST_TOPICS.iter().filter(|t| t.starts_with(&q)).map(|t| sug(t, t, "")).collect(), hint: usage_hint }
        }
        Arg::Item | Arg::Gun | Arg::Weapon => if counting { none(String::new()) } else {
            let q = rest.trim().trim_matches('"');
            let mut items = if c.arg == Arg::Weapon { vec![] } else { item_items(cat, q, c.arg == Arg::Gun) };
            if c.arg != Arg::Gun { items.extend(weapon_items(cat, q, c.arg != Arg::Weapon)); }
            Completion { token_start: rest_start, items, hint: usage_hint }
        }
        Arg::Skin | Arg::Pickaxe | Arg::Backbling | Arg::Glider => {
            let kind = match c.arg { Arg::Skin => CosmeticKind::Skin, Arg::Pickaxe => CosmeticKind::Pickaxe, Arg::Backbling => CosmeticKind::Backbling, _ => CosmeticKind::Glider };
            Completion { token_start: rest_start, items: cosmetic_items(cs, kind, rest.trim().trim_matches('"')), hint: usage_hint }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{test_catalog, test_weapons};
    use crate::exec::tests::FakeCosmetics;
    #[test] fn weapons_complete_for_give_and_equip() {
        let cat = test_weapons();
        let k = complete("give rock", &cat, &FakeCosmetics); assert_eq!(inserts(&k), ["WID_Launcher_Rocket_Athena_R_Ore_T03"]); assert_eq!(k.items[0].label, "Rocket Launcher"); assert!(k.items[0].hint.contains("Rare"));
        let k = complete("equip assault", &cat, &FakeCosmetics); assert_eq!(k.items.len(), 3, "every rarity is offered");
        assert!(complete("give pistol equip", &cat, &FakeCosmetics).items.is_empty());
        assert_eq!(complete("give ", &cat, &FakeCosmetics).items.len(), 8 + 4, "starter items + the weapons that are not starters");
        assert!(complete("equip ", &test_catalog(), &FakeCosmetics).items.is_empty());
    }
    fn c(line: &str) -> Completion { complete(line, &test_catalog(), &FakeCosmetics) }
    fn inserts(c: &Completion) -> Vec<&str> { c.items.iter().map(|s| s.insert.as_str()).collect() }

    #[test] fn command_names_complete_by_prefix_and_aliases_show_their_target() {
        let k = c("gi"); assert!(inserts(&k).contains(&"give") && inserts(&k).contains(&"giveathenafullkit")); assert_eq!(k.token_start, 0);
        let k = c("toggleunl"); assert_eq!(k.items[0].insert, "toggleunlimitedhealth"); assert!(k.items[0].label.contains("= god"));
        assert_eq!(c("  sk").token_start, 2);
        assert!(inserts(&c("xyzzy")).is_empty());
        assert_eq!(c("ammo").hint, "[gun] [count]", "an exact command shows its usage");
    }
    #[test] fn item_arguments_complete_ids_and_display_names() {
        let k = c("give pu"); assert_eq!(inserts(&k), ["WID_Shotgun_Standard_Athena_C_Ore_T03"]); assert_eq!(k.token_start, 5); assert_eq!(k.items[0].label, "Pump Shotgun");
        assert!(k.items[0].hint.contains("Common"), "{:?}", k.items[0]);
        assert_eq!(inserts(&c("give band")), ["Athena_Bandage"]);
        assert!(c("give ").items.len() == 8 && c("give ").hint == "<item|weapon> [count] [equip]");
        assert!(c("give pump shotgun 2").items.is_empty(), "after the count nothing completes");
        assert_eq!(inserts(&c("ammo sh")), ["WID_Shotgun_Standard_Athena_C_Ore_T03"]); assert!(c("ammo ").items.iter().all(|s| s.insert.starts_with("WID_")), "guns only");
        assert!(c("ammo 30").items.is_empty());
    }
    #[test] fn cosmetic_arguments_complete_from_the_source() {
        let k = c("skin alp"); assert_eq!(inserts(&k), ["CID_001", "CID_003"]); assert_eq!(k.items[0].label, "Alpha Skin"); assert!(k.items[0].hint.contains("Rare"));
        assert_eq!(inserts(&c("pickaxe ")), ["Pickaxe_ID_001"]); assert!(c("glider ").items.is_empty());
        assert_eq!(inserts(&c("skin CID_002")), ["CID_002"]);
    }
    #[test] fn fixed_words_and_topics() {
        assert_eq!(inserts(&c("list s")), ["skins"]); assert!(inserts(&c("list ")).contains(&"weapons")); assert!(c("list skins al").items.is_empty());
        assert_eq!(inserts(&c("god o")), ["on", "off"]); assert_eq!(inserts(&c("help gi")).first(), Some(&"give"));
        assert_eq!(c("speed ").hint, "[scale]");
    }
}
