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
    r0.x = P[11].y * 1.4426951;
    r0.x = exp2(r0.x);
    r0.x = (r0.x * 0.5) + 0.75;
    {
        let q8 = r0.x * 0.679542;
        let q9 = r0.x * 0.012983;
        let q10 = r0.x * 0.921582;
        r0.x = q8;
        r0.y = q9;
        r0.z = q10;
    }
    r1.x = v[6].z * v[6].x;
    r1.y = v[6].w * v[6].y;
    let q11 = tex_sample_k(1u, r1.x, r1.y);
    r2.x = q11.x;
    r2.y = q11.y;
    r2.z = q11.z;
    r2.w = q11.w;
    let q12 = tex_sample_k(0u, r1.x, r1.y);
    {
        let q13 = q12.x;
        let q14 = q12.y;
        let q15 = q12.z;
        let q16 = q12.w;
        r1.x = q13;
        r1.y = q14;
        r1.z = q15;
        r1.w = q16;
    }
    {
        let q17 = ((-r0.x) * r1.x) + r2.x;
        let q18 = ((-r0.y) * r1.y) + r2.y;
        let q19 = ((-r0.z) * r1.z) + r2.z;
        r2.x = q17;
        r2.y = q18;
        r2.z = q19;
    }
    {
        let q20 = r0.x * r1.x;
        let q21 = r0.y * r1.y;
        let q22 = r0.z * r1.z;
        r0.x = q20;
        r0.y = q21;
        r0.z = q22;
    }
    r1.w = saturate(r1.w);
    {
        let q23 = (r2.w * r2.x) + r0.x;
        let q24 = (r2.w * r2.y) + r0.y;
        let q25 = (r2.w * r2.z) + r0.z;
        r0.x = q23;
        r0.y = q24;
        r0.z = q25;
    }
    {
        let q26 = r0.x + P[17].x;
        let q27 = r0.y + P[17].y;
        let q28 = r0.z + P[17].z;
        r0.x = q26;
        r0.y = q27;
        r0.z = q28;
    }
    r1.x = max(r0.x, 0.0);
    r1.y = max(r0.y, 0.0);
    r1.z = max(r0.z, 0.0);
    r0.x = r1.x * v[1].x;
    r0.y = r1.y * v[1].y;
    r0.z = r1.z * v[1].z;
    r0.w = r1.w * v[1].w;
    r1.x = log2(r0.x);
    r1.y = log2(r0.y);
    r1.z = log2(r0.z);
    {
        let q29 = r1.x * P[10].x;
        let q30 = r1.y * P[10].x;
        let q31 = r1.z * P[10].x;
        r1.x = q29;
        r1.y = q30;
        r1.z = q31;
    }
    r2.x = r1.x * 0.41666666;
    r2.y = r1.y * 0.41666666;
    r2.z = r1.z * 0.41666666;
    {
        let q32 = exp2(r1.x);
        let q33 = exp2(r1.y);
        let q34 = exp2(r1.z);
        r1.x = q32;
        r1.y = q33;
        r1.z = q34;
    }
    {
        let q35 = exp2(r2.x);
        let q36 = exp2(r2.y);
        let q37 = exp2(r2.z);
        r2.x = q35;
        r2.y = q36;
        r2.z = q37;
    }
    {
        let q38 = (r2.x * 1.055) + (-0.055);
        let q39 = (r2.y * 1.055) + (-0.055);
        let q40 = (r2.z * 1.055) + (-0.055);
        r2.x = q38;
        r2.y = q39;
        r2.z = q40;
    }
    r3.x = r1.x * 12.92;
    r3.y = r1.y * 12.92;
    r3.z = r1.z * 12.92;
    {
        let q41 = bitcast<f32>(select(0u, 0xffffffffu, (r1.x >= 0.00313067)));
        let q42 = bitcast<f32>(select(0u, 0xffffffffu, (r1.y >= 0.00313067)));
        let q43 = bitcast<f32>(select(0u, 0xffffffffu, (r1.z >= 0.00313067)));
        r1.x = q41;
        r1.y = q42;
        r1.z = q43;
    }
    {
        let q44 = select(r3.x, r2.x, bitcast<u32>(r1.x) != 0u);
        let q45 = select(r3.y, r2.y, bitcast<u32>(r1.y) != 0u);
        let q46 = select(r3.z, r2.z, bitcast<u32>(r1.z) != 0u);
        r1.x = q44;
        r1.y = q45;
        r1.z = q46;
    }
    r1.w = bitcast<f32>(select(0u, 0xffffffffu, (P[10].y != 1.0)));
    o0.x = select(r0.x, r1.x, bitcast<u32>(r1.w) != 0u);
    o0.y = select(r0.y, r1.y, bitcast<u32>(r1.w) != 0u);
    o0.z = select(r0.z, r1.z, bitcast<u32>(r1.w) != 0u);
    o0.w = r0.w;
    return array<vec4<f32>, 1>(o0);
}
fn ps_main(v: array<vec4<f32>, 12>) -> vec4<f32> { return ps_raw(v)[0]; }
