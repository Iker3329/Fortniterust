//! `ItemDetailsHeader_C` (`/Game/UI/Foundation/Items/Details/ItemDetailsHeader`) as 1.8 fills it at runtime, shared by every cooked tree that nests it:
//! the inventory's `ItemDetails` panel (`inventory_cooked`) and the pickup card of the interaction prompt (`HUD-PickupItemWidget`, `interaction_cooked`).
//!
//! SOURCES (Kismet, `fn-inspect disasm`): `ItemDetailsHeader.UpdateItemsForWidgets`, `ItemDetailsHeaderRarityTypeText`, `ItemDetailsHeaderItemDisplayText`,
//! `ItemDetailsHeaderTagListText.FormatItemDisplayTags`, `ItemDetailsStackCounter.Set Stack Count` / `Refresh Visibility`, `ItemDescriptionDetailWidget`;
//! the native fills (exe): `FortItemTypeFunctionLibrary::ToText` (0x14061d7f0), `FortInventoryContext::GetUserFriendlyTags` (0x142e34410),
//! `FortItemCategoryIndicator` (0x142fee080 / 0x142febd90), `FortTierIndicator` (`UFortItem::GetTier` 0x14068d3b0). See `inventory_cooked` for the details.
use super::athena_lobby::paint::{Painter, Tf};
use super::athena_lobby::store::{jvec, Store};
use super::athena_lobby::styles::Brush;
use super::athena_lobby::umg::{Ui, Vis};
use crate::screens::inventory_panel::InvItem;
use serde_json::{json, Value};
use std::collections::HashMap;

const RARITY_DATA: &str = "/Game/Balance/RarityData";
/// `DefaultGameData.ItemCategories` (`UFortGameData` +0x160, read by `UFortItemDefinition` vfuncs 92 / 93 / 94).
const ITEM_CATEGORIES: &str = "/Game/Items/ItemCategories";
/// INVENTORY FIX4 (verify3 V6): `FilledTierPipMultiSizeBrushAsset` (`/Game/UI/Foundation/Items/Parts/`), the filled pip of `FortTierIndicator`.
const TIER_PIP: &str = "/Game/UI/Foundation/Items/Parts/FilledTierPipMultiSizeBrushAsset";

struct Rarity { name: Value, colors: [[f32; 4]; 5] }
/// `FFortItemCategory`: tag container, name and `CategoryBrush` (`FFortMultiSizeBrush`: `Brush_XXS` .. `Brush_XL`).
struct Category { tags: Vec<String>, name: Value, brush: Value }

