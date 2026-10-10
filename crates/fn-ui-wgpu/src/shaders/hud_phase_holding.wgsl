fn ps_raw(v: array<vec4<f32>, 12>) -> array<vec4<f32>, 1> {
    var r0 = vec4<f32>(0.0);
    var r1 = vec4<f32>(0.0);
    var r2 = vec4<f32>(0.0);
    var r3 = vec4<f32>(0.0);
    var o0 = vec4<f32>(0.0);
    {
        let q0 = (-v[3].x) + v[3].z;
        let q1 = (-v[3].y) + v[3].w;
        r0.x = q0;
        r0.y = (-v[3].x) + v[3].z;
        r0.z = q1;
        r0.w = (-v[3].y) + v[3].w;
    }
    {
        let q2 = r0.z * v[4].x;
        let q3 = r0.w * v[4].z;
        r0.z = q2;
        r0.w = q3;
    }
    {
        let q4 = (r0.x * v[4].y) + (-r0.z);
        let q5 = (r0.y * v[4].w) + (-r0.w);
        r0.x = q4;
        r0.y = q5;
    }
    r0.z = v[4].z * v[4].y;
    r0.z = (v[4].x * v[4].w) + (-r0.z);
    r1.y = 1.0 / r0.z;
    r1.x = -r1.y;
    r2.x = r0.x * r1.x;
    r2.y = r0.y * r1.y;
    r2.z = ((-r0.x) * r1.x) + 1.0;
    r2.w = ((-r0.y) * r1.y) + 1.0;
    r0.x = bitcast<f32>(select(0u, 0xffffffffu, (r2.x < 0.0)));
    r0.y = bitcast<f32>(select(0u, 0xffffffffu, (r2.y < 0.0)));
    r0.z = bitcast<f32>(select(0u, 0xffffffffu, (r2.z < 0.0)));
    r0.w = bitcast<f32>(select(0u, 0xffffffffu, (r2.w < 0.0)));
    {
        let q6 = bitcast<f32>((bitcast<u32>(r0.z) | bitcast<u32>(r0.x)));
        let q7 = bitcast<f32>((bitcast<u32>(r0.w) | bitcast<u32>(r0.y)));
        r0.x = q6;
        r0.y = q7;
    }
    r0.x = bitcast<f32>((bitcast<u32>(r0.y) | bitcast<u32>(r0.x)));
    r0.x = v[6].z * v[6].x;
    r0.y = v[6].w * v[6].y;
    let q8 = tex_sample_k(1u, r0.x, r0.y);
    r1.x = q8.x;
    r1.y = q8.y;
    r1.z = q8.z;
    r1.w = q8.w;
    let q9 = tex_sample_k(0u, r0.x, r0.y);
    {
        let q10 = q9.x;
        let q11 = q9.y;
        let q12 = q9.z;
        let q13 = q9.w;
        r0.x = q10;
        r0.y = q11;
        r0.z = q12;
        r0.w = q13;
    }
    {
        let q14 = ((-r0.x) * 0.679542) + r1.x;
        let q15 = ((-r0.y) * 0.012983) + r1.y;
        let q16 = ((-r0.z) * 0.921582) + r1.z;
        r1.x = q14;
        r1.y = q15;
        r1.z = q16;
    }
    r2.x = r0.x * 0.679542;
    r2.y = r0.y * 0.012983;
    r2.z = r0.z * 0.921582;
    r0.w = saturate(r0.w);
    {
        let q17 = (r1.w * r1.x) + r2.x;
        let q18 = (r1.w * r1.y) + r2.y;
        let q19 = (r1.w * r1.z) + r2.z;
        r1.x = q17;
        r1.y = q18;
        r1.z = q19;
    }
    {
        let q20 = r1.x + P[17].x;
        let q21 = r1.y + P[17].y;
        let q22 = r1.z + P[17].z;
        r1.x = q20;
        r1.y = q21;
        r1.z = q22;
    }
    r0.x = max(r1.x, 0.0);
    r0.y = max(r1.y, 0.0);
    r0.z = max(r1.z, 0.0);
    {
        let q23 = r0.x * v[1].x;
        let q24 = r0.y * v[1].y;
        let q25 = r0.z * v[1].z;
        let q26 = r0.w * v[1].w;
        r0.x = q23;
        r0.y = q24;
        r0.z = q25;
        r0.w = q26;
    }
    r1.x = log2(r0.x);
    r1.y = log2(r0.y);
    r1.z = log2(r0.z);
    {
        let q27 = r1.x * P[10].x;
        let q28 = r1.y * P[10].x;
        let q29 = r1.z * P[10].x;
        r1.x = q27;
        r1.y = q28;
        r1.z = q29;
    }
    r2.x = r1.x * 0.41666666;
    r2.y = r1.y * 0.41666666;
    r2.z = r1.z * 0.41666666;
    {
        let q30 = exp2(r1.x);
        let q31 = exp2(r1.y);
        let q32 = exp2(r1.z);
        r1.x = q30;
        r1.y = q31;
        r1.z = q32;
    }
    {
        let q33 = exp2(r2.x);
        let q34 = exp2(r2.y);
        let q35 = exp2(r2.z);
        r2.x = q33;
        r2.y = q34;
        r2.z = q35;
    }
    {
        let q36 = (r2.x * 1.055) + (-0.055);
        let q37 = (r2.y * 1.055) + (-0.055);
        let q38 = (r2.z * 1.055) + (-0.055);
        r2.x = q36;
        r2.y = q37;
        r2.z = q38;
    }
    r3.x = r1.x * 12.92;
    r3.y = r1.y * 12.92;
    r3.z = r1.z * 12.92;
    {
        let q39 = bitcast<f32>(select(0u, 0xffffffffu, (r1.x >= 0.00313067)));
        let q40 = bitcast<f32>(select(0u, 0xffffffffu, (r1.y >= 0.00313067)));
        let q41 = bitcast<f32>(select(0u, 0xffffffffu, (r1.z >= 0.00313067)));
        r1.x = q39;
        r1.y = q40;
        r1.z = q41;
    }
    {
        let q42 = select(r3.x, r2.x, bitcast<u32>(r1.x) != 0u);
        let q43 = select(r3.y, r2.y, bitcast<u32>(r1.y) != 0u);
        let q44 = select(r3.z, r2.z, bitcast<u32>(r1.z) != 0u);
        r1.x = q42;
        r1.y = q43;
        r1.z = q44;
    }
    r1.w = bitcast<f32>(select(0u, 0xffffffffu, (P[10].y != 1.0)));
    o0.x = select(r0.x, r1.x, bitcast<u32>(r1.w) != 0u);
    o0.y = select(r0.y, r1.y, bitcast<u32>(r1.w) != 0u);
    o0.z = select(r0.z, r1.z, bitcast<u32>(r1.w) != 0u);
    o0.w = r0.w;
    return array<vec4<f32>, 1>(o0);
}
fn ps_main(v: array<vec4<f32>, 12>) -> vec4<f32> { return ps_raw(v)[0]; }
