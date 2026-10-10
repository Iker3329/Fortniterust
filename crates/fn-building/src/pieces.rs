//! The four Athena player-buildable piece types, the three material tiers, and the cooked data behind them
//! ([`PieceCatalog`]: class, mesh, collision convexes, connectivity cube, health, build time, cost).
//!
//! SDK 1.8: `EFortBuildingType` (Wall 0, Floor 1, Corner 2, Deco 3, Prop 4, Stairs 5, Roof 6, Pillar 7, ...) and
//! `EFortResourceType` (Wood 0, Stone 1, Metal 2, Permanite 3, None).
//!
//! Where every number comes from (all read from the paks by [`PieceCatalog::load`]; nothing here is typed in):
//! * the build slot -> item: `/Game/Items/Weapons/BuildingTools/BuildingItemData_{Wall,Floor,Stair_W,RoofS}` (`FortBuildingItemDefinition`),
//!   whose `BuildingMetaData` is the edit pattern (`EMP_Wall_Solid`, `EMP_Floor_Floor`, `EMP_Stair_StairW`, `EMP_Roof_RoofC`).
//! * the item + material -> class: the 1.8 exe filters the player-buildable classes by `FPlayerBuildableClassFilter { ResourceType,
//!   BuildingType, Level, EditModeMetadata }` (`AFortGameStateZone::PlayerBuildableClasses[4]`, SDK). We run the same filter over the
//!   cooked `PBWA_*` Blueprints: `bIsPlayerBuildable`, `ResourceType`, `CurrentBuildingLevel` (L1 classes leave it at the native 0,
//!   `ABuildingActor::ABuildingActor` @140458440 zeroes +0x498) and `EditModePatternData` == the item's pattern.
//! * max health: the class `AttributeInitKey` (`PlayerBuildingWood1` / `Size9` ...) -> row `<Category>.<SubCategory>.FortHealthSet.MaxHealth`
//!   of `/Game/Balance/DataTables/AttributesBuildingSection`, evaluated at the `PlayerBuildingSkill` level (see [`ATHENA_BUILDING_LEVEL`]).
//! * build time: row `<Category>.FortBuildingActorSet.BuildTime` of the same table (`ABuildingSMActor` vfunc_371 @14046c580 reads the
//!   `FortBuildingActorSet.BuildTime` attribute while `bIsInitiallyBuilding`).
//! * initial health %: `UFortGameData::BuildingInitialHealthPercentHandles[resource]` of `/Game/Balance/DefaultGameData`
//!   (`UFortGameData::GetBuildingInitialHealthPct` @140586aa0, evaluated at the building level).
//! * cost: `UFortGameData::BuildingCostHandles[resource]` (`UFortGameData::GetBuildingCost` @140586240, evaluated at the building level).
//! * collision: the static mesh's `BodySetup.AggGeom.ConvexElems[].VertexData` (cm, actor local space).
//! * connectivity cube: the edit pattern's `ConnectivityCubeData` (6 faces x 25 bits; see `crate::placement`).
use std::collections::HashMap;

use glam::Vec3;

/// Player build piece type (SDK `EFortBuildingType` subset used by the four BR build slots).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PieceKind {
    /// `ABuildingWall` (slot 1).
    Wall,
    /// `ABuildingFloor` (slot 2).
    Floor,
    /// `ABuildingStairs : ABuildingFloor` (slot 3, the ramp).
    Stairs,
    /// `ABuildingRoof` (slot 4, the cone).
    Roof,
}

impl PieceKind {
    pub const ALL: [PieceKind; 4] = [PieceKind::Wall, PieceKind::Floor, PieceKind::Stairs, PieceKind::Roof];
    /// `EFortBuildingType` value (ABuildingWall/Floor/Stairs/Roof constructors write +0x3F0: 0 / 1 / 5 / 6).
    pub fn building_type(self) -> u8 { match self { PieceKind::Wall => 0, PieceKind::Floor => 1, PieceKind::Stairs => 5, PieceKind::Roof => 6 } }
    /// The `FortBuildingItemDefinition` of the BR build slot.
    pub fn item_definition(self) -> &'static str {
        match self {
            PieceKind::Wall => "/Game/Items/Weapons/BuildingTools/BuildingItemData_Wall",
            PieceKind::Floor => "/Game/Items/Weapons/BuildingTools/BuildingItemData_Floor",
            PieceKind::Stairs => "/Game/Items/Weapons/BuildingTools/BuildingItemData_Stair_W",
            PieceKind::Roof => "/Game/Items/Weapons/BuildingTools/BuildingItemData_RoofS",
        }
    }
}

