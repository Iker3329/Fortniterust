use crate::draw::RenderList;
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::team_info::{font_px, slate_line, NOTO_BOLD_BBOX_Y, NOTO_REGULAR_BBOX_Y};
use crate::screens::{atlas_px, put_text, srgb3, text_width, Snap};
use crate::text::FontId;

pub const MAX_ENTRIES: usize = 5;
pub const ENTRY_LIFETIME_S: f32 = 6.;
pub const FONT_PT: f32 = 12.;
pub const BORDER_COLOR: [f32; 4] = [0., 0., 0., 0.33];
pub const BORDER_PAD: (f32, f32) = (4., 2.);
pub const FEED_MIN_SIZE: (f32, f32) = (600., 150.);
pub const FEED_PAD_LEFT: f32 = 50.;
pub const BOTTOM_LEFT_PAD_BOTTOM: f32 = 40.;
pub const WATCHERS_PAD_TOP: f32 = 50.;
pub const WATCHERS_H: f32 = 32.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style { Default, Victim, Killer, LocalKiller, TeamKiller, TeamVictim }

impl Style {
    pub const ALL: [Style; 6] = [Style::Default, Style::Victim, Style::Killer, Style::LocalKiller, Style::TeamKiller, Style::TeamVictim];
    pub fn name(self) -> &'static str {
        match self { Style::Default => "Default", Style::Victim => "Victim", Style::Killer => "Killer", Style::LocalKiller => "LocalKiller", Style::TeamKiller => "TeamKiller", Style::TeamVictim => "TeamVictim" }
    }
    pub fn from_name(n: &str) -> Style { Style::ALL.into_iter().find(|s| s.name() == n).unwrap_or(Style::Default) }
    pub fn bold(self) -> bool { !matches!(self, Style::Default | Style::Killer) }
    pub fn font(self) -> FontId { if self.bold() { FontId::Bold } else { FontId::Regular } }
    pub fn linear_color(self) -> [f32; 3] {
        match self {
            Style::Default | Style::Killer => [1., 1., 1.],
            Style::Victim => [0.799103, 0.072272, 0.045186],
            Style::LocalKiller => [0.027321, 0.603828, 0.165132],
            Style::TeamKiller => [0.03434, 0.313989, 0.708376],
            Style::TeamVictim => [0.791298, 0.208637, 0.015996],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DeathCause { OutsideSafeZone = 0, FallDamage = 1, Pistol = 2, Shotgun = 3, Rifle = 4, Smg = 5, Sniper = 6, Melee = 7, Heavy = 8, DbnoTimeout = 9, Banhammer = 10, RemovedFromGame = 11, Unspecified = 12 }

impl DeathCause {
    pub fn from_weapon_tag(tag: &str) -> DeathCause {
        match tag.to_ascii_lowercase().as_str() {
            "weapon.melee" => DeathCause::Melee,
            "weapon.ranged.pistol" => DeathCause::Pistol,
            "weapon.ranged.shotgun" => DeathCause::Shotgun,
            "weapon.ranged.assault" => DeathCause::Rifle,
            "weapon.ranged.sniper" => DeathCause::Sniper,
            "weapon.ranged.smg" => DeathCause::Smg,
            "weapon.ranged.heavy" => DeathCause::Heavy,
            _ => DeathCause::Unspecified,
        }
    }
    pub fn from_weapon_row(row: &str) -> DeathCause {
        let r = row.to_ascii_lowercase();
        if r.starts_with("pickaxe") || r.contains("melee") { DeathCause::Melee }
        else if r.contains("autoheavy") || r.contains("smg") { DeathCause::Smg }
        else if r.starts_with("pistol") { DeathCause::Pistol }
        else if r.starts_with("shotgun") { DeathCause::Shotgun }
        else if r.starts_with("assault") { DeathCause::Rifle }
        else if r.starts_with("sniper") { DeathCause::Sniper }
        else if r.starts_with("launcher") || r.contains("rocket") || r.contains("grenade") { DeathCause::Heavy }
        else { DeathCause::Unspecified }
    }
    pub fn parse(s: &str) -> Option<DeathCause> {
        Some(match s.to_ascii_lowercase().as_str() {
            "storm" | "outsidesafezone" => DeathCause::OutsideSafeZone, "fall" | "falldamage" => DeathCause::FallDamage, "pistol" => DeathCause::Pistol, "shotgun" => DeathCause::Shotgun,
            "rifle" | "assault" => DeathCause::Rifle, "smg" => DeathCause::Smg, "sniper" => DeathCause::Sniper, "melee" | "pickaxe" => DeathCause::Melee, "heavy" | "explosive" | "rocket" => DeathCause::Heavy,
            "dbnotimeout" => DeathCause::DbnoTimeout, "banhammer" => DeathCause::Banhammer, "removed" | "removedfromgame" => DeathCause::RemovedFromGame, "self" | "unspecified" => DeathCause::Unspecified,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combatant { pub name: String, pub local: bool, pub teammate: bool }

impl Combatant {
    pub fn local(name: &str) -> Self { Combatant { name: name.to_owned(), local: true, teammate: true } }
    pub fn teammate(name: &str) -> Self { Combatant { name: name.to_owned(), local: false, teammate: true } }
    pub fn other(name: &str) -> Self { Combatant { name: name.to_owned(), local: false, teammate: false } }
    fn victim_style(&self) -> Style { if self.teammate || self.local { Style::TeamVictim } else { Style::Victim } }
    fn killer_style(&self) -> Style { if self.local { Style::LocalKiller } else if self.teammate { Style::TeamKiller } else { Style::Killer } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Killer { Nobody, Themself, Player(Combatant) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elimination { pub killer: Killer, pub victim: Combatant, pub cause: DeathCause, pub dbno: bool }

pub const KILL_FEED_OUTSIDE_SAFE_ZONE: &str = "<{VictimStyle}>{PlayerName}</> was lost in the storm";
pub const KILL_FEED_FALL: &str = "<{VictimStyle}>{PlayerName}</> didn't stick the landing";
pub const KILL_FEED_BANHAMMERED: &str = "<{VictimStyle}>{PlayerName}</> was struck by the banhammer";
pub const KILL_FEED_SUICIDE: &str = "<{VictimStyle}>{PlayerName}</> checked out early";
pub const KILL_FEED_DBNO_OUTSIDE_SAFE_ZONE: &str = "<{VictimStyle}>{PlayerName}</> was knocked out by the storm";
pub const KILL_FEED_DBNO_FALL: &str = "<{VictimStyle}>{PlayerName}</> got knocked out by a fall";
pub const KILL_FEED_DBNO_SUICIDE: &str = "<{VictimStyle}>{PlayerName}</> knocked themself out";
pub const KILL_FEED_PLAYER_ON_PLAYER_PISTOL: &str = "<{KillerStyle}>{Killer}</> eliminated <{VictimStyle}>{PlayerName}</> with a pistol";
pub const KILL_FEED_PLAYER_ON_PLAYER_SHOTGUN: &str = "<{KillerStyle}>{Killer}</> shotgunned <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_PLAYER_ON_PLAYER_RIFLE: &str = "<{KillerStyle}>{Killer}</> eliminated <{VictimStyle}>{PlayerName}</> with a rifle";
pub const KILL_FEED_PLAYER_ON_PLAYER_SMG: &str = "<{KillerStyle}>{Killer}</> eliminated <{VictimStyle}>{PlayerName}</> with an SMG";
pub const KILL_FEED_PLAYER_ON_PLAYER_SNIPER: &str = "<{KillerStyle}>{Killer}</> sniped <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_PLAYER_ON_PLAYER_MELEE: &str = "<{KillerStyle}>{Killer}</> bludgeoned <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_PLAYER_ON_PLAYER_HEAVY: &str = "<{KillerStyle}>{Killer}</> 'sploded <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_PLAYER_ON_PLAYER: &str = "<{KillerStyle}>{Killer}</> finally eliminated <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_PISTOL: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</> with a pistol";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_SHOTGUN: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</> with a shotgun";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_RIFLE: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</> with a rifle";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_SMG: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</> with an SMG";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_SNIPER: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</> with a sniper";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER: &str = "<{KillerStyle}>{Killer}</> knocked out <{VictimStyle}>{PlayerName}</>";
pub const KILL_FEED_DBNO_PLAYER_ON_PLAYER_HEAVY: &str = "<{KillerStyle}>{Killer}</> nearly 'sploded <{VictimStyle}>{PlayerName}</>";

pub const CHEAT_MESSAGES: [&str; 5] = [
    "<Killer>XWMXWMXWMXWMXWM</> eliminated <Victim>XWMXWMXWMXWMXWM</> with a rifle",
    "<LocalKiller>XWMXWMXWMXWMXWM</> eliminated <Victim>XWMXWMXWMXWMXWM</> with a rifle",
    "<TeamKiller>XWMXWMXWMXWMXWM</> eliminated <Victim>XWMXWMXWMXWMXWM</> with a rifle",
    "<Killer>XWMXWMXWMXWMXWM</> eliminated <Victim>XWMXWMXWMXWMXWM</> with a rifle",
    "<Killer>XWMXWMXWMXWMXWM</> eliminated <TeamVictim>XWMXWMXWMXWMXWM</> with a rifle",
];

impl Elimination {
    pub fn template(&self) -> Option<&'static str> {
        use DeathCause as C;
        match (&self.killer, self.dbno) {
            (Killer::Nobody, _) => None,
            (Killer::Themself, false) => match self.cause {
                C::OutsideSafeZone => Some(KILL_FEED_OUTSIDE_SAFE_ZONE),
                C::FallDamage => Some(KILL_FEED_FALL),
                C::Banhammer => Some(KILL_FEED_BANHAMMERED),
                C::RemovedFromGame => None,
                _ => Some(KILL_FEED_SUICIDE),
            },
            (Killer::Themself, true) => match self.cause {
                C::OutsideSafeZone => Some(KILL_FEED_DBNO_OUTSIDE_SAFE_ZONE),
                C::FallDamage => Some(KILL_FEED_DBNO_FALL),
                _ => Some(KILL_FEED_DBNO_SUICIDE),
            },
            (Killer::Player(_), false) => Some(match self.cause {
                C::Pistol => KILL_FEED_PLAYER_ON_PLAYER_PISTOL, C::Shotgun => KILL_FEED_PLAYER_ON_PLAYER_SHOTGUN, C::Rifle => KILL_FEED_PLAYER_ON_PLAYER_RIFLE,
                C::Smg => KILL_FEED_PLAYER_ON_PLAYER_SMG, C::Sniper => KILL_FEED_PLAYER_ON_PLAYER_SNIPER, C::Melee => KILL_FEED_PLAYER_ON_PLAYER_MELEE,
                C::Heavy => KILL_FEED_PLAYER_ON_PLAYER_HEAVY, _ => KILL_FEED_PLAYER_ON_PLAYER,
            }),
            (Killer::Player(_), true) => Some(match self.cause {
                C::Pistol => KILL_FEED_DBNO_PLAYER_ON_PLAYER_PISTOL, C::Shotgun => KILL_FEED_DBNO_PLAYER_ON_PLAYER_SHOTGUN, C::Rifle => KILL_FEED_DBNO_PLAYER_ON_PLAYER_RIFLE,
                C::Smg => KILL_FEED_DBNO_PLAYER_ON_PLAYER_SMG, C::Sniper => KILL_FEED_DBNO_PLAYER_ON_PLAYER_SNIPER, C::Heavy => KILL_FEED_DBNO_PLAYER_ON_PLAYER_HEAVY,
                _ => KILL_FEED_DBNO_PLAYER_ON_PLAYER,
            }),
        }
    }
    pub fn markup(&self) -> Option<String> {
        let t = self.template()?;
        let mut s = t.replace("{VictimStyle}", self.victim.victim_style().name()).replace("{PlayerName}", &self.victim.name);
        if let Killer::Player(k) = &self.killer { s = s.replace("{KillerStyle}", k.killer_style().name()).replace("{Killer}", &k.name); }
        Some(s)
    }
    pub fn counts_for_local(&self) -> bool { matches!(&self.killer, Killer::Player(k) if k.local) && !self.victim.local && !self.dbno }
}

pub fn parse_markup(markup: &str) -> Vec<(Style, String)> {
    let mut runs: Vec<(Style, String)> = Vec::new();
    let mut style = Style::Default;
    let mut cur = String::new();
    let mut rest = markup;
    while let Some(i) = rest.find('<') {
        cur.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(j) = tail.find('>') else { cur.push_str(tail); rest = ""; break };
        let tag = &tail[1..j];
        let valid = tag == "/" || (!tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if !valid { cur.push('<'); rest = &tail[1..]; continue; }
        if !cur.is_empty() { runs.push((style, std::mem::take(&mut cur))); }
        style = if tag == "/" { Style::Default } else { Style::from_name(tag) };
        rest = &tail[j + 1..];
    }
    cur.push_str(rest);
    if !cur.is_empty() { runs.push((style, cur)); }
    runs
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KillFeed { entries: Vec<(String, f32)> }

impl KillFeed {
    pub fn new() -> Self { Self::default() }
    pub fn push_markup(&mut self, markup: String, now: f32) {
        if self.entries.len() > MAX_ENTRIES - 1 { self.entries.remove(0); }
        self.entries.push((markup, now));
    }
    pub fn push(&mut self, e: &Elimination, now: f32) -> Option<String> {
        let m = e.markup()?;
        if m.is_empty() { return None; }
        self.push_markup(m.clone(), now);
        Some(m)
    }
    pub fn tick(&mut self, now: f32) -> bool {
        let n = self.entries.len();
        self.entries.retain(|(_, t)| !(now - *t > ENTRY_LIFETIME_S));
        n != self.entries.len()
    }
    pub fn lines(&self) -> Vec<String> { self.entries.iter().map(|(m, _)| m.clone()).collect() }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn clear(&mut self) { self.entries.clear(); }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineLayout { pub border: Rect, pub text_x: f32, pub baseline: f32 }

pub fn band_top(w: f32, h: f32, content_h: f32) -> f32 {
    let s = dpi_scale(w, h);
    h - (BOTTOM_LEFT_PAD_BOTTOM + WATCHERS_PAD_TOP + WATCHERS_H) * s - (FEED_MIN_SIZE.1 * s).max(content_h)
}

pub fn build(ctx: &mut UiContext, w: f32, h: f32, lines: &[String], out: &mut RenderList) {
    if lines.is_empty() { return; }
    let s = dpi_scale(w, h);
    let size_px = font_px(FONT_PT, s);
    let reg = slate_line(NOTO_REGULAR_BBOX_Y, size_px);
    let bold = slate_line(NOTO_BOLD_BBOX_Y, size_px);
    let px = atlas_px(FONT_PT, s);
    let (pad_x, pad_y) = (BORDER_PAD.0 * s, BORDER_PAD.1 * s);
    let start = lines.len().saturating_sub(MAX_ENTRIES);
    let parsed: Vec<Vec<(Style, String)>> = lines[start..].iter().map(|l| parse_markup(l)).collect();
    let metrics: Vec<(f32, f32)> = parsed.iter().map(|runs| {
        let any_bold = runs.iter().any(|(st, _)| st.bold());
        let any_reg = runs.is_empty() || runs.iter().any(|(st, _)| !st.bold());
        let mut lh: f32 = 0.; let mut asc: f32 = 0.;
        if any_reg { lh = lh.max(reg.0); asc = asc.max(reg.1); }
        if any_bold { lh = lh.max(bold.0); asc = asc.max(bold.1); }
        (lh, asc)
    }).collect();
    let content: f32 = metrics.iter().map(|(lh, _)| lh + 2. * pad_y).sum();
    let mut y = band_top(w, h, content);
    let x0 = FEED_PAD_LEFT * s;
    for (runs, (lh, asc)) in parsed.iter().zip(metrics) {
        let widths: Vec<f32> = runs.iter().map(|(st, t)| text_width(ctx, st.font(), t, px)).collect();
        let tw: f32 = widths.iter().sum();
        let bh = lh + 2. * pad_y;
        let border = Rect::new(x0.round(), y.round(), (x0 + tw + 2. * pad_x).round() - x0.round(), (y + bh).round() - y.round());
        out.fill(border, BORDER_COLOR);
        let baseline = (y + pad_y + asc).round();
        let mut pen = x0 + pad_x;
        for ((st, t), wd) in runs.iter().zip(widths) {
            let c = srgb3(st.linear_color());
            put_text(ctx, out, st.font(), px, pen.round(), baseline, t, [c[0], c[1], c[2], 1.], None, Snap::Pen);
            pen += wd;
        }
        y += bh;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn me() -> Combatant { Combatant::local("Brady") }
    #[test] fn environmental_deaths_use_the_self_templates() {
        let e = Elimination { killer: Killer::Themself, victim: me(), cause: DeathCause::OutsideSafeZone, dbno: false };
        assert_eq!(e.markup().unwrap(), "<TeamVictim>Brady</> was lost in the storm");
        let f = Elimination { cause: DeathCause::FallDamage, ..e.clone() };
        assert_eq!(f.markup().unwrap(), "<TeamVictim>Brady</> didn't stick the landing");
        let h = Elimination { cause: DeathCause::Heavy, ..e.clone() };
        assert_eq!(h.markup().unwrap(), "<TeamVictim>Brady</> checked out early");
        assert!(Elimination { cause: DeathCause::RemovedFromGame, ..e.clone() }.markup().is_none());
        assert!(Elimination { killer: Killer::Nobody, ..e }.markup().is_none());
    }
    #[test] fn player_kills_pick_the_weapon_template_and_styles() {
        let e = Elimination { killer: Killer::Player(me()), victim: Combatant::other("Bot"), cause: DeathCause::Shotgun, dbno: false };
        assert_eq!(e.markup().unwrap(), "<LocalKiller>Brady</> shotgunned <Victim>Bot</>");
        assert!(e.counts_for_local());
        let r = Elimination { killer: Killer::Player(Combatant::other("A")), victim: me(), cause: DeathCause::Unspecified, dbno: false };
        assert_eq!(r.markup().unwrap(), "<Killer>A</> finally eliminated <TeamVictim>Brady</>");
        let d = Elimination { killer: Killer::Player(Combatant::teammate("T")), victim: Combatant::other("B"), cause: DeathCause::Melee, dbno: true };
        assert_eq!(d.markup().unwrap(), "<TeamKiller>T</> knocked out <Victim>B</>");
    }
    #[test] fn markup_parses_into_styled_runs() {
        let r = parse_markup(CHEAT_MESSAGES[1]);
        assert_eq!(r, vec![(Style::LocalKiller, "XWMXWMXWMXWMXWM".into()), (Style::Default, " eliminated ".into()), (Style::Victim, "XWMXWMXWMXWMXWM".into()), (Style::Default, " with a rifle".into())]);
        assert_eq!(parse_markup("a < b"), vec![(Style::Default, "a < b".into())]);
    }
    #[test] fn the_feed_keeps_five_and_expires_after_six_seconds() {
        let mut f = KillFeed::new();
        for (i, m) in CHEAT_MESSAGES.iter().enumerate() { f.push_markup((*m).into(), i as f32); }
        f.push_markup("x".into(), 5.);
        assert_eq!(f.len(), 5);
        assert_eq!(f.lines()[0], CHEAT_MESSAGES[1]);
        assert!(!f.tick(7.));
        assert!(f.tick(7.01));
        assert_eq!(f.len(), 4);
        f.tick(100.);
        assert!(f.is_empty());
    }
    #[test] fn weapon_tags_map_like_the_pawn() {
        assert_eq!(DeathCause::from_weapon_tag("Weapon.ranged.assault"), DeathCause::Rifle);
        assert_eq!(DeathCause::from_weapon_tag("Weapon.Melee"), DeathCause::Melee);
        assert_eq!(DeathCause::from_weapon_row("Shotgun_Standard_Athena_C_Ore_T03"), DeathCause::Shotgun);
    }
}
