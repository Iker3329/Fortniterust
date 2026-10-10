use crate::events::PropSound;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub struct ClassRow {
    pub class: &'static str,
    pub hit: Option<&'static str>,
    pub death: Option<&'static str>,
    pub damage: Option<&'static str>,
    pub container_death: Option<&'static str>,
    pub slot: u8,
    pub attach: u8,
    pub radius: f32,
    pub offset: [f32; 3],
    pub grid: Option<bool>,
    pub resource: u8,
    pub surface: u8,
}

pub use crate::demolition_data::{ATTACH_LINKS, CLASSES, CUES, GENERIC_DEATH_SOUNDS, MATERIAL_SURFACES, MESH_SURFACES, PICKAXE_IMPACT_SOUNDS};

fn short(p: &str) -> &str { p.split('.').next().unwrap_or(p) }

pub fn class(name: &str) -> Option<&'static ClassRow> {
    static M: OnceLock<HashMap<&'static str, &'static ClassRow>> = OnceLock::new();
    let name = name.trim_end_matches("_C");
    M.get_or_init(|| CLASSES.iter().map(|c| (c.class, c)).collect()).get(name).copied()
}

pub fn mesh_surface(mesh: &str) -> Option<(u8, bool)> {
    static M: OnceLock<HashMap<String, (u8, bool)>> = OnceLock::new();
    M.get_or_init(|| MESH_SURFACES.iter().map(|(m, s, b)| (m.to_ascii_lowercase(), (*s, *b))).collect()).get(&short(mesh).to_ascii_lowercase()).copied()
}

pub fn material_surface(mat: &str) -> u8 {
    MATERIAL_SURFACES.iter().find(|(m, _)| m.eq_ignore_ascii_case(short(mat))).map_or(0, |x| x.1)
}

pub fn surface_of(mesh: &str, override0: Option<&str>) -> u8 {
    match mesh_surface(mesh) {
        Some((s, true)) => s,
        other => match override0 { Some(m) => material_surface(m), None => other.map_or(0, |x| x.0) },
    }
}

pub fn impact_sound(surface: u8) -> Option<&'static str> { PICKAXE_IMPACT_SOUNDS.get(surface as usize).copied().flatten() }

pub fn generic_death(resource: u8) -> Option<&'static str> { GENERIC_DEATH_SOUNDS.get(resource as usize).copied().filter(|s| !s.is_empty()) }

pub fn death_sound(row: Option<&ClassRow>, resource: u8) -> Option<&'static str> { row.and_then(|r| r.death).or_else(|| generic_death(resource)) }

pub fn can_attach(child_type: u8, parent_slot: u8) -> bool {
    match child_type {
        0 => parent_slot == 0,
        1 | 3 => parent_slot == 1,
        2 => parent_slot == 0 || parent_slot == 2,
        4 => parent_slot != 3,
        5 => parent_slot < 2,
        _ => false,
    }
}

pub fn prop_sound(class_name: &str, surface: u8, resource: u8) -> &'static PropSound {
    static CACHE: OnceLock<Mutex<HashMap<(String, u8, u8), &'static PropSound>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    let key = (class_name.trim_end_matches("_C").to_owned(), surface, resource);
    if let Some(p) = c.get(&key) { return p; }
    let row = class(&key.0);
    let p: &'static PropSound = Box::leak(Box::new(PropSound {
        id: Box::leak(key.0.clone().into_boxed_str()),
        hit: row.and_then(|r| r.hit),
        death: death_sound(row, resource),
        impact: impact_sound(surface),
        damage: row.and_then(|r| r.damage),
        container_death: row.and_then(|r| r.container_death),
    }));
    c.insert(key, p);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pickaxe_surfaces_follow_the_inherited_array() {
        assert!(impact_sound(0).unwrap().ends_with("PickAxe_Impact_Default_Cue"));
        assert!(impact_sound(1).unwrap().ends_with("Demo_Wood_Impact_MLTI_Cue"));
        assert!(impact_sound(2).unwrap().ends_with("Demo_Rock_Impact_MLTI_Cue"));
        assert!(impact_sound(3).unwrap().ends_with("Demo_Metal_Impact_MLTI_Cue"));
    }
    #[test]
    fn generic_death_by_resource_and_none_for_no_resource() {
        assert!(generic_death(0).unwrap().ends_with("Demo_Wood_ImpactFinal_Cue"));
        assert!(generic_death(1).unwrap().ends_with("Demo_Rock_ImpactFinal_Cue"));
        assert!(generic_death(2).unwrap().ends_with("Demo_Metal_ImpactFinal_Cue"));
        assert_eq!(generic_death(4), None);
    }
    #[test]
    fn can_attach_matches_the_exe_switch() {
        assert!(can_attach(0, 0) && !can_attach(0, 1) && !can_attach(0, 3));
        assert!(can_attach(2, 2) && can_attach(2, 0) && !can_attach(2, 1));
        assert!(can_attach(4, 2) && !can_attach(4, 3));
        assert!(can_attach(5, 1) && !can_attach(5, 2));
        assert!(!can_attach(6, 0));
    }
    #[test]
    fn every_table_cue_is_listed() {
        for r in CLASSES { for c in [r.hit, r.death, r.damage, r.container_death].into_iter().flatten() { assert!(CUES.contains(&c), "{c}"); } }
        for c in PICKAXE_IMPACT_SOUNDS.iter().flatten() { assert!(CUES.contains(c), "{c}"); }
    }
}