/// Building material (SDK `EFortResourceType`: WoodItemData / StoneItemData / MetalItemData).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Material {
    Wood,
    Stone,
    Metal,
}

impl Material {
    pub const ALL: [Material; 3] = [Material::Wood, Material::Stone, Material::Metal];
    /// Next material in the build-mode material cycle (Wood -> Stone -> Metal -> Wood).
    pub fn next(self) -> Material {
        match self {
            Material::Wood => Material::Stone,
            Material::Stone => Material::Metal,
            Material::Metal => Material::Wood,
        }
    }
    /// `EFortResourceType` index (also the index of the `UFortGameData` per-resource handle arrays).
    pub fn index(self) -> usize { match self { Material::Wood => 0, Material::Stone => 1, Material::Metal => 2 } }
    /// `EFortResourceType::<name>` enumerator text as cooked in the class defaults.
    pub fn resource_enum(self) -> &'static str { match self { Material::Wood => "EFortResourceType::Wood", Material::Stone => "EFortResourceType::Stone", Material::Metal => "EFortResourceType::Metal" } }
    /// Row suffix of the `UFortGameData` curve handles (`BuildingCost_Wood` ...).
    pub fn row_suffix(self) -> &'static str { match self { Material::Wood => "Wood", Material::Stone => "Stone", Material::Metal => "Metal" } }
    /// Directory of the player building Blueprints of this material.
    pub fn bp_dir(self) -> &'static str {
        match self { Material::Wood => "FortniteGame/Content/Building/ActorBlueprints/Player/Wood/", Material::Stone => "FortniteGame/Content/Building/ActorBlueprints/Player/Stone/", Material::Metal => "FortniteGame/Content/Building/ActorBlueprints/Player/Metal/" }
    }
}

/// Level the Athena building attributes / game-data curves are evaluated at. The class's `AttributeInitLevelSource` is
/// `PlayerBuildingSkill` = `AFortGameStateZone::PlayerBuildingSkillLevel` (+0x85C), which the constructor zeroes
/// (`AFortGameStateZone::AFortGameStateZone` @1405aeee0). `AFortGameStateAthena` (-> PvP -> Zone) inherits vfunc_271 @1405e0760, which
/// `UFortRegisteredPlayerInfo::Apply/RemoveHomebaseEffects` (@1405fbab0 / @140617fa0) call in BR too; it stores the max
/// `FortConstructionSet.BuildingSkill` of the players' `MyPlayerInfo` ability actors (vfunc_280 @1405c0e80), which only STW hero
/// attribute rows (all 0) initialise, so BR stays at 0 (docs/spec/building_core_gaps.md "PlayerBuildingSkillLevel"); the same 0 is the
/// `CurrentBuildingLevel` of the L1 classes that `UFortGameData::GetBuildingCost` / `GetBuildingInitialHealthPct` receive.
pub const ATHENA_BUILDING_LEVEL: f32 = 0.;

/// 6 faces x 25 bits (5x5) of `UBuildingEditModeMetadata::ConnectivityCubeData` (bit `row * 5 + col`, `FUN_1404b9a20`).
/// Face order as serialized and as used by the exe: 0 = +Y, 1 = -X, 2 = -Y, 3 = +X, 4 = top, 5 = bottom (see `crate::placement`).
pub type ConnectivityCube = [u32; 6];

/// Everything the core needs about one placeable class.
#[derive(Clone, Debug)]
pub struct ClassData {
    /// Object path of the generated class (`/Game/.../PBWA_W1_Solid.PBWA_W1_Solid_C`).
    pub class_path: String,
    pub kind_type: u8,
    pub material: Option<Material>,
    pub current_level: i32,
    pub edit_pattern: String,
    pub static_mesh: String,
    pub attribute_category: String,
    pub attribute_subcategory: String,
    /// Collision convexes (actor-local cm, UE axes).
    pub convexes: Vec<Vec<Vec3>>,
    pub cube: ConnectivityCube,
}

