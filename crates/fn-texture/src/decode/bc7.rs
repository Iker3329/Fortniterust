//! BC7 (BPTC) decoder. Follows the same algorithm and rounding as detex's `decompress-bptc.cpp`
//! (endpoint bit replication, 6-bit weight interpolation with +32 rounding), written as a plain sequential bit
//! reader instead of detex's hand optimised field extraction.

use super::decode_blocks;
use super::tables::{ANCHOR2, ANCHOR3_2, ANCHOR3_3, P2, P3, WEIGHT2, WEIGHT3, WEIGHT4};

struct Bits {
    v: u128,
    pos: u32,
}

impl Bits {
    fn new(b: &[u8]) -> Bits {
        Bits { v: u128::from_le_bytes(b[..16].try_into().unwrap()), pos: 0 }
    }
    fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let r = ((self.v >> self.pos) & ((1u128 << n) - 1)) as u32;
        self.pos += n;
        r
    }
}

// per mode: subsets, partition bits, rotation bits, index-selection bits, colour bits, alpha bits,
// per-endpoint p-bit, per-subset shared p-bit, index bits, secondary index bits
struct Mode {
    ns: usize,
    pb: u32,
    rb: u32,
    isb: u32,
    cb: u32,
    ab: u32,
    epb: bool,
    spb: bool,
    ib: u32,
    ib2: u32,
}

const MODES: [Mode; 8] = [
    Mode { ns: 3, pb: 4, rb: 0, isb: 0, cb: 4, ab: 0, epb: true, spb: false, ib: 3, ib2: 0 },
    Mode { ns: 2, pb: 6, rb: 0, isb: 0, cb: 6, ab: 0, epb: false, spb: true, ib: 3, ib2: 0 },
    Mode { ns: 3, pb: 6, rb: 0, isb: 0, cb: 5, ab: 0, epb: false, spb: false, ib: 2, ib2: 0 },
    Mode { ns: 2, pb: 6, rb: 0, isb: 0, cb: 7, ab: 0, epb: true, spb: false, ib: 2, ib2: 0 },
    Mode { ns: 1, pb: 0, rb: 2, isb: 1, cb: 5, ab: 6, epb: false, spb: false, ib: 2, ib2: 3 },
    Mode { ns: 1, pb: 0, rb: 2, isb: 0, cb: 7, ab: 8, epb: false, spb: false, ib: 2, ib2: 2 },
    Mode { ns: 1, pb: 0, rb: 0, isb: 0, cb: 7, ab: 7, epb: true, spb: false, ib: 4, ib2: 0 },
    Mode { ns: 2, pb: 6, rb: 0, isb: 0, cb: 5, ab: 5, epb: true, spb: false, ib: 2, ib2: 0 },
];

fn weight(bits: u32, idx: u32) -> u32 {
    (match bits {
        2 => WEIGHT2[idx as usize],
        3 => WEIGHT3[idx as usize],
        _ => WEIGHT4[idx as usize],
    }) as u32
}

fn lerp(e0: u32, e1: u32, idx: u32, bits: u32) -> u8 {
    let w = weight(bits, idx);
    (((64 - w) * e0 + w * e1 + 32) >> 6) as u8
}

fn anchor(ns: usize, partition: usize, subset: usize) -> usize {
    if subset == 0 {
        return 0;
    }
    if ns == 2 {
        return ANCHOR2[partition] as usize;
    }
    if subset == 1 {
        ANCHOR3_2[partition] as usize
    } else {
        ANCHOR3_3[partition] as usize
    }
}

/// Decode one BC7 block into 16 RGBA pixels. Invalid blocks (mode bits all zero) decode to transparent black.
pub fn decode_block(b: &[u8], out: &mut [[u8; 4]; 16]) {
    let mut bits = Bits::new(b);
    let mode_idx = (b[0].trailing_zeros()) as usize;
    if mode_idx >= 8 {
        *out = [[0; 4]; 16];
        return;
    }
    bits.pos = mode_idx as u32 + 1;
    let m = &MODES[mode_idx];
    let partition = bits.read(m.pb) as usize;
    let rotation = bits.read(m.rb);
    let isel = bits.read(m.isb);

    // endpoints [subset*2 + end][comp]
    let mut ep = [[0u32; 4]; 6];
    for c in 0..3 {
        for e in ep.iter_mut().take(m.ns * 2) {
            e[c] = bits.read(m.cb);
        }
    }
    if m.ab > 0 {
        for e in ep.iter_mut().take(m.ns * 2) {
            e[3] = bits.read(m.ab);
        }
    }
    let (cprec, aprec);
    if m.epb {
        for e in ep.iter_mut().take(m.ns * 2) {
            let p = bits.read(1);
            for v in e.iter_mut() {
                *v = (*v << 1) | p;
            }
        }
        cprec = m.cb + 1;
        aprec = if m.ab > 0 { m.ab + 1 } else { 0 };
    } else if m.spb {
        for s in 0..m.ns {
            let p = bits.read(1);
            for e in 0..2 {
                for c in 0..3 {
                    ep[s * 2 + e][c] = (ep[s * 2 + e][c] << 1) | p;
                }
            }
        }
        cprec = m.cb + 1;
        aprec = 0;
    } else {
        cprec = m.cb;
        aprec = m.ab;
    }
    for e in ep.iter_mut().take(m.ns * 2) {
        for c in 0..3 {
            e[c] = (e[c] << (8 - cprec)) | (e[c] >> (2 * cprec - 8));
        }
        e[3] = if aprec > 0 { (e[3] << (8 - aprec)) | (e[3] >> (2 * aprec - 8)) } else { 255 };
    }

    let subset_of = |i: usize| -> usize {
        match m.ns {
            1 => 0,
            2 => P2[partition * 16 + i] as usize,
            _ => P3[partition * 16 + i] as usize,
        }
    };
    let mut idx1 = [0u32; 16];
    let mut idx2 = [0u32; 16];
    for i in 0..16 {
        let s = subset_of(i);
        let n = if i == anchor(m.ns, partition, s) { m.ib - 1 } else { m.ib };
        idx1[i] = bits.read(n);
    }
    if m.ib2 > 0 {
        for i in 0..16 {
            let s = subset_of(i);
            let n = if i == anchor(m.ns, partition, s) { m.ib2 - 1 } else { m.ib2 };
            idx2[i] = bits.read(n);
        }
    }

    for i in 0..16 {
        let s = subset_of(i);
        let (e0, e1) = (&ep[s * 2], &ep[s * 2 + 1]);
        // colour / alpha index selection (only mode 4 has two index sets of different widths, mode 5 has two of 2 bits)
        let (ci, cbits, ai, abits) = if m.ib2 == 0 {
            (idx1[i], m.ib, idx1[i], m.ib)
        } else if isel == 0 {
            (idx1[i], m.ib, idx2[i], m.ib2)
        } else {
            (idx2[i], m.ib2, idx1[i], m.ib)
        };
        let mut px = [lerp(e0[0], e1[0], ci, cbits), lerp(e0[1], e1[1], ci, cbits), lerp(e0[2], e1[2], ci, cbits), lerp(e0[3], e1[3], ai, abits)];
        match rotation {
            1 => px.swap(0, 3),
            2 => px.swap(1, 3),
            3 => px.swap(2, 3),
            _ => {}
        }
        out[i] = px;
    }
}

/// Decode a whole BC7 image to RGBA8.
pub fn decode(src: &[u8], w: usize, h: usize, threads: usize) -> Vec<u8> {
    decode_blocks(src, w, h, 16, threads, |b, out| decode_block(b, out))
}
