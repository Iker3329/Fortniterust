# Vendored opus-decoder 0.1.1

Copy of `opus-decoder` 0.1.1 from crates.io (MIT OR Apache-2.0, pure Rust, no unsafe), vendored for `fn-audio`
(`crates/fn-audio/Cargo.toml` points here). Tests, benches and the original manifest were dropped.

Why patched: Fortnite 1.8's streaming SoundWaves (`UE4OPUS`) decode through this crate, and two defects showed up
on real data.

1. `celt/vq.rs::extract_collapse_mask` built the PVQ collapse mask in a `u8` with `1 << i` for `i < b`. libopus lets
   `B` reach 16 for transient 20 ms frames (the TF change loop does `B <<= 1`), so a valid stream panicked with
   "attempt to shift left with overflow" in debug builds and silently produced a wrong mask in release builds.
   The mask is now a `u32` (like libopus' `unsigned`) end to end (`PvqDecodeInfo::collapse_mask`, the
   `bands.rs` call site); the later narrowing to the per-band `u8` storage is unchanged.

2. (not changed, noted) `packet.rs` rejects packets larger than 1500 bytes although libopus only limits each *frame*
   to 1275 bytes. `fn-audio` works around it by splitting oversized code-3 packets into their frames
   (`fn_audio::codec::opus::split_code3`), which is equivalent because decoder state carries across frames.

The crate also contains disabled debug scaffolding (`trace_this_packet` is a constant `false`); it never runs.
