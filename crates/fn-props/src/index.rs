//! PERF (2026-10-08): a uniform xz grid over the props, so the per-tick collision queries (player blockers / hulls, the camera's props, the floor
//! below a point) stop scanning every prop of the map (~38k POI actors) several times per simulation step.
//!
//! EXACTNESS: the grid only picks CANDIDATES. Every query then applies the very same test as the linear scan it replaces, to the candidates in
//! ascending prop index (the linear scan's order), so the result is the same list in the same order. The grid is conservative: a hull prop is
//! filed in every cell its bounding circle's xz box touches, and a query visits every cell its range box (plus [`MARGIN_M`]) touches; any prop the
//! exact test can accept (horizontal distance - radius <= range: a 3D distance is never below the horizontal one) is therefore a candidate.
//! Props without a hull (the placeholder trees / rocks) are not in the grid: [`PropIndex::plain`] lists them for the queries that take them all.
//!
//! STALENESS: the index belongs to one `World::props` list. Destroyed / regrown props need no rebuild (every query still checks `alive()`), only a
//! changed list does (load, `--props`, scripted resets, tests). [`PropIndex::matches`] compares the length, the buffer address and a sample of up to
//! [`SAMPLES`] props (position bits + hull address); the owner rebuilds when it fails ([`PropIndex::build`]).
//!
//! OFF SWITCH: `FN_PROP_INDEX=0` keeps the old linear scans ([`enabled`]). `FN_PROP_INDEX_VERIFY=1` runs both and logs any difference ([`verify`]).
use glam::Vec2;
use fn_combat::Harvestable;

/// Grid cell size (m). CHOSEN: 16 m (a player-hull query, 6 m, touches 1-4 cells; the camera's 40 m touches ~36).
pub const CELL_M: f32 = 16.;
/// Extra reach (m) added to every query box so float rounding at a cell border can never drop a candidate (the exact test decides).
pub const MARGIN_M: f32 = 1.;
/// A prop whose box spans more cells than this goes to the always-tested list instead (a landscape-sized piece would fill the grid).
pub const MAX_CELLS_PER_PROP: i64 = 256;
/// Props sampled by [`PropIndex::matches`].
pub const SAMPLES: usize = 64;

/// `FN_PROP_INDEX=0`: the queries use the old linear scans.
pub fn enabled() -> bool { static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *ON.get_or_init(|| fn_live::var("FN_PROP_INDEX").map_or(true, |v| v != "0")) }
/// `FN_PROP_INDEX_VERIFY=1` (debug): every indexed query is compared with its linear scan, differences are logged.
pub fn verify() -> bool { static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *ON.get_or_init(|| fn_live::var("FN_PROP_INDEX_VERIFY").map_or(false, |v| v == "1")) }

#[derive(Clone, Debug, Default, PartialEq)]
struct Fingerprint { len: usize, addr: usize, sample: u64 }
fn fingerprint(props: &[Harvestable]) -> Fingerprint {
    let n = props.len();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| { h ^= v; h = h.wrapping_mul(0x0100_0000_01b3); };
    if n > 0 {
        let step = n.div_ceil(SAMPLES).max(1);
        let mut i = 0;
        loop {
            let p = &props[i];
            mix(p.pos.x.to_bits() as u64 | (p.pos.y.to_bits() as u64) << 32); mix(p.pos.z.to_bits() as u64);
            mix(p.hull.as_ref().map_or(0, |a| std::sync::Arc::as_ptr(a) as usize as u64));
            if i == n - 1 { break; }
            i = (i + step).min(n - 1);
        }
    }
    Fingerprint { len: n, addr: props.as_ptr() as usize, sample: h }
}

/// The grid (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct PropIndex {
    key: Fingerprint,
    origin: Vec2,
    nx: i32,
    nz: i32,
    /// CSR: the props of cell `c` are `items[starts[c]..starts[c + 1]]` (ascending).
    starts: Vec<u32>,
    items: Vec<u32>,
    /// Hull props too large for the grid: candidates of every query.
    big: Vec<u32>,
    /// Props without a hull (placeholders), ascending.
    plain: Vec<u32>,
}