/// `EFortBrushSize` -> the `FFortMultiSizeBrush` member (`EFortBrushSize::Small` -> `Brush_S`).
pub fn multi_size_brush(b: &Value, size: Option<&str>) -> Option<Brush> {
    let key = match size.map(|s| s.rsplit("::").next().unwrap_or(s)) { Some("ExtraExtraSmall") => "Brush_XXS", Some("ExtraSmall") => "Brush_XS", Some("Small") => "Brush_S", Some("Large") => "Brush_L", Some("ExtraLarge") => "Brush_XL", _ => "Brush_M" };
    b.get(key).and_then(Brush::from_json)
}
/// `EFortItemTier` (1.8 SDK declaration order: No_Tier, I .. X) of a cooked `Tier` value (`EFortItemTier::III` -> 3).
fn item_tier(v: Option<&Value>) -> u8 {
    const T: [&str; 11] = ["No_Tier", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    v.and_then(|v| v.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s)).and_then(|n| T.iter().position(|t| *t == n)).unwrap_or(0) as u8
}
/// `EFortItemType::WeaponRanged` -> 17 (declaration order of the 1.8 SDK enum).
pub fn item_type_index(s: &str) -> Option<u8> {
    const T: [&str; 44] = ["WorldItem", "Ammo", "Badge", "BackpackPickup", "BuildingPiece", "CharacterPart", "Deco", "EditTool", "Ingredient", "Food", "Gadget", "HomebaseGadget", "HeroAbility", "MissionItem", "Trap", "Weapon", "WeaponMelee", "WeaponRanged", "WeaponHarvest", "WorldResource", "AccountItem", "AccountResource", "CollectedResource", "Alteration", "CardPack", "CharacterCosmetic", "Currency", "Hero", "Schematic", "Worker", "Token", "DailyRewardScheduleToken", "CodeToken", "Buff", "BuffCredit", "Quest", "Compendium", "CompendiumBundle", "GameplayModifier", "Outpost", "HomebaseNode", "Defender", "ConversionControl", "DeployableBaseCloudSave"];
    let n = s.rsplit("::").next().unwrap_or(s);
    T.iter().position(|t| *t == n).map(|i| i as u8)
}
/// `UFortItemTypeFunctionLibrary::ToText(type, false)` (exe 0x14061d7f0): `LOCTEXT("EFortItemType", <enum name>, <source>)`, singular.
pub fn item_type_text(t: u8) -> (&'static str, &'static str) {
    match t {
        0 => ("WorldItem", "Zone Inventory Item"), 1 => ("Ammo", "Ammo"), 2 => ("Badge", "Badge"), 3 => ("BackpackPickup", "Backpack"), 4 => ("BuildingPiece", "Building Piece"),
        5 => ("CharacterPart", "Cosmetic Item"), 6 => ("Deco", "Decorative Item"), 7 => ("EditTool", "Editing Tool"), 8 => ("Ingredient", "Ingredient"), 9 => ("Food", "Food"),
        10 => ("Gadget", "Gadget"), 11 => ("HomebaseGadget", "Gadget"), 12 => ("HeroAbility", "Ability"), 13 => ("MissionItem", "Mission Item"), 14 => ("Trap", "Trap"),
        15 => ("Weapon", "Weapon"), 16 => ("WeaponMelee", "Melee Weapon"), 17 => ("WeaponRanged", "Ranged Weapon"), 18 => ("WeaponHarvest", "Harvesting Tool"),
        19 => ("WorldResource", "World Resource"), 20 => ("AccountItem", "Account Item"), 21 => ("AccountResource", "Account Resource"), _ => ("", ""),
    }
}
/// `FText::AsNumber` of an integer (default options: digit grouping).
pub fn group(v: i64) -> String {
    let s = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); }
    if v < 0 { format!("-{out}") } else { out }
}
/// Localized text: `Game.locres`, else the source string (what a native `LOCTEXT` / an unlocalized `FText` shows in English).
pub fn loc(store: &Store, ns: &str, key: &str, source: &str) -> String {
    store.locres().and_then(|l| l.get(ns, key).map(|s| s.to_owned())).unwrap_or_else(|| source.to_owned())
}
/// A cooked `FText` JSON value, localized.
pub fn text(store: &Store, v: &Value) -> String {
    let ns = v.get("namespace").and_then(|s| s.as_str()).unwrap_or("");
    let key = v.get("key").and_then(|s| s.as_str()).unwrap_or("");
    let src = v.get("text").and_then(|s| s.as_str()).unwrap_or("");
    if key.is_empty() { src.to_owned() } else { loc(store, ns, key, src) }
}

/// The data tables the header reads (`RarityData`, `ItemCategories`, the tier pip brush) plus per-definition caches.
pub struct HeaderData {
    rarity: Vec<Rarity>,
    primary: Vec<(u8, Value)>,
    secondary: Vec<Category>,
    tertiary: Vec<Category>,
    def_tags: HashMap<String, Vec<String>>,
    /// INVENTORY FIX4: the definition's `Tier` byte (+0x145) per definition path
    def_tier: HashMap<String, u8>,
    /// INVENTORY FIX4: `FilledTierPipMultiSizeBrushAsset.Brush` (multi-size)
    tier_pip: Value,
}

