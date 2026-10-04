//! SM4/SM5 opcode table: number, d3dcompiler mnemonic, number of destination and source operands.
//! Declarations (`dcl_*`) list their operands under `dst` = 0 and are decoded by `decode.rs` with their own trailing dwords.

macro_rules! opcodes {
    ($( $num:literal $var:ident $name:literal $dst:literal $src:literal; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Opcode { $( $var, )* Unknown(u16) }
        impl Opcode {
            pub fn from_u16(n: u16) -> Opcode { match n { $( $num => Opcode::$var, )* _ => Opcode::Unknown(n) } }
            pub fn to_u16(self) -> u16 { match self { $( Opcode::$var => $num, )* Opcode::Unknown(n) => n } }
            /// d3dcompiler mnemonic (without `_sat`, `_nz`, `_indexable(..)` decorations).
            pub fn name(self) -> &'static str { match self { $( Opcode::$var => $name, )* Opcode::Unknown(_) => "unknown" } }
            /// (number of destination operands, number of source operands) as laid out in the token stream.
            pub fn arity(self) -> (usize, usize) { match self { $( Opcode::$var => ($dst, $src), )* Opcode::Unknown(_) => (0, 0) } }
        }
    };
}

opcodes! {
    0 Add "add" 1 2;
    1 And "and" 1 2;
    2 Break "break" 0 0;
    3 Breakc "breakc" 0 1;
    4 Call "call" 0 1;
    5 Callc "callc" 0 2;
    6 Case "case" 0 1;
    7 Continue "continue" 0 0;
    8 Continuec "continuec" 0 1;
    9 Cut "cut" 0 0;
    10 Default "default" 0 0;
    11 DerivRtx "deriv_rtx" 1 1;
    12 DerivRty "deriv_rty" 1 1;
    13 Discard "discard" 0 1;
    14 Div "div" 1 2;
    15 Dp2 "dp2" 1 2;
    16 Dp3 "dp3" 1 2;
    17 Dp4 "dp4" 1 2;
    18 Else "else" 0 0;
    19 Emit "emit" 0 0;
    20 EmitThenCut "emitthen_cut" 0 0;
    21 EndIf "endif" 0 0;
    22 EndLoop "endloop" 0 0;
    23 EndSwitch "endswitch" 0 0;
    24 Eq "eq" 1 2;
    25 Exp "exp" 1 1;
    26 Frc "frc" 1 1;
    27 Ftoi "ftoi" 1 1;
    28 Ftou "ftou" 1 1;
    29 Ge "ge" 1 2;
    30 Iadd "iadd" 1 2;
    31 If "if" 0 1;
    32 Ieq "ieq" 1 2;
    33 Ige "ige" 1 2;
    34 Ilt "ilt" 1 2;
    35 Imad "imad" 1 3;
    36 Imax "imax" 1 2;
    37 Imin "imin" 1 2;
    38 Imul "imul" 2 2;
    39 Ine "ine" 1 2;
    40 Ineg "ineg" 1 1;
    41 Ishl "ishl" 1 2;
    42 Ishr "ishr" 1 2;
    43 Itof "itof" 1 1;
    44 Label "label" 0 1;
    45 Ld "ld" 1 2;
    46 LdMs "ld_ms" 1 3;
    47 Log "log" 1 1;
    48 Loop "loop" 0 0;
    49 Lt "lt" 1 2;
    50 Mad "mad" 1 3;
    51 Min "min" 1 2;
    52 Max "max" 1 2;
    53 CustomData "customdata" 0 0;
    54 Mov "mov" 1 1;
    55 Movc "movc" 1 3;
    56 Mul "mul" 1 2;
    57 Ne "ne" 1 2;
    58 Nop "nop" 0 0;
    59 Not "not" 1 1;
    60 Or "or" 1 2;
    61 Resinfo "resinfo" 1 2;
    62 Ret "ret" 0 0;
    63 Retc "retc" 0 1;
    64 RoundNe "round_ne" 1 1;
    65 RoundNi "round_ni" 1 1;
    66 RoundPi "round_pi" 1 1;
    67 RoundZ "round_z" 1 1;
    68 Rsq "rsq" 1 1;
    69 Sample "sample" 1 3;
    70 SampleC "sample_c" 1 4;
    71 SampleCLz "sample_c_lz" 1 4;
    72 SampleL "sample_l" 1 4;
    73 SampleD "sample_d" 1 5;
    74 SampleB "sample_b" 1 4;
    75 Sqrt "sqrt" 1 1;
    76 Switch "switch" 0 1;
    77 Sincos "sincos" 2 1;
    78 Udiv "udiv" 2 2;
    79 Ult "ult" 1 2;
    80 Uge "uge" 1 2;
    81 Umul "umul" 2 2;
    82 Umad "umad" 1 3;
    83 Umax "umax" 1 2;
    84 Umin "umin" 1 2;
    85 Ushr "ushr" 1 2;
    86 Utof "utof" 1 1;
    87 Xor "xor" 1 2;
    88 DclResource "dcl_resource" 0 0;
    89 DclConstantBuffer "dcl_constantbuffer" 0 0;
    90 DclSampler "dcl_sampler" 0 0;
    91 DclIndexRange "dcl_indexRange" 0 0;
    92 DclGsOutputPrimitiveTopology "dcl_outputtopology" 0 0;
    93 DclGsInputPrimitive "dcl_inputprimitive" 0 0;
    94 DclMaxOutputVertexCount "dcl_maxout" 0 0;
    95 DclInput "dcl_input" 0 0;
    96 DclInputSgv "dcl_input_sgv" 0 0;
    97 DclInputSiv "dcl_input_siv" 0 0;
    98 DclInputPs "dcl_input_ps" 0 0;
    99 DclInputPsSgv "dcl_input_ps_sgv" 0 0;
    100 DclInputPsSiv "dcl_input_ps_siv" 0 0;
    101 DclOutput "dcl_output" 0 0;
    102 DclOutputSgv "dcl_output_sgv" 0 0;
    103 DclOutputSiv "dcl_output_siv" 0 0;
    104 DclTemps "dcl_temps" 0 0;
    105 DclIndexableTemp "dcl_indexableTemp" 0 0;
    106 DclGlobalFlags "dcl_globalFlags" 0 0;
    108 Lod "lod" 1 3;
    109 Gather4 "gather4" 1 3;
    110 SamplePos "samplepos" 1 2;
    111 SampleInfo "sampleinfo" 1 1;
    113 HsDecls "hs_decls" 0 0;
    114 HsControlPointPhase "hs_control_point_phase" 0 0;
    115 HsForkPhase "hs_fork_phase" 0 0;
    116 HsJoinPhase "hs_join_phase" 0 0;
    117 EmitStream "emit_stream" 0 1;
    118 CutStream "cut_stream" 0 1;
    119 EmitThenCutStream "emitthen_cut_stream" 0 1;
    120 InterfaceCall "fcall" 0 1;
    121 Bufinfo "bufinfo" 1 1;
    122 DerivRtxCoarse "deriv_rtx_coarse" 1 1;
    123 DerivRtxFine "deriv_rtx_fine" 1 1;
    124 DerivRtyCoarse "deriv_rty_coarse" 1 1;
    125 DerivRtyFine "deriv_rty_fine" 1 1;
    126 Gather4C "gather4_c" 1 4;
    127 Gather4Po "gather4_po" 1 4;
    128 Gather4PoC "gather4_po_c" 1 5;
    129 Rcp "rcp" 1 1;
    130 F32toF16 "f32tof16" 1 1;
    131 F16toF32 "f16tof32" 1 1;
    132 Uaddc "uaddc" 2 2;
    133 Usubb "usubb" 2 2;
    134 Countbits "countbits" 1 1;
    135 FirstbitHi "firstbit_hi" 1 1;
    136 FirstbitLo "firstbit_lo" 1 1;
    137 FirstbitShi "firstbit_shi" 1 1;
    138 Ubfe "ubfe" 1 3;
    139 Ibfe "ibfe" 1 3;
    140 Bfi "bfi" 1 4;
    141 Bfrev "bfrev" 1 1;
    142 Swapc "swapc" 2 3;
    143 DclStream "dcl_stream" 0 0;
    144 DclFunctionBody "dcl_function_body" 0 0;
    145 DclFunctionTable "dcl_function_table" 0 0;
    146 DclInterface "dcl_interface" 0 0;
    147 DclInputControlPointCount "dcl_input_control_point_count" 0 0;
    148 DclOutputControlPointCount "dcl_output_control_point_count" 0 0;
    149 DclTessDomain "dcl_tessellator_domain" 0 0;
    150 DclTessPartitioning "dcl_tessellator_partitioning" 0 0;
    151 DclTessOutputPrimitive "dcl_tessellator_output_primitive" 0 0;
    152 DclHsMaxTessFactor "dcl_hs_max_tessfactor" 0 0;
    153 DclHsForkPhaseInstanceCount "dcl_hs_fork_phase_instance_count" 0 0;
    154 DclHsJoinPhaseInstanceCount "dcl_hs_join_phase_instance_count" 0 0;
    155 DclThreadGroup "dcl_thread_group" 0 0;
    156 DclUavTyped "dcl_uav_typed" 0 0;
    157 DclUavRaw "dcl_uav_raw" 0 0;
    158 DclUavStructured "dcl_uav_structured" 0 0;
    159 DclTgsmRaw "dcl_tgsm_raw" 0 0;
    160 DclTgsmStructured "dcl_tgsm_structured" 0 0;
    161 DclResourceRaw "dcl_resource_raw" 0 0;
    162 DclResourceStructured "dcl_resource_structured" 0 0;
    163 LdUavTyped "ld_uav_typed" 1 2;
    164 StoreUavTyped "store_uav_typed" 1 2;
    165 LdRaw "ld_raw" 1 2;
    166 StoreRaw "store_raw" 1 2;
    167 LdStructured "ld_structured" 1 3;
    168 StoreStructured "store_structured" 1 3;
    169 AtomicAnd "atomic_and" 1 2;
    170 AtomicOr "atomic_or" 1 2;
    171 AtomicXor "atomic_xor" 1 2;
    172 AtomicCmpStore "atomic_cmp_store" 1 3;
    173 AtomicIadd "atomic_iadd" 1 2;
    174 AtomicImax "atomic_imax" 1 2;
    175 AtomicImin "atomic_imin" 1 2;
    176 AtomicUmax "atomic_umax" 1 2;
    177 AtomicUmin "atomic_umin" 1 2;
    178 ImmAtomicAlloc "imm_atomic_alloc" 1 1;
    179 ImmAtomicConsume "imm_atomic_consume" 1 1;
    180 ImmAtomicIadd "imm_atomic_iadd" 2 2;
    181 ImmAtomicAnd "imm_atomic_and" 2 2;
    182 ImmAtomicOr "imm_atomic_or" 2 2;
    183 ImmAtomicXor "imm_atomic_xor" 2 2;
    184 ImmAtomicExch "imm_atomic_exch" 2 2;
    185 ImmAtomicCmpExch "imm_atomic_cmp_exch" 2 3;
    186 ImmAtomicImax "imm_atomic_imax" 2 2;
    187 ImmAtomicImin "imm_atomic_imin" 2 2;
    188 ImmAtomicUmax "imm_atomic_umax" 2 2;
    189 ImmAtomicUmin "imm_atomic_umin" 2 2;
    190 Sync "sync" 0 0;
    191 Dadd "dadd" 1 2;
    192 Dmax "dmax" 1 2;
    193 Dmin "dmin" 1 2;
    194 Dmul "dmul" 1 2;
    195 Deq "deq" 1 2;
    196 Dge "dge" 1 2;
    197 Dlt "dlt" 1 2;
    198 Dne "dne" 1 2;
    199 Dmov "dmov" 1 1;
    200 Dmovc "dmovc" 1 3;
    201 Dtof "dtof" 1 1;
    202 Ftod "ftod" 1 1;
    203 EvalSnapped "eval_snapped" 1 2;
    204 EvalSampleIndex "eval_sample_index" 1 2;
    205 EvalCentroid "eval_centroid" 1 1;
    206 DclGsInstanceCount "dcl_gsinstances" 0 0;
    207 Abort "abort" 0 0;
    208 DebugBreak "debug_break" 0 0;
    210 Ddiv "ddiv" 1 2;
    211 Dfma "dfma" 1 3;
    212 Drcp "drcp" 1 1;
    213 Msad "msad" 1 3;
    214 Dtoi "dtoi" 1 1;
    215 Dtou "dtou" 1 1;
    216 Itod "itod" 1 1;
    217 Utod "utod" 1 1;
}

impl Opcode {
    pub fn is_decl(self) -> bool {
        use Opcode::*;
        matches!(self, DclResource | DclConstantBuffer | DclSampler | DclIndexRange | DclGsOutputPrimitiveTopology | DclGsInputPrimitive | DclMaxOutputVertexCount | DclInput | DclInputSgv | DclInputSiv | DclInputPs | DclInputPsSgv | DclInputPsSiv | DclOutput | DclOutputSgv | DclOutputSiv | DclTemps | DclIndexableTemp | DclGlobalFlags | DclStream | DclFunctionBody | DclFunctionTable | DclInterface | DclInputControlPointCount | DclOutputControlPointCount | DclTessDomain | DclTessPartitioning | DclTessOutputPrimitive | DclHsMaxTessFactor | DclHsForkPhaseInstanceCount | DclHsJoinPhaseInstanceCount | DclThreadGroup | DclUavTyped | DclUavRaw | DclUavStructured | DclTgsmRaw | DclTgsmStructured | DclResourceRaw | DclResourceStructured | DclGsInstanceCount)
    }
}
