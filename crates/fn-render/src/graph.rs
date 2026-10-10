//! A deliberately small frame graph: an ordered list of named passes recorded into one command encoder.
//! Resource lifetimes are owned by the renderer (fixed targets per window size), so the graph only provides ordering,
//! enable/disable per pass and CPU encode timing, which is all this renderer needs. Dependencies between passes are
//! tracked by wgpu itself (it inserts the barriers between render/compute passes).
use std::time::Instant;

/// CPU time spent recording one pass.
#[derive(Clone, Debug)]
pub struct PassTiming {
    pub name: String,
    pub encode_ms: f32,
}

type PassFn<'a> = Box<dyn FnOnce(&mut wgpu::CommandEncoder) + 'a>;

#[derive(Default)]
pub struct FrameGraph<'a> {
    passes: Vec<(String, PassFn<'a>)>,
}
impl<'a> FrameGraph<'a> {
    pub fn new() -> Self { FrameGraph { passes: Vec::new() } }
    /// Appends a pass; passes execute in the order they were added.
    pub fn add(&mut self, name: impl Into<String>, f: impl FnOnce(&mut wgpu::CommandEncoder) + 'a) -> &mut Self {
        self.passes.push((name.into(), Box::new(f)));
        self
    }
    /// Appends a pass only when `enabled`.
    pub fn add_if(&mut self, enabled: bool, name: impl Into<String>, f: impl FnOnce(&mut wgpu::CommandEncoder) + 'a) -> &mut Self {
        if enabled { self.add(name, f); }
        self
    }
    pub fn names(&self) -> Vec<&str> { self.passes.iter().map(|(n, _)| n.as_str()).collect() }
    pub fn len(&self) -> usize { self.passes.len() }
    pub fn is_empty(&self) -> bool { self.passes.is_empty() }
    /// Like [`execute`](Self::execute), calling `around(encoder, name, begin)` before (`true`) and after (`false`) each pass (PERF: per-node timestamps).
    pub fn execute_with(self, encoder: &mut wgpu::CommandEncoder, mut around: impl FnMut(&mut wgpu::CommandEncoder, &str, bool)) -> Vec<PassTiming> {
        self.passes.into_iter().map(|(name, f)| {
            let t = Instant::now();
            around(encoder, &name, true);
            f(encoder);
            around(encoder, &name, false);
            PassTiming { name, encode_ms: t.elapsed().as_secs_f32() * 1000. }
        }).collect()
    }
    /// Records every pass into `encoder`, returning the per-pass encode times.
    pub fn execute(self, encoder: &mut wgpu::CommandEncoder) -> Vec<PassTiming> {
        self.passes.into_iter().map(|(name, f)| {
            let t = Instant::now();
            f(encoder);
            PassTiming { name, encode_ms: t.elapsed().as_secs_f32() * 1000. }
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passes_keep_insertion_order_and_respect_enable_flags() {
        let mut g = FrameGraph::new();
        g.add("a", |_| {}).add_if(false, "skipped", |_| {}).add_if(true, "b", |_| {}).add("c", |_| {});
        assert_eq!(g.names(), vec!["a", "b", "c"]);
        assert_eq!(g.len(), 3);
    }
}
