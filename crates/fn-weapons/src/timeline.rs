//! Serialized original montage metadata. No replacement animation playback implementation.
#[derive(Clone, Debug, PartialEq)]
pub struct Notify { pub name: String, pub time: f32, pub duration: f32, pub object: Option<String> }
#[derive(Clone, Debug, PartialEq)]
pub struct Segment { pub slot: String, pub sequence: String, pub montage_start: f32, pub anim_start: f32, pub anim_end: f32, pub play_rate: f32, pub loops: u32 }
#[derive(Clone, Debug, PartialEq)]
pub struct Montage { pub package: String, pub length: f32, pub blend_in: Option<f32>, pub blend_out: Option<f32>, pub segments: Vec<Segment>, pub notifies: Vec<Notify> }
