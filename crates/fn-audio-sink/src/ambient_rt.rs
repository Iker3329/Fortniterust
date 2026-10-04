//! AMBIENCE HOOK: executes the [`fn_audio::ambient::Director`]'s acts on the engine (the director is pure logic; this is its only side-effect layer).
//! Shared between the game thread (`AudioSink::ambient_frame`) and the loader thread (starts the bed when the cues are decoded), behind a mutex.

use fn_audio::ambient::{Act, Director, Frame, Key, Layout, WorldMap};
use fn_audio::mixer::{EngineHandle, PlayId, PlayParams, Vec3};
use fn_audio::soundclass::SoundMix;
use std::collections::HashMap;

pub struct AmbientRt {
    pub dir: Director,
    keys: HashMap<Key, PlayId>,
    /// `Fort_InsideAmbient_Athena_MixMod` (pushed / popped by the bed's "surrounded" test).
    pub mix: Option<SoundMix>,
}

impl Default for AmbientRt {
    fn default() -> Self {
        AmbientRt { dir: Director::new(Layout::embedded()), keys: HashMap::new(), mix: None }
    }
}

impl AmbientRt {
    pub fn set_map(&mut self, center_ue_cm: [f32; 3], anchor_cm: [f32; 3]) {
        self.dir.map = WorldMap { center_ue_cm, anchor_cm };
    }

    /// Execute the acts on `h`.
    pub fn exec(&mut self, h: &EngineHandle, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Play { key, cue, position, volume, pitch, adjust } => {
                    if let Ok(id) = h.play_cue(&cue, PlayParams { position, volume, pitch, ..Default::default() }) {
                        h.adjust_play(id, adjust);
                        self.keys.insert(key, id);
                    }
                }
                Act::Adjust { key, adjust } => {
                    if let Some(&id) = self.keys.get(&key) {
                        let stops = adjust.gain.map_or(false, |g| g.3);
                        h.adjust_play(id, adjust);
                        if stops {
                            self.keys.remove(&key); // the mixer ends the play at silence
                        }
                    }
                }
                Act::Stop { key } => {
                    if let Some(id) = self.keys.remove(&key) {
                        h.stop(id);
                    }
                }
                Act::PushMix(_) => {
                    if let Some(m) = &self.mix {
                        h.push_mix(m.clone());
                    }
                }
                Act::PopMix(_) => {
                    if let Some(m) = &self.mix {
                        h.pop_mix(&m.name);
                    }
                }
            }
        }
    }

    /// One director step + execution.
    pub fn step(&mut self, h: &EngineHandle, dt: f32, listener: Vec3, pawn: Vec3, forward: Vec3, trace: &mut dyn FnMut(Vec3, Vec3, f32) -> bool) {
        let acts = self.dir.update(&mut Frame { dt, listener, pawn, forward, trace });
        self.exec(h, acts);
    }

    /// Enable / disable the whole system (`BeginPlay` / `EndPlay`) and execute the transition at once (no traces, no time passing).
    pub fn enable(&mut self, h: &EngineHandle, on: bool, at: Vec3, forward: Vec3) {
        self.dir.set_enabled(on);
        let mut none = |_: Vec3, _: Vec3, _: f32| false;
        self.step(h, 0.0, at, at, forward, &mut none);
    }
}
