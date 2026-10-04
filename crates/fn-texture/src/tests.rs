//! Unit tests with hand-computed blocks. Tests that need game data live in `tests/gamedata.rs`.

use crate::decode::bc1235::{alpha_block, bc1_block, build_normal_z};
use crate::decode::{bc6h, bc7};
use crate::{decode_mip, DecodeOptions, DecodedImage, PixelFormat, Pixels, Rounding};

/// LSB-first bit packer for building BPTC blocks.
struct W {
    v: u128,
    pos: u32,
}
impl W {
    fn new() -> W {
        W { v: 0, pos: 0 }
    }
    fn put(&mut self, val: u32, n: u32) -> &mut W {
        self.v |= ((val as u128) & ((1u128 << n) - 1)) << self.pos;
        self.pos += n;
        self
    }
    fn bytes(&self) -> [u8; 16] {
        self.v.to_le_bytes()
    }
}

fn opts() -> DecodeOptions {
    DecodeOptions::default()
}

#[test]
fn bc1_four_colour_block() {
    // c0 = pure red (565 0xF800), c1 = pure blue (0x001F), c0 > c1 -> 4 colour mode
    let mut b = vec![0x00, 0xF8, 0x1F, 0x00];
    b.extend_from_slice(&[0xE4; 4]); // every row: indices 0,1,2,3
    let mut out = [[0u8; 4]; 16];
    bc1_block(&b, Rounding::NvttTruncate, &mut out);
    assert_eq!(out[0], [255, 0, 0, 255]);
    assert_eq!(out[1], [0, 0, 255, 255]);
    assert_eq!(out[2], [170, 0, 85, 255]); // (2*255+0)/3, (2*0+255)/3 truncated
    assert_eq!(out[3], [85, 0, 170, 255]);
    assert_eq!(out[12..16], out[0..4]);
    bc1_block(&b, Rounding::Nearest, &mut out);
    assert_eq!(out[2], [170, 0, 85, 255]); // 510/3 and 255/3 are exact
}

#[test]
fn bc1_rounding_modes_differ() {
    // c0 = (r,g,b) = (2,4,2) in 565 -> expanded (16,16,16); c1 = 0; c0 > c1 -> 4 colour mode, index 2 = (2*c0+c1)/3
    let c0: u16 = (2 << 11) | (4 << 5) | 2;
    let mut b = c0.to_le_bytes().to_vec();
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&[0xAA; 4]); // all index 2
    let mut out = [[0u8; 4]; 16];
    bc1_block(&b, Rounding::NvttTruncate, &mut out);
    assert_eq!(out[0], [10, 10, 10, 255]); // 32/3 truncated
    bc1_block(&b, Rounding::Nearest, &mut out);
    assert_eq!(out[0], [11, 11, 11, 255]); // (32+1)/3
}

#[test]
fn bc1_three_colour_block() {
    // c0 = blue (0x001F) < c1 = red (0xF800): 3 colour mode, index 3 = transparent black
    let mut b = vec![0x1F, 0x00, 0x00, 0xF8];
    b.extend_from_slice(&[0xE4; 4]);
    let mut out = [[0u8; 4]; 16];
    bc1_block(&b, Rounding::NvttTruncate, &mut out);
    assert_eq!(out[0], [0, 0, 255, 255]);
    assert_eq!(out[1], [255, 0, 0, 255]);
    assert_eq!(out[2], [127, 0, 127, 255]); // (0+255)/2, (255+0)/2
    assert_eq!(out[3], [0, 0, 0, 0]);
}

#[test]
fn bc1_bit_expansion() {
    // 565 white -> 255; (31<<3)|(31>>2) = 255, (63<<2)|(63>>4) = 255; 565 value 1 in blue -> (1<<3)|0 = 8
    let c0: u16 = 0xFFFF;
    let mut b = c0.to_le_bytes().to_vec();
    b.extend_from_slice(&[1, 0]);
    b.extend_from_slice(&[0x00; 4]);
    let mut out = [[0u8; 4]; 16];
    bc1_block(&b, Rounding::NvttTruncate, &mut out);
    assert_eq!(out[0], [255, 255, 255, 255]);
    let mut b2 = vec![1, 0, 0, 0];
    b2.extend_from_slice(&[0x00; 4]);
    // c0 = 1 > c1 = 0 -> four colour mode, entry 0 = blue 8
    bc1_block(&b2, Rounding::NvttTruncate, &mut out);
    assert_eq!(out[0], [0, 0, 8, 255]);
}

fn pack3(idx: &[u8; 16]) -> [u8; 6] {
    let mut bits = 0u64;
    for (i, &v) in idx.iter().enumerate() {
        bits |= (v as u64) << (3 * i);
    }
    let b = bits.to_le_bytes();
    [b[0], b[1], b[2], b[3], b[4], b[5]]
}

