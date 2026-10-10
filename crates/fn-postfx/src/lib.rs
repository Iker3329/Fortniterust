//! fn-postfx: UE 4.16 post effects as wgpu passes with CPU references, plus the evidence tooling that decides which effects are active in 1.8.
pub mod bloom;
pub mod cpu;
pub mod evidence;
pub mod gpu;
pub mod harness;
pub mod params;
pub mod shadercache;
pub mod synth;

