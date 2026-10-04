//! Frame-time statistics (scripted runs write them next to the captures).
#[derive(Default, Clone)]
pub struct FrameStats { pub ms: Vec<f32> }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary { pub frames: usize, pub mean: f32, pub median: f32, pub p95: f32, pub p99: f32, pub max: f32, pub min: f32 }

impl FrameStats {
    pub fn push(&mut self, ms: f32) { self.ms.push(ms); }
    /// Percentile by the nearest-rank method over the sorted samples (`q` in 0..=1).
    pub fn percentile(sorted: &[f32], q: f32) -> f32 {
        if sorted.is_empty() { return 0.; }
        let rank = ((q.clamp(0., 1.) * sorted.len() as f32).ceil() as usize).clamp(1, sorted.len());
        sorted[rank - 1]
    }
    /// Skips the first `skip` frames (pipeline warm-up, first exposure snap).
    pub fn summary(&self, skip: usize) -> Option<Summary> {
        let mut v: Vec<f32> = self.ms.iter().copied().skip(skip).collect();
        if v.is_empty() { return None; }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(Summary { frames: v.len(), mean: v.iter().sum::<f32>() / v.len() as f32, median: Self::percentile(&v, 0.5), p95: Self::percentile(&v, 0.95), p99: Self::percentile(&v, 0.99), max: *v.last().unwrap(), min: v[0] })
    }
    pub fn report(&self, label: &str, skip: usize) -> String {
        match self.summary(skip) {
            Some(s) => format!("{label}: {} frames (first {skip} skipped) mean {:.2} ms ({:.0} fps) median {:.2} p95 {:.2} p99 {:.2} max {:.2} min {:.2}\n", s.frames, s.mean, 1000. / s.mean.max(1e-6), s.median, s.p95, s.p99, s.max, s.min),
            None => format!("{label}: no frames\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn percentiles_use_nearest_rank() {
        let mut s = FrameStats::default(); for i in 1..=100 { s.push(i as f32); }
        let m = s.summary(0).unwrap();
        assert_eq!((m.median, m.p95, m.p99, m.max, m.min), (50., 95., 99., 100., 1.)); assert!((m.mean - 50.5).abs() < 1e-4);
    }
    #[test] fn warm_up_frames_are_skipped_and_empty_is_none() {
        let mut s = FrameStats::default(); s.push(500.); s.push(5.); s.push(5.);
        assert_eq!(s.summary(1).unwrap().max, 5.); assert!(s.summary(3).is_none()); assert!(FrameStats::default().summary(0).is_none());
    }
    #[test] fn single_sample() { let s = FrameStats { ms: vec![7.] }; let m = s.summary(0).unwrap(); assert_eq!((m.p99, m.median), (7., 7.)); }
}