impl HeaderData {
    pub fn load(store: &Store) -> HeaderData {
        let tier_pip = store.pkg(TIER_PIP).and_then(|p| p.exports.iter().find(|e| e.class == "FortMultiSizeBrushAsset").and_then(|e| e.props.get("Brush").cloned())).unwrap_or(Value::Null);
        // RarityData rows (`RarityCollection`, `RarityCollection[1]`, ...)
        let mut rarity = vec![];
        if let Some(e) = store.pkg(RARITY_DATA).and_then(|p| p.exports.iter().find(|e| e.class == "FortRarityData").map(|e| e.props.clone())) {
            for k in 0..8 {
                let key = if k == 0 { "RarityCollection".to_owned() } else { format!("RarityCollection[{k}]") };
                let Some(r) = e.get(&key) else { break };
                let c = |n: &str| r.get(n).and_then(|v| jvec::<4>(Some(v))).unwrap_or([1.; 4]);
                rarity.push(Rarity { name: r.get("Name").cloned().unwrap_or(Value::Null), colors: [c("Color1"), c("Color2"), c("Color3"), c("Color4"), c("Color5")] });
            }
        }
        // ItemCategories (Primary by item type, Secondary / Tertiary by gameplay tags)
        let (mut primary, mut secondary, mut tertiary) = (vec![], vec![], vec![]);
        if let Some(p) = store.pkg(ITEM_CATEGORIES).and_then(|p| p.exports.iter().find(|e| e.class == "FortItemCategory").map(|e| e.props.clone())) {
            for e in p.get("PrimaryCategories").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
                if let Some(t) = e.get("CategoryType").and_then(|t| t.as_str()).and_then(item_type_index) { primary.push((t, e.get("CategoryName").cloned().unwrap_or(Value::Null))); }
            }
            let cats = |k: &str| -> Vec<Category> { p.get(k).and_then(|a| a.as_array()).cloned().unwrap_or_default().iter().map(|e| Category {
                tags: e.get("TagContainer").and_then(|t| t.get("GameplayTags")).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_ascii_lowercase())).collect()).unwrap_or_default(),
                name: e.get("CategoryName").cloned().unwrap_or(Value::Null), brush: e.get("CategoryBrush").cloned().unwrap_or(Value::Null) }).collect() };
            secondary = cats("SecondaryCategories"); tertiary = cats("TertiaryCategories");
        }
        HeaderData { rarity, primary, secondary, tertiary, def_tags: HashMap::new(), def_tier: HashMap::new(), tier_pip }
    }
    /// The item definition's `GameplayTags` plus every parent tag (`FGameplayTagContainer` GameplayTags + ParentTags), lower case (FName compare).
    fn tags_of(&mut self, store: &Store, def: &str) -> Vec<String> {
        if let Some(t) = self.def_tags.get(def) { return t.clone(); }
        let mut out: Vec<String> = vec![];
        if !def.is_empty() {
            let obj = if def.contains('.') { def.to_owned() } else { format!("{def}.{}", def.rsplit('/').next().unwrap_or(def)) };
            if let Some((pkg, i)) = store.object(&obj) {
                for t in pkg.exports[i].props.get("GameplayTags").and_then(|g| g.get("GameplayTags")).and_then(|a| a.as_array()).cloned().unwrap_or_default() {
                    let Some(t) = t.as_str() else { continue };
                    let t = t.to_ascii_lowercase();
                    let parts: Vec<&str> = t.split('.').collect();
                    for k in 1..=parts.len() { let p = parts[..k].join("."); if !out.contains(&p) { out.push(p); } }
                }
            }
        }
        self.def_tags.insert(def.to_owned(), out.clone());
        out
    }
    /// INVENTORY FIX4: `UFortItem::GetTier` (0x14068d3b0) -> definition vfunc +0x278 (0x14068d3e0) = the definition's `Tier` byte (+0x145). Cooked BR values:
    /// weapons Common 1 .. Legendary 5 (`EFortItemTier::III` for a Rare sniper); ammo / materials / consumables / traps / pickaxes carry none (0).
    pub fn tier_of(&mut self, store: &Store, def: &str) -> u8 {
        if let Some(t) = self.def_tier.get(def) { return *t; }
        let mut t = 0;
        if !def.is_empty() {
            let obj = if def.contains('.') { def.to_owned() } else { format!("{def}.{}", def.rsplit('/').next().unwrap_or(def)) };
            if let Some((pkg, i)) = store.object(&obj) { t = item_tier(pkg.exports[i].props.get("Tier")); }
        }
        self.def_tier.insert(def.to_owned(), t);
        t
    }
    /// `UFortInventoryContext::GetUserFriendlyTags` (0x142e34410) for a 1.8 BR world item: only items whose `GetSchematicCraftingResultOrCraftedWeaponBP` is a weapon
    /// definition get tags: [DisplayTier text (BR weapons carry no `DisplayTier`)], the primary category of the item type (vfunc 92), then for ranged / trap
    /// definitions (not melee) the first `SecondaryCategories` entry whose tags meet the definition's tags (vfunc 93), then the tertiary one (vfunc 94).
    pub fn user_friendly_tags(&mut self, store: &Store, it: &InvItem) -> Vec<String> {
        if !matches!(it.item_type, 14 | 16 | 17 | 18) { return vec![]; }
        let mut out = vec![];
        if let Some((_, n)) = self.primary.iter().find(|(t, _)| *t == it.item_type) { out.push(text(store, &n.clone())); }
        let tags = self.tags_of(store, &it.def_path);
        let pick = |cats: &[Category]| cats.iter().find(|c| c.tags.iter().any(|t| tags.contains(t))).map(|c| c.name.clone());
        if matches!(it.item_type, 17 | 14) { if let Some(n) = pick(&self.secondary) { out.push(text(store, &n)); } }
        if let Some(n) = pick(&self.tertiary) { out.push(text(store, &n)); }
        out.retain(|s| !s.is_empty());
        out
    }
}

