//! `catalog.json` + per-kind `props.json`.
use crate::collision::Collision;
use crate::material::Material;
use crate::mesh::{load_lods, LodMesh};
use crate::layout::Layout;
use crate::util::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a prop yields when harvested with the pickaxe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKind {
    Wood,
    Stone,
    Metal,
    /// Not harvestable / unknown.
    None,
}

impl ResourceKind {
    pub fn parse(s: &str) -> ResourceKind {
        match s.trim_start_matches("EFortResourceType::") {
            "Wood" => ResourceKind::Wood,
            "Stone" => ResourceKind::Stone,
            "Metal" => ResourceKind::Metal,
            _ => ResourceKind::None,
        }
    }
}

/// Harvest data. The two curves (`ResourceRates` and `AthenaResourceRates`, row `amount_row`) were read from the cooked curve
/// tables: key at time 0 and time 1. Their *meaning* (per hit vs. total, how time maps to damage) is evaluated by native
/// `FortBuildingActor` code that is not in the cooked data; the values are exposed unmodified.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceInfo {
    pub kind: ResourceKind,
    pub amount_row: Option<String>,
    /// `ResourceRates` row: `(value at t = 0, value at t = 1)`.
    pub rates: Option<(f32, f32)>,
    /// `AthenaResourceRates` row (the table the Battle Royale mode is meant to use).
    pub athena_rates: Option<(f32, f32)>,
}

/// Destruction / feedback data of the Blueprint class default object. Paths are UE object paths into the pak (the particle
/// systems / sound cues themselves are not exported by this target).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Destruction {
    pub death_particles: Option<String>,
    pub death_sound: Option<String>,
    pub hit_sound: Option<String>,
    pub loot_tier_group: Option<String>,
    /// Blueprint class that replaces the actor when destroyed, when the class names one (`None` for all 1.8 trees / rocks:
    /// the actor is removed and a particle system plays).
    pub replacement_class: Option<String>,
}