impl PropIndex {
    pub fn build(props: &[Harvestable]) -> PropIndex {
        let mut plain = vec![];
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        let mut boxes: Vec<(u32, Vec2, Vec2)> = Vec::with_capacity(props.len());
        for (i, p) in props.iter().enumerate() {
            match &p.hull {
                None => plain.push(i as u32),
                Some(h) => {
                    let c = Vec2::new(h.center.x, h.center.z);
                    let r = if h.radius.is_finite() { h.radius.max(0.) } else { f32::MAX };
                    let (a, b) = (c - Vec2::splat(r), c + Vec2::splat(r));
                    if !(a.is_finite() && b.is_finite()) { boxes.push((i as u32, Vec2::NAN, Vec2::NAN)); continue; } // goes to `big`
                    lo = lo.min(a); hi = hi.max(b);
                    boxes.push((i as u32, a, b));
                }
            }
        }
        let mut idx = PropIndex { key: fingerprint(props), plain, ..Default::default() };
        if lo.x > hi.x { idx.big = boxes.iter().map(|b| b.0).collect(); idx.starts = vec![0]; return idx; }
        idx.origin = lo;
        idx.nx = (((hi.x - lo.x) / CELL_M).floor() as i32 + 1).max(1);
        idx.nz = (((hi.y - lo.y) / CELL_M).floor() as i32 + 1).max(1);
        let ncell = idx.nx as usize * idx.nz as usize;
        let mut counts = vec![0u32; ncell + 1];
        let mut placed: Vec<(u32, [i32; 4])> = Vec::with_capacity(boxes.len());
        for &(i, a, b) in &boxes {
            if !a.is_finite() { idx.big.push(i); continue; }
            let r = idx.cells(a, b);
            if (r[2] - r[0] + 1) as i64 * (r[3] - r[1] + 1) as i64 > MAX_CELLS_PER_PROP { idx.big.push(i); continue; }
            for z in r[1]..=r[3] { for x in r[0]..=r[2] { counts[(z * idx.nx + x) as usize] += 1; } }
            placed.push((i, r));
        }
        let mut starts = Vec::with_capacity(ncell + 1);
        let mut total = 0u32;
        for c in &counts[..ncell] { starts.push(total); total += c; }
        starts.push(total);
        let mut fill = starts.clone();
        let mut items = vec![0u32; total as usize];
        // props in ascending index: every cell list comes out ascending
        for &(i, r) in &placed { for z in r[1]..=r[3] { for x in r[0]..=r[2] { let c = (z * idx.nx + x) as usize; items[fill[c] as usize] = i; fill[c] += 1; } } }
        idx.starts = starts; idx.items = items;
        idx
    }
    /// The index still describes `props` (see the module docs).
    pub fn matches(&self, props: &[Harvestable]) -> bool { self.key == fingerprint(props) }
    /// Props without a hull, ascending.
    pub fn plain(&self) -> &[u32] { &self.plain }
    /// Cell range (x0, z0, x1, z1), clamped to the grid, of the xz box `a..b`.
    fn cells(&self, a: Vec2, b: Vec2) -> [i32; 4] {
        let f = |v: f32, n: i32| (((v / CELL_M).floor()).clamp(-1., n as f32)) as i32;
        let (x0, z0) = (f(a.x - self.origin.x, self.nx), f(a.y - self.origin.y, self.nz));
        let (x1, z1) = (f(b.x - self.origin.x, self.nx), f(b.y - self.origin.y, self.nz));
        [x0.max(0), z0.max(0), x1.min(self.nx - 1), z1.min(self.nz - 1)]
    }
    /// Hull props whose bounding circle may come within `range` (m, horizontal) of `around` (world x / z): ascending, no duplicates, into `out`
    /// (cleared first). A superset of the props the exact tests accept.
    pub fn hull_candidates(&self, around: Vec2, range: f32, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.big);
        if self.nx > 0 && self.nz > 0 {
            let m = range.max(0.) + MARGIN_M;
            let (a, b) = (around - Vec2::splat(m), around + Vec2::splat(m));
            let ox = (a.x - self.origin.x) / CELL_M; let oz = (a.y - self.origin.y) / CELL_M;
            let ex = (b.x - self.origin.x) / CELL_M; let ez = (b.y - self.origin.y) / CELL_M;
            // wholly outside the grid (or NaN): no grid prop can be near
            if ex >= 0. && ez >= 0. && ox < self.nx as f32 && oz < self.nz as f32 {
                let r = self.cells(a, b);
                for z in r[1]..=r[3] { for x in r[0]..=r[2] {
                    let c = (z * self.nx + x) as usize;
                    out.extend_from_slice(&self.items[self.starts[c] as usize..self.starts[c + 1] as usize]);
                } }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
    pub fn segment_candidates(&self, a: Vec2, b: Vec2, range: f32, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.big);
        if self.nx > 0 && self.nz > 0 && a.is_finite() && b.is_finite() {
            let m = range.max(0.) + MARGIN_M;
            let (lo, hi) = (a.min(b) - Vec2::splat(m), a.max(b) + Vec2::splat(m));
            let ex = (hi.x - self.origin.x) / CELL_M; let ez = (hi.y - self.origin.y) / CELL_M;
            let ox = (lo.x - self.origin.x) / CELL_M; let oz = (lo.y - self.origin.y) / CELL_M;
            if ex >= 0. && ez >= 0. && ox < self.nx as f32 && oz < self.nz as f32 {
                let r = self.cells(lo, hi);
                let reach = m + CELL_M * std::f32::consts::FRAC_1_SQRT_2;
                let ab = b - a; let l2 = ab.length_squared();
                for z in r[1]..=r[3] { for x in r[0]..=r[2] {
                    let c = self.origin + Vec2::new((x as f32 + 0.5) * CELL_M, (z as f32 + 0.5) * CELL_M);
                    let t = if l2 > 0. { ((c - a).dot(ab) / l2).clamp(0., 1.) } else { 0. };
                    if (a + ab * t - c).length() > reach { continue; }
                    let cell = (z * self.nx + x) as usize;
                    out.extend_from_slice(&self.items[self.starts[cell] as usize..self.starts[cell + 1] as usize]);
                } }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}
