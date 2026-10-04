//! fn-dxbc: pure-Rust DXBC (Shader Model 4/5) toolkit.
//!
//! * [`container`]: DXBC container, checksum, ISGN/OSGN/PCSG signatures, RDEF constant buffers / bindings
//! * [`decode`] + [`ir`] + [`opcode`]: instruction stream decoder and typed IR
//! * [`disasm`]: text listing in the d3dcompiler (`D3DDisassemble`) format
//! * [`interp`]: CPU interpreter (pixel / vertex shaders)
//! * [`codegen`]: IR -> Rust source and IR -> WGSL translators
pub mod analysis;
pub mod codegen;
pub mod container;
pub mod decode;
pub mod disasm;
pub mod extract;
pub mod unreal_packet;
pub mod unreal_entry;
pub mod interp;
pub mod ir;
pub mod opcode;

pub use container::{checksum, resign, Chunk, Container, DxbcError, Rdef, SigElement, Signature};
pub use disasm::{disassemble, disassemble_container, disassemble_hex, DisasmOptions};
pub use ir::*;
pub use opcode::Opcode;

