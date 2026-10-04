# fn-audio-cli

Command line front end of `fn-audio`. Install root from `--root <FortniteGame>`, `$FN_ROOT`, or the 1.8 default path.

```
fn-audio-cli list [substr]                       package paths under Sounds/Audio
fn-audio-cli info <package>                      SoundWave properties, bulk-data layout, streaming chunks
fn-audio-cli decode <package> <out.wav>          decode to 16-bit WAV, print peak/RMS/spectrum
fn-audio-cli sweep [report.json [issues.tsv]]    decode all SoundWaves, check vs Duration/rate/channels, summary JSON
fn-audio-cli cue <package>                       dump a SoundCue graph (nodes, attenuation, class, concurrency)
fn-audio-cli cues [report.json] [--all]          parse every SoundCue, node-class table, unresolved references
fn-audio-cli classes                             SoundClass tree with chain volumes and buses, SoundMix summary
fn-audio-cli events                              every gameplay-event cue loads, decodes and renders
fn-audio-cli render <cue> <out.wav> [--seed N] [--secs S] [--pos x,y,z] [--listener x,y,z] [--rate HZ]
fn-audio-cli analyze <in.wav>                    peak / RMS / spectrum / octave bands / loop-seam ratio
fn-audio-cli device-smoke                        (--features device-output) 0.3 s of the pickaxe swing at low volume
```

Examples:

```
cargo run --release -p fn-audio-cli -- render /Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue swing.wav --seed 7 --secs 1.5
cargo run --release -p fn-audio-cli -- render /Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue right.wav --pos 200,0,0 --listener 0,0,0
cargo run --release -p fn-audio-cli --features device-output -- device-smoke
```

Positions are Unreal centimetres; the listener looks down -Z with +Y up (so +X is to the right).
The format findings and verification numbers are in `crates/fn-audio/README.md`.