/// Placement numbers of a (piece, material) pair, all from the cooked tables.
#[derive(Clone, Debug)]
pub struct PieceDef {
    pub kind: PieceKind,
    pub material: Material,
    pub class: ClassData,
    pub max_health: f32,
    /// `BuildingInitialHealthPercent_<res>`.
    pub initial_health_pct: f32,
    /// `<Category>.FortBuildingActorSet.BuildTime` (s).
    pub build_time: f32,
    /// `BuildingCost_<res>` (materials per piece).
    pub cost: u32,
}

impl PieceDef {
    /// Health the piece spawns with (`FUN_14046f440`: MaxHealth * InitialHealthPct).
    pub fn start_health(&self) -> f32 { self.max_health * self.initial_health_pct }
}

/// All four pieces in all three materials, plus a cache of any other building class (edit variants) the world uses.
#[derive(Clone, Debug, Default)]
pub struct PieceCatalog {
    defs: Vec<PieceDef>,
    classes: HashMap<String, ClassData>,
    pub notes: Vec<String>,
}

impl PieceCatalog {
    pub fn def(&self, kind: PieceKind, material: Material) -> Option<&PieceDef> { self.defs.iter().find(|d| d.kind == kind && d.material == material) }
    pub fn defs(&self) -> &[PieceDef] { &self.defs }
    /// Class data by class object path (the 12 base classes plus any class added with [`PieceCatalog::ensure_class`]).
    pub fn class(&self, class_path: &str) -> Option<&ClassData> { self.classes.get(class_path) }
    /// Loads (once) a building class that is not one of the 12 base classes (an edit variant), for its collision and cube.
    pub fn ensure_class(&mut self, set: &fn_pak::PakSet, class_path: &str) -> Option<&ClassData> {
        if !self.classes.contains_key(class_path) {
            match load_class(set, class_path) {
                Ok(c) => { self.classes.insert(class_path.to_owned(), c); }
                Err(e) => { self.notes.push(format!("{class_path}: {e}")); return None; }
            }
        }
        self.classes.get(class_path)
    }

    /// Reads every value from the paks (see the module docs for the sources).
    pub fn load(set: &fn_pak::PakSet) -> Result<PieceCatalog, String> {
        let attr = CurveTable::load(set, "/Game/Balance/DataTables/AttributesBuildingSection")?;
        let gd = load_game_data_handles(set)?;
        let mut cat = PieceCatalog::default();
        for material in Material::ALL {
            // candidate classes: every PBWA Blueprint of the material
            let mut candidates: Vec<ClassData> = Vec::new();
            let mut pkgs: Vec<String> = set.effective().map(|(_, e)| e.rel_path().to_owned()).filter(|k| k.starts_with(material.bp_dir()) && k.ends_with(".uasset") && k.contains("/PBWA_")).collect();
            pkgs.sort();
            for p in pkgs {
                let long = fn_pak::pak_path_to_game(&p).unwrap_or_default();
                let name = long.rsplit('/').next().unwrap_or("").to_owned();
                let class_path = format!("{long}.{name}_C");
                match load_class(set, &class_path) { Ok(c) => candidates.push(c), Err(e) => cat.notes.push(format!("{class_path}: {e}")) }
            }
            for kind in PieceKind::ALL {
                let pattern = item_pattern(set, kind)?;
                let hits: Vec<&ClassData> = candidates.iter().filter(|c| c.material == Some(material) && c.kind_type == kind.building_type() && c.current_level == ATHENA_BUILDING_LEVEL as i32 && c.edit_pattern == pattern).collect();
                let class = match hits.as_slice() {
                    [one] => (*one).clone(),
                    [] => return Err(format!("no player-buildable {kind:?} class for {material:?} with pattern {pattern}")),
                    many => return Err(format!("{} classes match {kind:?} {material:?}: {:?}", many.len(), many.iter().map(|c| &c.class_path).collect::<Vec<_>>())),
                };
                let health_row = format!("{}.{}.FortHealthSet.MaxHealth", class.attribute_category, class.attribute_subcategory);
                let time_row = format!("{}.FortBuildingActorSet.BuildTime", class.attribute_category);
                let max_health = attr.eval(&health_row, ATHENA_BUILDING_LEVEL)?;
                let build_time = attr.eval(&time_row, ATHENA_BUILDING_LEVEL)?;
                let initial_health_pct = gd.initial_pct[material.index()].eval(set, ATHENA_BUILDING_LEVEL)?;
                let cost = gd.cost[material.index()].eval(set, ATHENA_BUILDING_LEVEL)?.round() as u32;
                cat.classes.insert(class.class_path.clone(), class.clone());
                cat.defs.push(PieceDef { kind, material, class, max_health, initial_health_pct, build_time, cost });
            }
        }
        Ok(cat)
    }
}

