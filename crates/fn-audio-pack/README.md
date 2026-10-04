# fn-audio-pack

Exports only the sounds a game build needs into one `*.fnaudiopack`, so a game can ship sound without the Fortnite 1.8 paks. NEW, additive crate (nothing else in this workspace was changed).

A pack is a normal Unreal `.pak` (v3, mount `../../../`, entries stored uncompressed and unencrypted) holding the original `.uasset/.uexp/.ubulk` bytes of the packages reached by the exported SoundCues (cue -> SoundWave, SoundAttenuation, SoundConcurrency, SoundClass, DialogueWave, CurveFloat, SoundMix) plus every package of `/FortSoundClassesAndModes/`. Ogg/Vorbis and UE4OPUS payloads are kept as they are (no re-encoding). `fn_pak::PakSet` opens it unchanged, so every `fn-audio` loader works on it: `Library::new(fn_audio_pack::open_pack(path)?)`.

```
fn-audio-pack [--root <FortniteGame>] build <dir|file> [--manifest m.json] [--cue /Game/...]...   (default cues = events::TABLE + EXTRA_CUES)
fn-audio-pack list <pack>
fn-audio-pack verify <pack>     # every cue renders bit-identically from the pack and from the install; SoundClass tree and duck mix present
```

The old standalone runtime ships `assets/audio/audio.fnaudiopack` (22 cues, 309 packages, 618 files, 2.37 MB; `verify`: 22/22 identical).