/// The widgets of one `ItemDetailsHeader_C` instance (plus the inventory's `ItemDescriptionDetailWidget`, when the scope has one).
#[derive(Default)]
pub struct DetailsW { pub header: Option<usize>, pub rarity_text: Option<usize>, pub type_text: Option<usize>, pub name: Option<usize>, pub tag_list: Option<usize>, pub stack: Option<usize>, pub stack_text: Option<usize>,
    pub desc_widget: Option<usize>, pub desc_text: Option<usize>, pub rarity_border: Option<usize>, pub level_bar: Option<usize>, pub category: Option<usize>, pub icon_spacer: Option<usize>, pub tertiary: Option<usize>,
    /// INVENTORY FIX4: `TierIndicator` (`FortTierIndicator`) and the pip count of the shown item
    pub tier: Option<usize>, pub tier_pips: u8 }

impl DetailsW {
    /// The header widgets under `scope` (`header_name` = the `ItemDetailsHeader_C` instance's widget name in the outer tree).
    pub fn find(ui: &mut Ui, scope: Option<usize>, header_name: &str) -> DetailsW {
        let f = |ui: &Ui, n: &str| scope.and_then(|d| ui.find_in(d, n));
        let d = DetailsW {
            header: f(ui, header_name), rarity_text: f(ui, "RarityTextBlock"), type_text: f(ui, "TypeTextBlock"), name: f(ui, "ItemDisplayName"), tag_list: f(ui, "TagListTextBlock"),
            stack: f(ui, "ItemDetailsStackCounter"), stack_text: f(ui, "TextCounter"), desc_widget: f(ui, "ItemDescriptionDetailWidget"), desc_text: f(ui, "ItemDescriptionTextBlock"),
            rarity_border: f(ui, "RarityBorder"), level_bar: f(ui, "LevelBar"), category: f(ui, "CategoryIndicator"), icon_spacer: f(ui, "IconSpacer"), tertiary: f(ui, "TertiaryRow"),
            tier: f(ui, "TierIndicator"), tier_pips: 0,
        };
        // INVENTORY FIX4 (verify3 V6): the two native leaf widgets of the header, laid out as what their `RebuildWidget` builds:
        // * `CategoryIndicator` = `UFortIconWithLabel` (layout 0x142f7e5b0: HBox [SFortMultiSizeImage] [SSpacer] [STextBlock]); `ShouldShowLabel` is false in the data, so the
        //   label is Collapsed (0x142fec220), the spacer too (0x142fece60: only when icon and label both show) and the icon slot is Fill / Fill (0x142febd90):
        //   the widget is the icon image at its `IconBrushSize` brush (an `Image` with that brush, set per item in `apply`);
        // * `TierIndicator` = `UFortTierIndicator`: an `SHorizontalBox` of one `SImage` per pip (slots centred, `InterPipPadding` 0 from the ctor 0x143004fd0):
        //   a box of Tier x pip size (a `Spacer` of that size); the pips are painted over its geometry in `paint_tier`.
        if let Some(c) = d.category { ui.w[c].class = "Image".into(); ui.w[c].vis = Vis::Collapsed; }
        if let Some(t) = d.tier { ui.w[t].class = "Spacer".into(); ui.w[t].props["Size"] = json!([0., 0.]); }
        d
    }

