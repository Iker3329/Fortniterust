# fn-audio

Pure-Rust audio for the Fortnite 1.8 (UE 4.16, CL 3724489) rewrite: SoundWave parsing and decoding straight from the
paks, SoundCue graphs, SoundClass / SoundMix assets, recovered attenuation settings, a deterministic offline mixer, and
an optional cpal (WASAPI) output behind a lock-free command queue.

| crate | role |
|---|---|
| `fn-audio` | this crate: `soundwave`, `codec` (OGG / OPUS / PCM / ADPCM), `cue`, `soundclass`, `attenuation`, `curve`, `mixer`, `library`, `events`, `sweep`, `analysis`, `device` (feature `device-output`) |
| `fn-audio-cli` | `list`, `info`, `decode`, `sweep`, `cue`, `cues`, `classes`, `events`, `render`, `analyze`, `device-smoke` |
| `fn-audio-sink` | `AudioSink` / `SoundEvent` (copy of fn-app's seam) implemented on this crate; see its crate docs for the one-line fn-app switch |
| `fn-audio-pack` | NEW: `build` / `list` / `verify` a `*.fnaudiopack` (a stored UE pak v3 with only the packages the game's cues need) so a game ships sound without the paks; opened with `fn_audio_pack::open_pack` (used by the old standalone runtime's `assets/audio`) |

Dependencies: `fn-pak`, `fn-uasset` (package + tagged properties), `lewton` (Vorbis), a **vendored, patched**
`opus-decoder` 0.1.1 (`vendor/opus-decoder`, see its `PATCHES.md`), `crossbeam-queue`, `rayon`, `cpal` (optional).
`fn-ueio` is a dev-dependency only (a test pins the bulk-data flag values against its constants).
No `unsafe` in this crate (the vendored decoder and cpal are the only third-party code involved).

## Format findings (verified on the whole 1.8 install: 50,733 SoundWaves in 112,462 packages)

The Codex task failed on `PickAxe_Enemy_001` with "no decodable Ogg/Vorbis payload". The stream was fine: the payload is a
plain Ogg/Vorbis stream stored **inline** after a small header; the old locator scanned the raw package bytes and handed
`lewton` everything after the first `OggS`, i.e. the stream plus the 16 trailing `FGuid` bytes. Reading the structure
properly (below) fixes it, and the same structure covers every wave in the install.

### `USoundWave` native tail (UE 4.16, cooked, unversioned packages)

After the tagged properties and the 4-byte `bHasGuid` flag (`fn-uasset`'s `ExportObject::trailing`):

```
non-streaming (bStreaming == false), 50,620 waves:
  i32  bCooked                         = 1
  i32  NumFormats                      = 1 in every wave
  NumFormats x { FName Name            (i32 index, i32 number -> "OGG")
                 FByteBulkData }
  FGuid CompressedDataGuid             (16 bytes)

streaming (bStreaming == true), 113 waves:
  i32  bCooked                         = 1
  FGuid CompressedDataGuid             (note: *before* the chunk data here)
  i32  NumChunks
  FName AudioDataFormat                ("OPUS")
  NumChunks x { i32 bCooked ; FByteBulkData ; i32 DataSize ; i32 AudioDataSize }
```

`FByteBulkData` = `u32 flags, i32 elementCount, i32 sizeOnDisk, i64 offsetInFile`, then the payload when it is inline.

| flags seen | meaning | payload location |
|---|---|---|
| `0x48` (SingleUse + ForceInline) | all 50,620 OGG blocks | immediately after the header, in `.uexp` |
| `0x40` (ForceInline) | chunk 0 of every OPUS wave | inline |
| `0x501` (PayloadAtEndOfFile + PayloadInSeperateFile + Force_NOT_InlinePayload) | chunks 1.. of OPUS waves (127 chunks) | `.ubulk` at `offsetInFile + Summary.BulkDataStartOffset`; the stored offset is **negative**, `BulkDataStartOffset` = size of `.uasset ++ .uexp` |

`PayloadAtEndOfFile` alone (offset + `BulkDataStartOffset` inside `.uasset ++ .uexp`) is implemented but does not occur.
Every export parses to the last byte (`leftover == 0`, no tail errors) in all 50,733 waves.
Chunks are 256 KiB (`DataSize = 262144`); only `AudioDataSize` bytes of each carry audio, the rest is zero padding.

Format names that occur: **`OGG` 50,620 (non-streaming) and `OPUS` 113 (streaming)**. `PCM`, `ADPCM`, `XMA`, `BINKA`
never occur; `PCM` (raw LE16) and `ADPCM` (WAV-wrapped MS ADPCM, mono/stereo, round-trip tested) are implemented from the
UE layout anyway.

### `OGG`

A normal Ogg logical stream (serial 0, libVorbis 1.3.x, `ENCODER=UnrealEngine4`). `lewton` decodes it; the result is
trimmed to the last page's granule position, which makes the length **exactly** `Duration * SampleRate` (50,729 of 50,733
waves exactly, the other 4 within one frame). Mono 2,722, stereo 48,011; rates 11025 (1), 16000 (5), 22050 (10),
24000 (46,909), 32000 (591), 44100 (118), 48000 (2,983), 96000 (3). `RawPCMDataSize` = frames x channels x 2.

### `OPUS` ("UE4OPUS", streaming waves = ambience / music loops)

UE's own framing around plain Opus packets (`FOpusAudioInfo`), after concatenating the chunks cut to `AudioDataSize`:

```
"UE4OPUS\0"        8 bytes
u16 sample rate    24000 (85 waves) or 48000 (28 waves); sources at 32 / 44.1 kHz are resampled to 48 kHz
u32 true sample count   exact output length in frames per channel
u8  channels
u16 frame count
frame count x { u16 packet size ; Opus packet }
```

Each packet is a **code-3 packet of three 20 ms frames** (one 60 ms "UE frame": 1440 samples at 24 kHz, 2880 at 48 kHz;
TOC bytes `0xDF`/`0xFF`, count byte `0x83` = VBR, 3 frames). No pre-skip is applied (UE does not). Property
`SampleRate` is the *source* rate and differs from the stream rate for 32/44.1 kHz sources, so the OPUS length check is
time-based (`Duration`). `opus-decoder` rejects packets > 1500 bytes (48 kHz music reaches ~1.8 KB, libopus only limits
a frame to 1275), so oversized packets are re-packetized into three single-frame packets (`split_code3`; unit-tested and
checked equal to whole-packet decoding on real data). Decoding speed is ~8x real time in release (pure Rust, no SIMD):
decode music on a loader thread (115 s of lobby music = ~14 s).

### Sweep results (`fn-audio-cli sweep`, 213 s on this machine with rayon, release)

* 50,733 / 50,733 SoundWaves parse and decode: **100.00 %**, **0 decoder panics**, 0 non-finite samples, 0 samples beyond
  full scale, 36.6 h (131,773 s) of audio, shortest 0.0625 s, longest 170.3 s.
* Duration: exact for 50,729, within 1 frame for 50,733 (spec: 1 frame or 0.5 %). Channels equal the property in all; rate equals the property in all
  OGG waves (OPUS: see above).
* 31 looping waves: seam step vs typical step median 0.73, 29 of 31 <= 3 (the two outliers are authored with a hard cut).
* Nothing stays undecodable. Fuzzing (truncations, byte flips, header corruption of real OGG / UE4OPUS payloads, mutated
  `.uasset`/`.uexp` bytes through parse -> decode, mutated SoundCue packages): 0 panics, errors only.

## SoundCue graphs

All **5,836** `SoundCue` packages parse (0 failures, 0 node references outside the install; 37,176 nodes, 53 `WavePlayer`s
with a null wave pointer, 49 cues with no `FirstNode`). Everything is in tagged properties (`ChildNodes` = export
references; `FirstNode` on the cue). Node classes and counts: WavePlayer 15,619, DialoguePlayer 5,491 (`DialogueWave` ->
first context's SoundWave via `Library::resolve_dialogue`), Modulator 4,817, Random 3,762, Mixer 2,062, Enveloper 2,029,
QualityLevel 938, Delay 1,066, Attenuation 676, SoundClass 454, Looping 151, DistanceCrossFade 40, GroupControl 28,
Doppler 19, ModulatorContinuous 18, ParamCrossFade 3, Switch 3 (`FS_Surfaces`). Engine defaults for absent properties
(cooked data stores only non-defaults): Modulator pitch/volume 0.95..1.05, Looping `bLoopIndefinitely = true` (all 151),
Delay 0..0, Random weights 1.0 / without-replacement false, Mixer/Concatenator input volume 1.0.

Semantics implemented in `mixer`: Random (weights, without replacement), Mixer / Concatenator (input volumes, sequential
timing from the children's real lengths), Modulator (pitch + volume, seeded RNG), Attenuation (subtree attenuation),
Looping (re-expands the subtree each iteration so Random re-picks; finite counts), Delay, Switch (int params), Branch,
QualityLevel (child index, default 0), ParamCrossFade / DistanceCrossFade (gain from the float param / listener
distance), Enveloper (volume + pitch curves over time, looping), SoundClass override, Dialogue. Simplifications:
Doppler and ModulatorContinuous pass through (no pitch shift), GroupControl plays all children, QualityLevel child 0 is
assumed to be the high level.

Cue properties used: `VolumeMultiplier`, `PitchMultiplier`, `Priority`, `SoundClassObject`, cue-level attenuation
(`AttenuationSettings` / `AttenuationOverrides`), concurrency (`ConcurrencyOverrides` or the `SoundConcurrency` asset:
`MaxCount`, `ResolutionRule`, `bLimitToOwner`, `VolumeScale`; all seven rules map to stop-oldest / prevent-new /
stop-farthest behaviour, StopQuietest/LowestPriority act as stop-oldest).

## SoundClass / SoundMix

103 SoundClasses, 50 SoundMixes (186 class adjusters). Volumes recovered from the assets equal
`reports/settings-catalog.json`: **Master 0.7, Music 0.5, SFX 0.5, Voice 0.8**; `Fort_01_VOIP` (parent Master) and
`Fort_UI` (parent SFX) set no volume (1.0). A cue's class volume is the product along the parent chain *including*
Master, e.g. `Fort_WeaponMelee_PSM -> Fort_WeaponMelee -> Fort_Weapons (0.8) -> Fort_06_Player_Attacks_Abilities ->
SFX (0.5) -> Master (0.7)` = 0.28, times the cue's `VolumeMultiplier` (0.3 for the pickaxe swing). Settings sliders map to
buses: Master -> `Bus::Master`, Music -> `Bus::Music`, Sound FX -> `Bus::Sfx` (UI sounds are under SFX), Dialog ->
`Bus::Voice`, Voice Chat -> `Bus::VoiceChat`; final gain = class chain x master slider x bus slider.
`Fort_WeaponMelee_PSM` carries a `PassiveSoundMixModifier` for `Fort_WeaponMelee_Duck_MixMod` (fade in 0.4 s, hold 1 s,
fade out 2 s; music x0.5, ambience x0.5, gadgets x0.5, impacts x0.8 ...). `SoundMix` push/pop with fades is implemented
in the mixer; the sink pushes this mix on a swing. The passive-modifier thresholds are not evaluated generically.

## Attenuation (`SoundAttenuation` assets + overrides)

**Recovered per asset:** shape + extents (the inner radius with no falloff, e.g. 300 cm), `FalloffDistance` (800),
`DistanceAlgorithm`, `OmniRadius` (200), `StereoSpread`, LPF radii/frequencies, `dBAttenuationAtMax`, custom curve keys
(and `CurveFloat` external curves), listener-focus values. Missing properties take the UE 4.16 constructor defaults
(documented on `Attenuation::default`). **Assumed:** the closed forms of the Logarithmic / Inverse / LogReverse /
NaturalSound curves (native code, not in the install) - they are monotone, 1 at the inner radius and 0 at
`inner + falloff`; Linear and Custom (Hermite/linear/constant keys, exact) are exact. Non-sphere shapes are reduced to
their first extent. Spatialisation: equal-power pan by listener-relative azimuth (behind folds to the front), blended by
`OmniRadius`; optional 1-pole low-pass (`bAttenuateWithLPF`). Not implemented: occlusion, listener focus, stereo spread,
Doppler, reverb sends.

## Gameplay event -> cue (`events::TABLE`; `fn-audio-cli events` verifies each loads, decodes and renders)

| event | cue | source |
|---|---|---|
| swing | `/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue` | **recovered**: `B_Melee_Impact_Pickaxe_Athena_C` CDO `PrimaryFireSound` (the montages have no sound notifies; played when the weapon fires = TriggerGameplayAbility notify) |
| equip / ready | `PickAxe_Ready_Athena_Cue` (+ `Player_Equip_Melee_Cue` via the `Melee_Equip` montage `AnimNotify_PlaySound`) | recovered (`ReloadSounds`) |
| impact (any surface) | `/Game/Sounds/Fort_Weapons/PickAxe/PickAxe_Impact_Default_Cue` | recovered: `ImpactPhysicalSurfaceSounds[0]`; `[1]` = `PickAxe_Impact_Husk_Cue`; index = `EPhysicalSurface` (Wood = SurfaceType1, Dirt = SurfaceType10) |
| harvest tree hit / destroyed | `/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Tree_Impact_Cue` (oak: `Demo_OakTree_Impact_Cue`, pine: `Demo_PineTree_Impact_Cue`) / `Demo_Tree_Destroyed_Cue` | recovered: tree BP `PlayHitSound` / `DeathSound` |
| harvest rock / wood / metal | `Demo_Rock_Impact_MLTI_Cue` / `Demo_Wood_Impact_MLTI_Cue` / `Demo_Metal_Impact_MLTI_Cue` | **assumed**: `Athena_Prop_Rocks_StoneLarge0x` (a `BuildingProp`) serialises no hit sound; these are `B_Melee_Generic` surface entries [2] / [1] / [3] |
| footsteps walk / sprint / crouch / crouch-sprint | `Athena_Character_{Walk,Sprint,Crouch,Crouch_Sprint}_Footstep_Cue` (`_3P_` variants for other players) | recovered: `PlayerPawn_Athena_C` `Default{,Fast,Crouched,CrouchSprint}FootstepSound`; **no grass-specific cue exists** in 1.8 Athena content (grass = default; only `Athena_Character_Bush_Footstep_Cue` differs) |
| jump / land / hard land | `Athena_Character_{Jump,Land,LandHard}_Cue` | recovered (`DefaultJumpSound`, `DefaultLandingSound`, `DefaultHardLandingSound`); hard-landing speed threshold assumed 9 m/s |
| menu open / close / click | `Fort_UI_MenuOpen_Cue` / `Fortnite_UI_Options_Menu_03_Cue` / `Fort_UI_MenuButtonClick_Cue` | assumed (name-matched) |
| island ambience | `Athena/Sounds/Ambient/Temperate_Grasslands_Hills_{Morning,Day,Evening,Night}_Athena_Cue` | assumed (which one plays is decided by the time-of-day Blueprint, not decoded) |

Surface footsteps/impacts: `AnimNotify_FootStep(_Left/_Right)` call native `FortPawn::PlayFootStepSound(FootIndex)`;
the native surface selection is not in the install.

## Using it

```rust
let mut lib = fn_audio::library::Library::open(r"C:\...\FortniteGame")?;
let mut eng = fn_audio::mixer::AudioEngine::new(48_000);          // offline; or fn_audio::device::open_default()
let h = eng.handle();
lib.prepare_cue(&h, fn_audio::events::SWING)?;                      // parse + decode + queue AddWave/AddCue
h.play_cue(fn_audio::events::SWING, Default::default())?;
let pcm = eng.render_offline(72_000, /*seed*/ 7);                    // interleaved stereo f32, deterministic
```

`EngineHandle` is `Clone + Send + Sync` and only pushes commands (`play_cue` -> `PlayId`, `stop`, `stop_cue`, `stop_all`,
`set_bus_volume`, `set_listener(position cm, forward, up)`, `set_suspended`, `push_mix`/`pop_mix`, `set_quality`,
`set_max_voices`); the `Mixer` drains them every 64 frames. Voice limit 48 (steals lowest priority, oldest first),
per-cue / per-asset concurrency rules, 1-pole LPF, linear gain ramps. World units are centimetres, right-handed y-up
listener basis (`fn-audio-sink` converts fn-app's metres).

Device output: `fn_audio::device::open_default()` (feature `device-output`) uses the default device's own rate and
sample format (f32/i16/u16/i32), writes stereo to the first two channels. The callback never locks or blocks.
Smoke run on this machine (once, 0.3 s, master 0.15): `SteelSeries Sonar virtual device, 96000 Hz, 8 ch`: OK.

## Verification

`cargo test -p fn-audio -p fn-audio-cli -p fn-audio-sink` (install-backed tests skip when the 1.8 install is missing;
`FN_ROOT` overrides the path): 32 unit tests (codecs, attenuation, curves, mixer behaviours, SoundClass math, events)
+ 10 real-install tests (about 40 s in the dev profile) + 3 CLI tests + 2 sink tests: the pickaxe wave decodes to exactly 11,599 frames; 61 waves (56 OGG +
5 OPUS, rates 16 / 22.05 / 24 / 32 / 44.1 / 48 kHz) match their Duration / channels / rate with worst delta 0 frames (the full sweep above covers all 50,733); three waves are written to
WAV and checked (header fields, round trip, peak/RMS, no NaN, clip runs <= 8, dominant frequency and band-limited
spectrum); looping OGG seams; a golden offline render of the swing cue (seed 7: 72,000 frames, peak 0.088005, RMS
0.008343, bit-identical on re-render, different seed differs); 3D pan/attenuation/cull; Opus split == whole decode;
fuzzing of payloads, packages and cue graphs.

Spectral checks (octave-band dB, `analyze`): the 115 s lobby Opus music decodes with a smooth music-like tilt
(<125 Hz -18, <1 kHz -31, <8 kHz -37, <16 kHz -43), no brick-wall or noise floor.

## Not done / assumptions

* The Opus decoder is slow (8x real time) and decodes whole waves; there is no incremental chunk streaming yet.
* Attenuation curve shapes other than Linear/Custom, non-sphere shapes, occlusion, focus, Doppler, reverb: see above.
* Footstep surface selection, per-resource harvest sounds for rocks/bricks/metal, the time-of-day ambience selection and
  the UI sound bindings are assumptions (flagged in `events::TABLE`).
* Passive SoundMix modifier thresholds are not generic (only the melee duck mix is wired, by the sink).
* The release profile uses `panic = "abort"`; `codec::guarded` turns third-party decoder panics into errors in dev/test
  builds only (zero occurred in the sweep and fuzzing).
* cpal's output stream is not `Send` on every platform: create it on the thread that owns it.