#[test]
fn bc3_alpha_eight_value_mode() {
    let idx: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 2, 1, 0];
    let mut b = vec![255, 0];
    b.extend_from_slice(&pack3(&idx));
    let a = alpha_block(&b, Rounding::NvttTruncate);
    // palette: 255, 0, 6*255/7=218, 5*255/7=182, 4*255/7=145, 3*255/7=109, 2*255/7=72, 255/7=36
    assert_eq!(&a[..8], &[255, 0, 218, 182, 145, 109, 72, 36]);
    assert_eq!(&a[8..], &[36, 72, 109, 145, 182, 218, 0, 255]);
    let n = alpha_block(&b, Rounding::Nearest);
    assert_eq!(n[2], 219); // (1530+3)/7
}

#[test]
fn bc3_alpha_six_value_mode() {
    let idx: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut b = vec![0, 255]; // a0 <= a1 -> 6 value mode
    b.extend_from_slice(&pack3(&idx));
    let a = alpha_block(&b, Rounding::NvttTruncate);
    // palette: 0, 255, (4*0+255)/5=51, (3*0+2*255)/5=102, 153, 204, explicit 0, explicit 255
    assert_eq!(&a[..8], &[0, 255, 51, 102, 153, 204, 0, 255]);
}

#[test]
fn dxt5_full_image_decode() {
    // one block: colour = solid red via c0=c1=0xF800 (3 colour mode, index 0), alpha a0=a1=77 index 0
    let mut b = vec![77, 77, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
    let img = decode_mip(&PixelFormat::Dxt5, &b, 4, 4, &opts()).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(&p[..4], &[255, 0, 0, 77]);
    assert_eq!(p.len(), 64);
}

#[test]
fn dxt3_alpha_nibbles() {
    // alpha nibble pattern: pixel i has alpha nibble i
    let mut b = vec![0u8; 16];
    for i in 0..8 {
        b[i] = (2 * i as u8) | ((2 * i as u8 + 1) << 4);
    }
    b[8..12].copy_from_slice(&[0x00, 0xF8, 0x00, 0xF8]); // solid red
    let img = decode_mip(&PixelFormat::Dxt3, &b, 4, 4, &opts()).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    for i in 0..16 {
        assert_eq!(p[i * 4 + 3], (i as u8) * 17, "pixel {i}");
        assert_eq!(p[i * 4], 255);
    }
}

#[test]
fn dxt1_odd_size_is_clipped() {
    let mut blocks = Vec::new();
    for _ in 0..2 {
        blocks.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
    }
    let img = decode_mip(&PixelFormat::Dxt1, &blocks, 5, 3, &opts()).unwrap();
    assert_eq!((img.width, img.height), (5, 3));
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(p.len(), 5 * 3 * 4);
    assert!(p.chunks_exact(4).all(|px| px == [255, 0, 0, 255]));
}

#[test]
fn bc4_gray() {
    let mut b = vec![200, 100];
    b.extend_from_slice(&[0; 6]);
    let img = decode_mip(&PixelFormat::Bc4, &b, 4, 4, &opts()).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(&p[..4], &[200, 200, 200, 255]);
}

#[test]
fn bc5_normal_reconstruction() {
    // x = 255, y = 255: nx = ny = 1 -> radicand -1 -> nz = 0 -> z = int(255 * 1 / 2) = 127
    assert_eq!(build_normal_z(255, 255), 127);
    // x = 0, y = 0: nx = ny = -1 -> z = 127
    assert_eq!(build_normal_z(0, 0), 127);
    // x = y = 128: nx = ny = 2*128/255 - 1 = 0.003921569 -> nz = 0.9999846 -> int(255 * 1.9999846 / 2) = 254
    assert_eq!(build_normal_z(128, 128), 254);
    let mut b = vec![128, 128, 0, 0, 0, 0, 0, 0, 128, 128, 0, 0, 0, 0, 0, 0];
    let img = decode_mip(&PixelFormat::Bc5, &b, 4, 4, &opts()).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(&p[..4], &[128, 128, 254, 255]);
    let o = DecodeOptions { bc5_reconstruct_z: false, ..opts() };
    let img = decode_mip(&PixelFormat::Bc5, &b, 4, 4, &o).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(&p[..4], &[128, 128, 0, 255]);
    b.truncate(8);
    assert!(decode_mip(&PixelFormat::Bc5, &b, 4, 4, &opts()).is_err(), "short data must be rejected");
}

#[test]
fn bc7_mode6_endpoints_and_interpolation() {
    // mode 6: 6 zero bits + 1, r0 r1 g0 g1 b0 b1 (7 bits each), a0 a1 (7 bits), p0 p1, 4 bit indices (anchor 3 bits)
    let mut w = W::new();
    w.put(1 << 6, 7);
    for _ in 0..3 {
        w.put(127, 7).put(0, 7); // e0 = 127, e1 = 0 for r, g, b
    }
    w.put(127, 7).put(0, 7); // alpha
    w.put(1, 1).put(0, 1); // p-bits: e0 -> 255, e1 -> 0
    let mut idx = [0u32; 16];
    idx[8] = 7; // weight 30
    idx[15] = 15; // weight 64 -> e1
    for (i, &v) in idx.iter().enumerate() {
        w.put(v, if i == 0 { 3 } else { 4 });
    }
    assert_eq!(w.pos, 128);
    let mut out = [[0u8; 4]; 16];
    bc7::decode_block(&w.bytes(), &mut out);
    assert_eq!(out[0], [255, 255, 255, 255]);
    // ((64-30)*255 + 30*0 + 32) >> 6 = 8702 >> 6 = 135
    assert_eq!(out[8], [135, 135, 135, 135]);
    assert_eq!(out[15], [0, 0, 0, 0]);
}

#[test]
fn bc7_mode5_rotation() {
    // mode 5: 5 zero bits + 1, rotation 2 bits, r/g/b endpoints 7 bits, alpha 8 bits, 2+2 bit indices
    let mut w = W::new();
    w.put(1 << 5, 6).put(1, 2); // rotation 1: swap R and A
    w.put(127, 7).put(127, 7); // r0 r1 -> 255
    w.put(0, 7).put(0, 7); // g
    w.put(0, 7).put(0, 7); // b
    w.put(0, 8).put(0, 8); // a -> 0
    w.put(0, 31).put(0, 31); // colour then alpha indices (anchor pixel uses one bit less each)
    let mut out = [[0u8; 4]; 16];
    bc7::decode_block(&w.bytes(), &mut out);
    assert_eq!(out[0], [0, 0, 0, 255]); // (255,0,0,0) rotated
    assert_eq!(out[7], [0, 0, 0, 255]);
}

#[test]
fn bc7_invalid_block_is_transparent() {
    let mut out = [[9u8; 4]; 16];
    bc7::decode_block(&[0u8; 16], &mut out);
    assert!(out.iter().all(|p| *p == [0, 0, 0, 0]));
}

#[test]
fn bc6h_mode3_endpoints() {
    // detex mode 10 (original mode 3): m = 0b00011, r0 g0 b0 r1 g1 b1 = 10 bits each, 4 bit indices from bit 65
    let mut w = W::new();
    w.put(0b00011, 5);
    for _ in 0..3 {
        w.put(0x3ff, 10);
    }
    for _ in 0..3 {
        w.put(0, 10);
    }
    assert_eq!(w.pos, 65);
    let mut idx = [0u32; 16];
    idx[15] = 15;
    idx[9] = 8; // weight 34
    for (i, &v) in idx.iter().enumerate() {
        w.put(v, if i == 0 { 3 } else { 4 });
    }
    assert_eq!(w.pos, 128);
    let mut out = [[0u16; 4]; 16];
    bc6h::decode_block(&w.bytes(), &mut out);
    // e0 = 0xFFFF unquantised, interpolate(0xFFFF, 0, 0) = 65535, * 31 / 64 = 31743 = 0x7BFF = 65504.0 (max half)
    assert_eq!(out[0], [0x7bff, 0x7bff, 0x7bff, 0x3c00]);
    assert_eq!(out[15], [0, 0, 0, 0x3c00]);
    // weight4[8] = 34 -> ((64-34)*65535 + 32) >> 6, then * 31 / 64
    let w34 = (((64 - 34) * 65535 + 32) >> 6) * 31 / 64;
    assert_eq!(out[9][0] as i32, w34);
}

#[test]
fn bc6h_image_output_is_f16() {
    let mut w = W::new();
    w.put(0b00011, 5);
    for _ in 0..3 {
        w.put(0x3ff, 10);
    }
    for _ in 0..3 {
        w.put(0, 10);
    }
    w.put(0, 63);
    let img = decode_mip(&PixelFormat::Bc6h, &w.bytes(), 4, 4, &opts()).unwrap();
    assert!(matches!(img.pixels, Pixels::RgbaF16(_)));
    // 0x7bff clamps to 1.0 -> 255
    assert_eq!(&img.to_rgba8()[..4], &[255, 255, 255, 255]);
}

#[test]
fn raw_formats() {
    let img = decode_mip(&PixelFormat::B8G8R8A8, &[1, 2, 3, 4, 5, 6, 7, 8], 2, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::Rgba8(vec![3, 2, 1, 4, 7, 6, 5, 8]));
    let img = decode_mip(&PixelFormat::A8R8G8B8, &[1, 2, 3, 4], 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::Rgba8(vec![3, 2, 1, 4]));
    let img = decode_mip(&PixelFormat::R8G8B8A8, &[1, 2, 3, 4], 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::Rgba8(vec![1, 2, 3, 4]));
    let img = decode_mip(&PixelFormat::G8, &[9, 200], 2, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::Rgba8(vec![9, 9, 9, 255, 200, 200, 200, 255]));
    let img = decode_mip(&PixelFormat::G16, &[0x34, 0x12], 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::Rgba8(vec![0x12, 0x12, 0x12, 255]));
    // RGBA16F: 1.0, 0.5, 0.0, 1.0
    let src = [0x00, 0x3c, 0x00, 0x38, 0x00, 0x00, 0x00, 0x3c];
    let img = decode_mip(&PixelFormat::FloatRGBA, &src, 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::RgbaF16(vec![0x3c00, 0x3800, 0, 0x3c00]));
    assert_eq!(img.to_rgba8(), vec![255, 128, 0, 255]);
    let img = decode_mip(&PixelFormat::R32F, &1.0f32.to_le_bytes(), 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::RgbaF16(vec![0x3c00, 0, 0, 0x3c00]));
    let img = decode_mip(&PixelFormat::R16F, &[0x00, 0x38], 1, 1, &opts()).unwrap();
    assert_eq!(img.pixels, Pixels::RgbaF16(vec![0x3800, 0, 0, 0x3c00]));
    // V8U8: (0,0) -> (128,128), t = 1 -> ... quirky z
    let img = decode_mip(&PixelFormat::V8U8, &[0, 0], 1, 1, &opts()).unwrap();
    let Pixels::Rgba8(p) = img.pixels else { panic!() };
    assert_eq!(&p[..2], &[128, 128]);
    assert!(decode_mip(&PixelFormat::Other("PF_ASTC_4x4".into()), &[0; 16], 4, 4, &opts()).is_err());
}

#[test]
fn format_sizes() {
    assert_eq!(PixelFormat::Dxt1.mip_bytes(2048, 2048), Some(2048 * 2048 / 2));
    assert_eq!(PixelFormat::Dxt5.mip_bytes(1, 1), Some(16));
    assert_eq!(PixelFormat::Dxt5.mip_bytes(5, 3), Some(2 * 16));
    assert_eq!(PixelFormat::B8G8R8A8.mip_bytes(3, 2), Some(24));
    assert_eq!(PixelFormat::from_name("PF_BC7"), PixelFormat::Bc7);
    assert_eq!(PixelFormat::from_name("PF_Foo"), PixelFormat::Other("PF_Foo".into()));
}

#[test]
fn threaded_equals_single_threaded() {
    // pseudo random DXT5 data, 256x192 -> multi thread path
    let (w, h) = (256usize, 192usize);
    let mut data = vec![0u8; w / 4 * h / 4 * 16];
    let mut s = 0x1234_5678u32;
    for b in data.iter_mut() {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        *b = (s >> 24) as u8;
    }
    let a = decode_mip(&PixelFormat::Dxt5, &data, w, h, &DecodeOptions { threads: 1, ..opts() }).unwrap();
    let b = decode_mip(&PixelFormat::Dxt5, &data, w, h, &DecodeOptions { threads: 4, ..opts() }).unwrap();
    assert_eq!(a, b);
}

#[test]
fn png_roundtrip() {
    let img = DecodedImage { width: 2, height: 1, pixels: Pixels::Rgba8(vec![1, 2, 3, 255, 4, 5, 6, 128]) };
    let dir = std::env::temp_dir().join("fn_texture_png_test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a.png");
    img.write_png(&p).unwrap();
    let back = image::open(&p).unwrap().to_rgba8();
    assert_eq!(back.as_raw(), &vec![1, 2, 3, 255, 4, 5, 6, 128]);
    let opaque = DecodedImage { width: 1, height: 1, pixels: Pixels::Rgba8(vec![9, 8, 7, 255]) };
    opaque.write_png(&p).unwrap();
    assert!(matches!(image::open(&p).unwrap(), image::DynamicImage::ImageRgb8(_)));
    let hdr = DecodedImage { width: 1, height: 1, pixels: Pixels::RgbaF16(vec![0x3c00, 0x3800, 0, 0x3c00]) };
    hdr.write_png(&p).unwrap();
    let back = image::open(&p).unwrap().to_rgba16();
    assert_eq!(back.as_raw(), &vec![65535, 32768, 0, 65535]);
}
