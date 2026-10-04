//! Hand-assembled shaders (no old data needed): interpreter semantics of loops, switch, if/else, relative addressing and modifiers,
//! the disassembler, and WGSL validation of the translations.
mod common;
use common::asm::handmade;
use fn_dxbc::codegen::translate_wgsl;
use fn_dxbc::interp::{Machine, NullEnv};
use fn_dxbc::{disassemble, Shader};

fn shader(name: &str) -> (Vec<u8>, Shader) {
    let b = handmade().into_iter().find(|(n, _)| n == name).unwrap().1;
    let s = Shader::parse(&b).unwrap();
    (b, s)
}

#[test]
fn loop_sum() {
    let (_, sh) = shader("h_loop_sum");
    let mut m = Machine::new(&sh);
    let out = m.run(&mut NullEnv).unwrap();
    assert_eq!(out.o[0][0], 45.0);
}

#[test]
fn switch_cases() {
    let (_, sh) = shader("h_switch");
    for (x, want) in [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0), (4.0, 30.0), (0.0, -1.0), (7.0, -1.0)] {
        let mut m = Machine::new(&sh);
        m.set_cb(0, &[[x, 0.0, 0.0, 0.0]]);
        assert_eq!(m.run(&mut NullEnv).unwrap().o[0][0], want, "switch on {x}");
    }
}

#[test]
fn if_else_nested() {
    let (_, sh) = shader("h_if_else");
    for (x, want) in [(0.25, [1.0, 2.0, 3.0, 4.0]), (0.6, [5.0, 6.0, 7.0, 8.0]), (0.9, [9.0, 10.0, 11.0, 12.0])] {
        let mut m = Machine::new(&sh);
        m.set_cb(0, &[[x, 0.0, 0.0, 0.0]]);
        assert_eq!(m.run(&mut NullEnv).unwrap().o[0], want, "x = {x}");
    }
}

#[test]
fn relative_addressing_and_modifiers() {
    let (_, sh) = shader("h_indexable");
    let mut m = Machine::new(&sh);
    // cb0[0] = (0.3, 2, -, -); cb0[r0.x + 1] = cb0[3]
    m.set_cb(0, &[[0.3, 2.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0, 0.0, 0.0, 42.0]]);
    let o = m.run(&mut NullEnv).unwrap().o[0];
    assert_eq!(o[0], 6.0, "x0[2].z = 2 * 3");
    assert_eq!(o[1], 42.0, "cb0[2 + 1].w");
    assert_eq!(o[2], 0.0, "sat(-|0.3| * 2)");
    assert!((o[3] - (-0.3 + 0.25)).abs() < 1e-7);
}

#[test]
fn disassembly_of_a_handmade_program() {
    let (b, _) = shader("h_loop_sum");
    let t = disassemble(&b).unwrap();
    let body: Vec<&str> = t.lines().skip_while(|l| !l.starts_with("ps_5_0")).collect();
    assert_eq!(body[..], ["ps_5_0", "dcl_globalFlags refactoringAllowed", "dcl_output o0.xyzw", "dcl_temps 1", "mov r0.x, l(0)", "mov r0.y, l(0)", "loop ", "  ige r0.z, r0.xxxx, l(10)", "  breakc_nz r0.z", "  iadd r0.y, r0.yyyy, r0.xxxx",
        "  iadd r0.x, r0.xxxx, l(1)", "endloop ", "itof o0.x, r0.yyyy", "ret ", "// Approximately 0 instruction slots used"][..]);
}

#[test]
fn handmade_shaders_translate_to_valid_wgsl() {
    for (name, bytes) in handmade() {
        let sh = Shader::parse(&bytes).unwrap();
        let w = translate_wgsl(&sh, &name).unwrap();
        let module = naga::front::wgsl::parse_str(&w).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&w)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("{name}: {e:?}\n{w}"));
    }
}