/// Render parameters of the Blueprint (scale / yaw randomisation, draw distance, material wind controls).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderInfo {
    /// `StaticMeshComponent.CachedMaxDrawDistance` / `LDMaxDrawDistance` in cm (0 = unlimited).
    pub cull_distance_cm: f32,
    pub random_scale: bool,
    pub random_scale_range: Option<(f32, f32)>,
    pub random_yaw: bool,
    pub cast_shadow: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LodInfo {
    pub vertices: usize,
    pub triangles: usize,
    pub screen_size: f32,
    pub uv_channels: usize,
    pub has_vertex_colors: bool,
    /// `(material_slot, triangles)` per section.
    pub sections: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSlot {
    pub slot: usize,
    pub slot_name: String,
    pub name: String,
    /// Path of the `.mat.json` relative to the kind folder.
    pub mat_json: String,
    /// `mesh` (the static mesh's own slot) or `component_override` (Blueprint `OverrideMaterials`).
    pub source: String,
}

/// One harvestable prop class.
#[derive(Debug, Clone)]
pub struct PropKind {
    /// Blueprint class name without `_C`, e.g. `Athena_Tree_Pine_3` (the key used by layouts).
    pub id: String,
    pub display: String,
    /// `tree`, `rock`, `bush`, `hedge`, `vehicle`, `container`, `fence`, `furniture`, `crate`, `pallet` or `other`.
    pub category: String,
    /// `/Game/...` package of the Blueprint.
    pub blueprint: String,
    pub mesh_name: String,
    pub mesh_ue_path: String,
    /// `FBoxSphereBounds` of the mesh in cm (UE local space).
    pub bounds_origin_cm: [f32; 3],
    pub bounds_extent_cm: [f32; 3],
    pub lods: Vec<LodInfo>,
    pub materials: Vec<MaterialSlot>,
    pub collision: Collision,
    /// `FortHealthSet.MaxHealth` of the prop (`AttributesBuildingProps` row at curve time 0).
    pub health: f32,
    /// The `AttributesBuildingProps` row name the health came from.
    pub health_row: String,
    pub resource: ResourceInfo,
    pub destruction: Destruction,
    pub render: RenderInfo,
    /// Placements of this class in `Athena_Terrain` (persistent level + sublevels).
    pub placed_in_map: u32,
    /// The whole `props.json`, for fields this struct does not model (`ue_properties`, `verification`, wind, ...).
    pub raw: Value,
    dir: PathBuf,
    gltf: String,
}

impl PropKind {
    /// Folder holding this kind's `props.json`, glTF, `materials/`, `collision.json`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn gltf_path(&self) -> PathBuf {
        self.dir.join(&self.gltf)
    }

    /// Decode every LOD of the glTF (LOD 0 first).
    pub fn load_lods(&self) -> Result<Vec<LodMesh>> {
        load_lods(&self.gltf_path())
    }

    /// Decode the `.mat.json` of material slot `slot` (textures are not decoded until requested).
    pub fn load_material(&self, slot: usize) -> Result<Material> {
        let m = self.materials.iter().find(|m| m.slot == slot).ok_or_else(|| Error::Schema(format!("{}: no material slot {slot}", self.id)))?;
        Material::load(&self.dir.join(&m.mat_json))
    }

    pub fn is_tree(&self) -> bool {
        self.category == "tree"
    }
    pub fn is_rock(&self) -> bool {
        self.category == "rock"
    }

    /// Height of the mesh bounds in metres (UE Z extent).
    pub fn height_m(&self) -> f32 {
        (self.bounds_origin_cm[2] + self.bounds_extent_cm[2]) * 0.01
    }

    fn from_json(dir: &Path, j: &Value) -> Result<PropKind> {
        let id = req_str(j, "id", "props.json")?;
        let mesh = j.get("mesh").ok_or_else(|| Error::Schema(format!("{id}: no mesh")))?;
        let bounds = mesh.get("bounds_cm");
        let mut lods = vec![];
        for l in mesh.get("lods").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            lods.push(LodInfo {
                vertices: u32_of(l, "vertices").unwrap_or(0) as usize,
                triangles: u32_of(l, "triangles").unwrap_or(0) as usize,
                screen_size: f32_of(l, "screen_size").unwrap_or(0.0),
                uv_channels: u32_of(l, "uv_channels").unwrap_or(0) as usize,
                has_vertex_colors: bool_of(l, "vertex_colors").unwrap_or(false),
                sections: l
                    .get("sections")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().map(|s| (u32_of(s, "material_slot").unwrap_or(0) as usize, u32_of(s, "triangles").unwrap_or(0) as usize)).collect())
                    .unwrap_or_default(),
            });
        }
        let mut materials = vec![];
        for m in j.get("materials").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            materials.push(MaterialSlot {
                slot: u32_of(m, "slot").unwrap_or(0) as usize,
                slot_name: str_of(m, "slot_name").unwrap_or_default(),
                name: str_of(m, "name").unwrap_or_default(),
                mat_json: req_str(m, "mat_json", &format!("{id}.materials"))?,
                source: str_of(m, "source").unwrap_or_default(),
            });
        }
        let collision_file = str_of(j, "collision").unwrap_or_else(|| "collision.json".into());
        let collision = Collision::load(&dir.join(&collision_file))?;
        let h = j.get("health");
        let res = j.get("resource");
        let pair = |v: Option<&Value>, a: &str, b: &str| -> Option<(f32, f32)> { Some((f32_of(v?, a)?, f32_of(v?, b)?)) };
        let d = j.get("destruction");
        let ds = |k: &str| d.and_then(|d| str_of(d, k)).filter(|s| !s.is_empty());
        let r = j.get("render");
        Ok(PropKind {
            display: str_of(j, "display").unwrap_or_else(|| id.replace('_', " ")),
            category: str_of(j, "category").unwrap_or_else(|| "other".into()),
            blueprint: str_of(j, "blueprint").unwrap_or_default(),
            mesh_name: str_of(mesh, "name").unwrap_or_default(),
            mesh_ue_path: str_of(mesh, "ue_path").unwrap_or_default(),
            bounds_origin_cm: bounds.and_then(|b| arr_f32::<3>(b.get("origin"))).unwrap_or([0.0; 3]),
            bounds_extent_cm: bounds.and_then(|b| arr_f32::<3>(b.get("extent"))).unwrap_or([0.0; 3]),
            lods,
            materials,
            collision,
            health: h.and_then(|h| f32_of(h, "max")).unwrap_or(0.0),
            health_row: h.and_then(|h| str_of(h, "attribute_row")).unwrap_or_default(),
            resource: ResourceInfo {
                kind: ResourceKind::parse(&res.and_then(|r| str_of(r, "type")).unwrap_or_default()),
                amount_row: res.and_then(|r| str_of(r, "amount_row")).filter(|s| !s.is_empty()),
                rates: pair(res, "rate_t0", "rate_t1"),
                athena_rates: pair(res, "athena_rate_t0", "athena_rate_t1"),
            },
            destruction: Destruction { death_particles: ds("death_particles"), death_sound: ds("death_sound"), hit_sound: ds("hit_sound"), loot_tier_group: ds("loot_tier_group"), replacement_class: ds("replacement_class") },
            render: RenderInfo {
                cull_distance_cm: r.and_then(|r| f32_of(r, "cull_distance_cm")).unwrap_or(0.0),
                random_scale: r.and_then(|r| bool_of(r, "random_scale")).unwrap_or(false),
                random_scale_range: r.and_then(|r| arr_f32::<2>(r.get("random_scale_range"))).map(|a| (a[0], a[1])),
                random_yaw: r.and_then(|r| bool_of(r, "random_yaw")).unwrap_or(false),
                cast_shadow: r.and_then(|r| bool_of(r, "cast_shadow")).unwrap_or(true),
            },
            placed_in_map: j.get("placement").and_then(|p| u32_of(p, "athena_terrain_count")).unwrap_or(0),
            gltf: str_of(mesh, "gltf").ok_or_else(|| Error::Schema(format!("{id}: mesh.gltf")))?,
            dir: dir.to_path_buf(),
            raw: j.clone(),
            id,
        })
    }
}