    /// `SetItemToDetail(it)`: everything the header's Blueprint graphs set from the item.
    pub fn apply(&mut self, ui: &mut Ui, data: &mut HeaderData, it: &InvItem) {
        let store = ui.store.clone();
        let set_vis = |ui: &mut Ui, id: Option<usize>, v: Vis| if let Some(i) = id { ui.w[i].vis = v; };
        let set_text = |ui: &mut Ui, id: Option<usize>, t: &str| if let Some(i) = id { ui.w[i].text = Some(t.to_owned()); };
        let r = data.rarity.get(it.rarity as usize).map(|r| (r.name.clone(), r.colors));
        let (rname, rc) = match r { Some((n, c)) => (text(&store, &n), c), None => (String::new(), [[1.; 4]; 5]) };
        // ItemDetailsHeaderRarityTypeText: RarityTextBlock = RarityData.Name, colour Color1 (not a schematic); TypeTextBlock = Format("| {Type}", ToText(GetType()))
        set_text(ui, self.rarity_text, &rname);
        if let Some(i) = self.rarity_text { ui.w[i].props["ColorAndOpacity"] = json!(rc[0]); }
        let (key, src) = item_type_text(it.item_type);
        let tname = if key.is_empty() { String::new() } else { loc(&store, "EFortItemType", key, src) };
        let fmt = loc(&store, "[7E09F09D4ABDE90F6C6D14BA8696184F]", "D677E949474D19BB4205F5A3DB5C88D4", "| {Type}");
        set_text(ui, self.type_text, &fmt.replace("{Type}", &tname));
        // ItemDetailsHeaderItemDisplayText: TextToUpper(GetDisplayName()), Collapsed when empty
        let up = it.name.trim().to_uppercase();
        set_text(ui, self.name, &up); set_vis(ui, self.name, if up.is_empty() { Vis::Collapsed } else { Vis::Visible });
        // ItemDetailsHeaderTagListText: FormatItemDisplayTags(GetUserFriendlyTags) = "{A} | {B}" folded; Visible when not empty else Collapsed
        let tags = data.user_friendly_tags(&store, it);
        let join = loc(&store, "[9D36BA5049495F3995E5DC823D8D8850]", "41C051894D429ED6BD80CCA7A7535745", "{A} | {B}");
        let mut res = String::new();
        for t in &tags { res = if res.is_empty() { t.clone() } else { join.replace("{A}", &res).replace("{B}", t) }; }
        set_text(ui, self.tag_list, &res); set_vis(ui, self.tag_list, if res.is_empty() { Vis::Collapsed } else { Vis::Visible });
        // ItemDetailsHeader.UpdateItemsForWidgets
        if let Some(b) = self.rarity_border { ui.w[b].mat_vectors = vec![("Rarity 2".into(), rc[1]), ("Rarity 3".into(), rc[2]), ("Rarity 4".into(), rc[3])]; ui.w[b].mat_scalars = vec![("bIsSchematic".into(), 0.)]; }
        let ty = it.item_type;
        let cat_vis = if matches!(ty, 27 | 14 | 15 | 16 | 17 | 18) { Vis::SelfHitTestInvisible } else { Vis::Collapsed };
        set_vis(ui, self.category, cat_vis); set_vis(ui, self.icon_spacer, cat_vis);
        // INVENTORY FIX4 (verify3 V6): `FortItemCategoryIndicator` refresh 0x142fee080 -> item vfunc +0x2d0 `TryGetCategory` (0x140698750) with the widget's
        // `OrdinalOfCategoryToRepresent` (Secondary -> definition vfunc +0x2e8: the `ItemCategories.SecondaryCategories` entry whose `TagContainer` meets the
        // definition's gameplay tags, the same match as `user_friendly_tags`; BR definitions cook no `SecondaryCategoryOverrideTags`); found -> the icon
        // (`CategoryBrush` at `IconBrushSize`) Visible (0x142febd90: shown and a brush exists), else the icon Collapsed (`FUN_142fe0370(this, 0)`)
        if let Some(c) = self.category {
            let tags = data.tags_of(&store, &it.def_path);
            let size = ui.w[c].props.get("IconBrushSize").and_then(|s| s.as_str()).map(|s| s.to_owned());
            let b = data.secondary.iter().find(|k| k.tags.iter().any(|t| tags.contains(t))).and_then(|k| multi_size_brush(&k.brush, size.as_deref()));
            let b = b.filter(|b| b.draw != super::athena_lobby::styles::DrawAs::NoDraw && b.resource.is_some());
            ui.w[c].vis = if cat_vis.takes_space() && b.is_some() { Vis::Visible } else { Vis::Collapsed };
            ui.w[c].brush = b;
        }
        // INVENTORY FIX4 (verify3 V6): `FortTierIndicator` pip count = `UFortItem::GetTier`; pip i < Tier = `FilledTierPipMultiSizeBrushAsset` at `BrushSize`
        // (`Small` in the data: `T-Stars-Full-S`, 24 x 24); Tier 0 draws nothing. The box is Tier x pip wide (`InterPipPadding` 0) and one pip tall.
        if let Some(t) = self.tier {
            let n = data.tier_of(&store, &it.def_path);
            let size = ui.w[t].props.get("BrushSize").and_then(|s| s.as_str()).map(|s| s.to_owned());
            let pip = multi_size_brush(&data.tier_pip, size.as_deref()).map(|b| b.image_size).unwrap_or([0., 0.]);
            self.tier_pips = n;
            ui.w[t].props["Size"] = if n > 0 { json!([pip[0] * n as f32, pip[1]]) } else { json!([0., 0.]) };
        }
        let show_ammo = self.header.and_then(|h| ui.w[h].props.get("Show Ammo Header").and_then(|b| b.as_bool())).unwrap_or(false);
        set_vis(ui, self.tertiary, if ty == 17 && show_ammo { Vis::SelfHitTestInvisible } else { Vis::Collapsed });
        set_vis(ui, self.level_bar, if matches!(ty, 28 | 15 | 16 | 17 | 18 | 14 | 27 | 41 | 29 | 8) { Vis::SelfHitTestInvisible } else { Vis::Collapsed });
        if let Some(b) = self.level_bar { ui.w[b].mat_vectors = vec![("RGBA0".into(), rc[3]), ("RGBA1".into(), rc[2])]; }
        // ItemDetailsStackCounter.Set Stack Count(GetNumInStack): TextCounter = Conv_IntToText(count, grouping); HitTestInvisible when count > 1 (or AlwaysVisible) else Hidden
        let always = self.stack.and_then(|s| ui.w[s].props.get("AlwaysVisible").and_then(|b| b.as_bool())).unwrap_or(false);
        set_text(ui, self.stack_text, &group(it.count as i64));
        set_vis(ui, self.stack, if it.count > 1 || always { Vis::HitTestInvisible } else { Vis::Hidden });
        // ItemDescriptionDetailWidget: ShouldShowOnlyIfItemIsSimple (true in the data) && !IsItemSimple (type in [28,15,16,17,18,14,27]) -> Collapsed; else the
        // description (GetCombinedDescription has no tooltip context here -> GetDescription), SelfHitTestInvisible when not empty else Collapsed
        let only_simple = self.desc_widget.and_then(|w| ui.w[w].props.get("ShouldShowOnlyIfItemIsSimple").and_then(|b| b.as_bool())).unwrap_or(false);
        let simple = !matches!(ty, 28 | 15 | 16 | 17 | 18 | 14 | 27);
        if !only_simple || simple {
            set_text(ui, self.desc_text, &it.description);
            set_vis(ui, self.desc_widget, if it.description.trim().is_empty() { Vis::Collapsed } else { Vis::SelfHitTestInvisible });
        } else { set_vis(ui, self.desc_widget, Vis::Collapsed); }
    }

    /// INVENTORY FIX4 (verify3 V6): the `FortTierIndicator` pips over its box (each `SImage` slot centred H and V, `InterPipPadding` 0), colour (1, 1, 1, 1).
    pub fn paint_tier(&self, p: &mut Painter, data: &HeaderData) {
        let (Some(t), n) = (self.tier, self.tier_pips) else { return };
        let shown = { let mut cur = Some(t); let mut ok = true; while let Some(i) = cur { if !p.c.ui.w[i].vis.draws() { ok = false; break; } cur = p.c.ui.w[i].parent; } ok };
        let size = p.c.ui.w[t].props.get("BrushSize").and_then(|s| s.as_str()).map(|s| s.to_owned());
        if let (true, Some(b)) = (shown && n > 0, multi_size_brush(&data.tier_pip, size.as_deref())) {
            let r = p.c.lay.rect[t];
            let cw = r[2] / n as f32;
            for i in 0..n as usize {
                let pr = [r[0] + i as f32 * cw + (cw - b.image_size[0]) * 0.5, r[1] + (r[3] - b.image_size[1]) * 0.5, b.image_size[0], b.image_size[1]];
                p.draw_brush(&b, pr, [1.; 4], Tf::ID, false);
            }
        }
    }
}
