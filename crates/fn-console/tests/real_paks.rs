//! The console against the real 1.8 paks (needs FN_TEST_ROOT; skipped otherwise): cosmetic lists, completion and `skin` / `pickaxe` through fn_game::cosmetic_swap.
use fn_console::*;
use fn_game::GameSim;

#[test]
fn skin_and_pickaxe_use_the_real_catalog() {
    if std::env::var_os("FN_TEST_ROOT").is_none() { eprintln!("FN_TEST_ROOT not set: skipped"); return; }
    let skins = PakCosmetics.list(CosmeticKind::Skin);
    assert!(skins.len() > 10, "{} skins", skins.len());
    let mut sim = GameSim::new();
    let cat = Catalog::default();
    let mut con = Console::new(); con.open();
    let first = skins[0].clone();
    for c in format!("skin {}", first.id).chars() { con.key(Key::Char(c), &mut Env { sim: &mut sim, catalog: &cat, vitals: Vitals::default(), cosmetics: &PakCosmetics }); }
    // completion lists the skin by id and display name
    assert!(con.completion().items.iter().any(|s| s.insert == first.id), "{:?}", con.completion().items.iter().take(3).collect::<Vec<_>>());
    let fx = con.key(Key::Enter, &mut Env { sim: &mut sim, catalog: &cat, vitals: Vitals::default(), cosmetics: &PakCosmetics });
    assert_eq!(fx.lines[0].0, LineKind::Ok, "{:?}", fx.lines);
    assert_eq!(sim.cosmetics.skin.as_ref().map(|i| i.id.as_str()), Some(first.id.as_str()));
    let fx = execute("pickaxe definitely-not-a-pickaxe", &mut Env { sim: &mut sim, catalog: &cat, vitals: Vitals::default(), cosmetics: &PakCosmetics }, &mut Flags::default());
    assert_eq!(fx.lines[0].0, LineKind::Error);
    let fx = execute("list pickaxes", &mut Env { sim: &mut sim, catalog: &cat, vitals: Vitals::default(), cosmetics: &PakCosmetics }, &mut Flags::default());
    assert!(fx.lines.len() > 3, "{:?}", fx.lines);
}