/// The exported prop set (`assets/props/`).
#[derive(Debug, Clone)]
pub struct PropCatalog {
    pub dir: PathBuf,
    pub kinds: Vec<PropKind>,
    by_id: BTreeMap<String, usize>,
}

impl PropCatalog {
    /// Load `<dir>/catalog.json` and every kind's `props.json` + `collision.json`. Meshes, materials and textures are
    /// decoded lazily through the [`PropKind`] methods.
    pub fn load(dir: impl AsRef<Path>) -> Result<PropCatalog> {
        let dir = dir.as_ref().to_path_buf();
        let cat = read_json(&dir.join("catalog.json"))?;
        let schema_v = cat.get("schema").and_then(|x| x.as_u64()).unwrap_or(0);
        if schema_v != 1 {
            return schema(format!("catalog.json schema {schema_v}, this loader reads 1"));
        }
        let mut kinds = vec![];
        for e in cat.get("kinds").and_then(|x| x.as_array()).ok_or_else(|| Error::Schema("catalog: kinds".into()))? {
            let id = req_str(e, "id", "catalog kind")?;
            let sub = dir.join(req_str(e, "dir", &id)?);
            let j = read_json(&sub.join("props.json"))?;
            kinds.push(PropKind::from_json(&sub, &j)?);
        }
        let by_id = kinds.iter().enumerate().map(|(i, k)| (k.id.clone(), i)).collect();
        Ok(PropCatalog { dir, kinds, by_id })
    }

    pub fn get(&self, id: &str) -> Option<&PropKind> {
        self.by_id.get(id).map(|&i| &self.kinds[i])
    }

    pub fn by_category<'a>(&'a self, category: &'a str) -> impl Iterator<Item = &'a PropKind> + 'a {
        self.kinds.iter().filter(move |k| k.category == category)
    }

    pub fn by_resource(&self, kind: ResourceKind) -> impl Iterator<Item = &PropKind> + '_ {
        self.kinds.iter().filter(move |k| k.resource.kind == kind)
    }

    /// `layout/sample.json`: the 40-instance sample around a dense tree + rock cluster of the Athena map.
    pub fn load_sample_layout(&self) -> Result<Layout> {
        Layout::load_sample(&self.dir.join("layout/sample.json"))
    }

    /// `layout/all.json`: every harvestable placement of `Athena_Terrain`.
    pub fn load_full_layout(&self) -> Result<Layout> {
        Layout::load_all(&self.dir.join("layout/all.json"))
    }
}