/// `BuildingMetaData` of the slot's item definition (object path, `/Game/.../EMP_Wall_Solid.EMP_Wall_Solid`).
fn item_pattern(set: &fn_pak::PakSet, kind: PieceKind) -> Result<String, String> {
    let pd = fn_uasset::PackageData::load(set, kind.item_definition()).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let i = pkg.exports_of_class("FortBuildingItemDefinition").next().ok_or("no FortBuildingItemDefinition export")?;
    let o = pkg.read_export(i).map_err(|e| e.to_string())?;
    obj_path(&pkg, o.get("BuildingMetaData")).ok_or_else(|| format!("{} has no BuildingMetaData", kind.item_definition()))
}

fn obj_path(pkg: &fn_uasset::Package, v: Option<&fn_uasset::Value>) -> Option<String> {
    match v? {
        fn_uasset::Value::Object(i) if !i.is_null() => Some(pkg.object_path(*i)),
        fn_uasset::Value::SoftObject(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Reads one building class: CDO values (inherited through Blueprint parents is not needed for the PBWA classes: they derive
/// directly from the native `BuildingWall` / `BuildingFloor` / `BuildingStairs` / `BuildingRoof`), mesh convexes, pattern cube.
pub fn load_class(set: &fn_pak::PakSet, class_path: &str) -> Result<ClassData, String> {
    let long = class_path.split('.').next().unwrap_or(class_path);
    let pd = fn_uasset::PackageData::load(set, long).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let class_i = pkg.exports.iter().position(|e| e.name.ends_with("_C") && !e.name.starts_with("Default__")).ok_or("no generated class")?;
    let sup = pkg.object_path(pkg.exports[class_i].super_index);
    let kind_type = match sup.as_str() {
        "/Script/FortniteGame.BuildingWall" => 0,
        "/Script/FortniteGame.BuildingFloor" => 1,
        "/Script/FortniteGame.BuildingStairs" => 5,
        "/Script/FortniteGame.BuildingRoof" => 6,
        other => return Err(format!("parent {other} is not a player building type")),
    };
    let cdo_i = pkg.exports.iter().position(|e| e.name.starts_with("Default__")).ok_or("no class default object")?;
    let cdo = pkg.read_export(cdo_i).map_err(|e| e.to_string())?;
    if cdo.get("bIsPlayerBuildable").and_then(|v| v.as_bool()) != Some(true) { return Err("not bIsPlayerBuildable".into()); }
    let material = match cdo.get("ResourceType").and_then(|v| v.as_str()) {
        Some("EFortResourceType::Wood") => Some(Material::Wood),
        Some("EFortResourceType::Stone") => Some(Material::Stone),
        Some("EFortResourceType::Metal") => Some(Material::Metal),
        _ => None,
    };
    let current_level = cdo.get("CurrentBuildingLevel").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let edit_pattern = obj_path(&pkg, cdo.get("EditModePatternData")).ok_or("no EditModePatternData")?;
    let static_mesh = obj_path(&pkg, cdo.get("StaticMesh")).ok_or("no StaticMesh")?;
    let key = cdo.get("AttributeInitKey");
    let attribute_category = key.and_then(|k| k.field("AttributeInitCategory")).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let attribute_subcategory = key.and_then(|k| k.field("AttributeInitSubCategory")).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let convexes = load_convexes(set, &static_mesh)?;
    let cube = load_cube(set, &edit_pattern)?;
    Ok(ClassData { class_path: class_path.to_owned(), kind_type, material, current_level, edit_pattern, static_mesh, attribute_category, attribute_subcategory, convexes, cube })
}

/// `BodySetup.AggGeom.ConvexElems[].VertexData` of a static mesh package (cm, mesh local; the PBW elements have identity transforms,
/// any element transform is applied anyway).
pub fn load_convexes(set: &fn_pak::PakSet, mesh_path: &str) -> Result<Vec<Vec<Vec3>>, String> {
    let long = mesh_path.split('.').next().unwrap_or(mesh_path);
    let pd = fn_uasset::PackageData::load(set, long).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in pkg.exports_of_class("BodySetup").collect::<Vec<_>>() {
        let o = pkg.read_export(i).map_err(|e| e.to_string())?;
        let Some(elems) = o.get("AggGeom").and_then(|g| g.field("ConvexElems")).and_then(|v| v.as_array()) else { continue };
        for e in elems {
            let verts: Vec<Vec3> = e.field("VertexData").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|v| v.as_vec3()).map(Vec3::from).collect();
            let (rot, tr, sc) = match e.field("Transform") {
                Some(fn_uasset::Value::Transform { rotation, translation, scale }) => (glam::Quat::from_array(*rotation), Vec3::from(*translation), Vec3::from(*scale)),
                Some(t) => (
                    t.field("Rotation").and_then(|v| match v { fn_uasset::Value::Quat(q) => Some(glam::Quat::from_array(*q)), _ => None }).unwrap_or(glam::Quat::IDENTITY),
                    t.field("Translation").and_then(|v| v.as_vec3()).map(Vec3::from).unwrap_or(Vec3::ZERO),
                    t.field("Scale3D").and_then(|v| v.as_vec3()).map(Vec3::from).unwrap_or(Vec3::ONE),
                ),
                None => (glam::Quat::IDENTITY, Vec3::ZERO, Vec3::ONE),
            };
            if verts.len() >= 4 { out.push(verts.into_iter().map(|v| rot * (v * sc) + tr).collect()); }
        }
    }
    if out.is_empty() { return Err(format!("{mesh_path}: no convex collision")); }
    Ok(out)
}

/// `ConnectivityCubeData`: native-serialized, 6 x (`int32 NumBits` = 25, one `uint32` word), decoded by fn-uasset's native
/// `ConnectivityCube` reader (older readers left it `Unparsed` with the raw bytes; both are accepted).
pub fn load_cube(set: &fn_pak::PakSet, pattern_path: &str) -> Result<ConnectivityCube, String> {
    let long = pattern_path.split('.').next().unwrap_or(pattern_path);
    let pd = fn_uasset::PackageData::load(set, long).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    for i in 0..pkg.exports.len() {
        let Ok(o) = pkg.read_export(i) else { continue };
        if let Some(v) = o.get("ConnectivityCubeData") { return parse_cube(v).ok_or_else(|| format!("{pattern_path}: unexpected ConnectivityCubeData layout")); }
    }
    Err(format!("{pattern_path}: no ConnectivityCubeData"))
}

fn parse_cube(v: &fn_uasset::Value) -> Option<ConnectivityCube> {
    // STRUCTURAL FIX (2026-10-05): fn-uasset now reads `FConnectivityCube` natively (`fn_uasset::native`, struct `ConnectivityCube`
    // with one `BitArray { NumBits, Data }` per face in EFortConnectivityCubeFace order); the raw-bytes path below stays as a fallback.
    if let fn_uasset::Value::Struct { name, props } = v {
        if name != "ConnectivityCube" || props.len() != 6 { return None; }
        let mut c = [0u32; 6];
        for (w, p) in c.iter_mut().zip(props.iter()) {
            if p.value.field("NumBits").and_then(|n| n.as_i64()) != Some(25) { return None; }
            match p.value.field("Data").and_then(|d| d.as_array()) { Some([fn_uasset::Value::UInt(x)]) => *w = *x as u32, _ => return None }
        }
        return Some(c);
    }
    let fn_uasset::Value::Unparsed { bytes, .. } = v else { return None };
    if bytes.len() != 48 { return None; }
    let mut c = [0u32; 6];
    for (f, w) in c.iter_mut().enumerate() {
        let n = u32::from_le_bytes(bytes[f * 8..f * 8 + 4].try_into().ok()?);
        if n != 25 { return None; }
        *w = u32::from_le_bytes(bytes[f * 8 + 4..f * 8 + 8].try_into().ok()?);
    }
    Some(c)
}

/// A curve table with the rows' keys (time, value), linear interpolation between keys and constant extrapolation (every key of the
/// building tables is `RCIM_Linear` / `RCCE_Constant`).
pub struct CurveTable { rows: HashMap<String, Vec<(f32, f32)>> }
impl CurveTable {
    pub fn load(set: &fn_pak::PakSet, path: &str) -> Result<CurveTable, String> {
        let pd = fn_uasset::PackageData::load(set, path).map_err(|e| format!("{path}: {e}"))?;
        let pkg = pd.package().map_err(|e| format!("{path}: {e}"))?;
        let tb = fn_uasset::classes::read_table(&pkg, 0).map_err(|e| format!("{path}: {e}"))?;
        let mut rows = HashMap::new();
        for (n, p) in tb.rows {
            let keys: Vec<(f32, f32)> = p.get("Keys").and_then(fn_uasset::Value::as_array).unwrap_or(&[]).iter()
                .filter_map(|k| Some((k.field("Time")?.as_f32()?, k.field("Value")?.as_f32()?))).collect();
            rows.insert(n, keys);
        }
        Ok(CurveTable { rows })
    }
    pub fn eval(&self, row: &str, t: f32) -> Result<f32, String> {
        let k = self.rows.get(row).ok_or_else(|| format!("no curve row {row}"))?;
        eval_keys(k, t).ok_or_else(|| format!("curve row {row} has no keys"))
    }
}
pub fn eval_keys(k: &[(f32, f32)], t: f32) -> Option<f32> {
    let first = k.first()?; let last = k.last()?;
    if t <= first.0 { return Some(first.1); }
    if t >= last.0 { return Some(last.1); }
    for w in k.windows(2) {
        if t >= w[0].0 && t <= w[1].0 { let a = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-9); return Some(w[0].1 + (w[1].1 - w[0].1) * a); }
    }
    Some(last.1)
}

/// `FCurveTableRowHandle` (table object path + row).
#[derive(Clone, Debug)]
pub struct RowHandle { pub table: String, pub row: String }
impl RowHandle {
    pub fn eval(&self, set: &fn_pak::PakSet, t: f32) -> Result<f32, String> { CurveTable::load(set, self.table.split('.').next().unwrap_or(&self.table))?.eval(&self.row, t) }
}
struct GameDataHandles { cost: [RowHandle; 3], initial_pct: [RowHandle; 3] }

/// `UFortGameData` = `/Game/Balance/DefaultGameData` (class `FortGameData`): `BuildingCostHandles[0..3]`, `BuildingInitialHealthPercentHandles[0..3]`.
fn load_game_data_handles(set: &fn_pak::PakSet) -> Result<GameDataHandles, String> {
    let path = "/Game/Balance/DefaultGameData";
    let pd = fn_uasset::PackageData::load(set, path).map_err(|e| format!("{path}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{path}: {e}"))?;
    let o = pkg.read_export(0).map_err(|e| e.to_string())?;
    let handle = |name: &str, idx: usize| -> Result<RowHandle, String> {
        let p = o.props.0.iter().find(|p| p.name == name && p.index as usize == idx).ok_or_else(|| format!("{path}: no {name}[{idx}]"))?;
        let table = obj_path(&pkg, p.value.field("CurveTable")).ok_or_else(|| format!("{name}[{idx}] has no table"))?;
        let row = p.value.field("RowName").and_then(|v| v.as_str()).ok_or_else(|| format!("{name}[{idx}] has no row"))?.to_owned();
        Ok(RowHandle { table, row })
    };
    Ok(GameDataHandles {
        cost: [handle("BuildingCostHandles", 0)?, handle("BuildingCostHandles", 1)?, handle("BuildingCostHandles", 2)?],
        initial_pct: [handle("BuildingInitialHealthPercentHandles", 0)?, handle("BuildingInitialHealthPercentHandles", 1)?, handle("BuildingInitialHealthPercentHandles", 2)?],
    })
}

/// EDITING (additive, editing agent; see `API.md`): the `BuildingInitialHealthPercentHandles[Wood, Stone, Metal]` rows that
/// [`PieceCatalog::load`] uses, so the edit data can evaluate MaxHealth / InitialHealthPct / BuildTime of every edit variant class the
/// same way (`UFortGameData::GetBuildingInitialHealthPct` is keyed by ResourceType only, `FUN_14046f440`).
pub fn initial_health_pct_handles(set: &fn_pak::PakSet) -> Result<[RowHandle; 3], String> { Ok(load_game_data_handles(set)?.initial_pct) }
