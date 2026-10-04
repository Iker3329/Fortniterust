// Fragment stage shared by every translated Slate material pixel shader (ps_main is the generated translation).
// Feeds the shader what Slate's vertex stage feeds it: v1 tint (linear), v6 brush uv, v7.zw pixel size, v8 material uv.
@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let lin = in.lin;
    let size = P[4].zw;
    var ub = lin;
    if (P[3].y > 0.5) {
        let pos = lin * size;
        ub = vec2<f32>(nine_axis(pos.x, size.x, P[4].x, P[5].x, P[5].z), nine_axis(pos.y, size.y, P[4].y, P[5].y, P[5].w));
    }
    var v: array<vec4<f32>, 12>;
    v[1] = P[2];
    v[6] = vec4<f32>(ub, 1.0, 1.0);
    v[7] = vec4<f32>(0.0, 0.0, size);
    v[8] = vec4<f32>(lin, 0.0, 0.0);
    let c = ps_main(v);
    return finish(c);
}
