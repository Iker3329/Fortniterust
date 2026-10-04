# FortniteRuntime-rs: the pure-Rust rewrite

Cargo workspace; every crate lives in `crates/`. The existing OpenGL/Node-tooling prototype stays untouched in `..\FortniteRuntime` until this reaches parity.
Definition of pure Rust here: everything that ships or runs (game, renderer, shaders, asset extraction) is Rust source. See PLAN.md.


## Asset foundation: `fn-pak`, `fn-uasset`, `fn-inspect`

Pure-Rust (no non-Rust dependency at build or run time; `memmap2`, `miniz_oxide`, `serde_json`, `rayon` only) reading of the
Fortnite paks and cooked UE 4.16 packages. Verified against the Node prototype's evidence reports and a full-install sweep:
see "Verification" below. Build with `CARGO_TARGET_DIR=C:\Users\Brady\AppData\Local\Temp\cargo-rs-pak`.

```text
fn-inspect --root <install> ls 'FortniteGame/Content/Athena/**/*.uasset' -l
fn-inspect cat /Game/Foo/Bar            > Bar.uasset      # raw bytes (long path or pak path)
fn-inspect props /Game/TimeOfDay/TODM/TODM_A_Child_Athena cdo      # tagged properties as JSON (export index | name | cdo | all)
fn-inspect classes /Game/Athena/HUD/AthenaHUD                       # classes, functions + disassembly, properties, enums, data tables
fn-inspect disasm /Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint
fn-inspect index > pak-index.json                                   # byte-identical to the Node tool's reports/pak-index.json
fn-inspect diff-index <installA|index.json> <installB|index.json>   # added / removed / size-changed by directory
fn-inspect sweep [--report out.json]                                # parse every .uasset/.umap (rayon) + coverage report
fn-inspect paks | info <pkg> | exports <pkg>
```
`--root` defaults to `$FN_ROOT` or the 1.8 `FortniteGame` directory; `--ue4-version 514` (or `$FN_UE4_VERSION`) selects the 1.10 layout.

### `fn-pak` public API
* `PakFile::open(path)` - one memory-mapped pak (`.info`, `.mount`, `.entries`, `.read(&PakEntry) -> Cow<[u8]>`, `.bytes()`); footer v1..=8
  inline index (1.8 = v3, 1.10 = v4), stored + zlib blocks, AES entries/indexes are reported (`PakError::Encrypted*`), never decrypted.
* `PakSet::open_install(dir)` / `open_dir` / `open_files` - mounts every pak of an install: **case-insensitive** lookup
  (`find`, `contains`, `read`, `read_game("/Game/..", "uasset")`), later pak overrides earlier (`pak_load_order`), `glob` (`*`, `**`, `?`),
  `effective()`, `all_entries()`, `paks()`, `total_entries()`.
* `PakEntry { path, offset, compressed_size, size, method, hash, blocks, encrypted, block_size }`, `normalize_path`, `glob_match`,
  `game_path_to_pak("/Game/A/B.B_C", "uasset") -> FortniteGame/Content/A/B.uasset`, `pak_path_to_game`, `find_paks_dir`.

### `fn-uasset` public API
* `Package::parse(asset, uexp)` / `parse_with(.., ParseOptions { assumed_ue4_version })` - `summary`, `names`, `imports`, `exports`, `depends`,
  `preload_dependencies`, `soft_package_references`, `thumbnails`; `export_bytes(i)`, `export_class_name(i)`, `find_export(name)`,
  `default_object_export()`, `class_export()`, `super_class()`, `object_path(PackageIndex)`, `resolve(PackageIndex) -> ObjectRef`.
* `PackageData::load(&PakSet, "/Game/..")` - pulls `.uasset`/`.uexp` (`.umap`) out of a pak set, `.package()` borrows it.
* `Package::read_export(i) -> ExportObject { name, class, props: PropertyList, trailing (native payload), error }`, `read_default_object()`,
  `read_export_named(name)`, `read_tagged_into(reader, &mut list)`.
* `Value` (typed tree: `Bool/Int/UInt/Float/Double/Byte/Enum/Name/Str/Text/Object/SoftObject/Array/Set/Map/Struct/Vector/Rotator/Quat/LinearColor/
  Color/Transform/...`, `Unparsed{ty, bytes, reason}`), accessors (`as_f32`, `as_str`, `as_vec3`, `field`, `path("A.B")`), `to_json(&Package)`;
  `PropertyList::{get, get_ci, get_all}`; every `Property` carries its tag byte `offset`.
* `classes::{read_class, read_function, read_property_decl, read_enum, read_script_struct, read_table, check_export, package_classes_json}`.
* `kismet::{disassemble, parse, Expr, Statement, Disassembly}`; `Expr::render()` reproduces `tools/kismet-disasm.cjs` text, `Expr::children()`.
* `native::{native_layout, is_native_struct}` - structs with native serializers; `Error { kind: ErrorKind, msg }`.

### Verification (all green: `cargo test -p fn-pak -p fn-uasset`)
* `fn-pak/tests/verify_index.rs`: entry count/paths/offsets/sizes/methods/encrypted flags identical to `reports/pak-index.json` (238,484 entries);
  `fn-inspect index` output is byte-identical to the Node file. Synthetic paks cover zlib blocks, v3/v4/v5 footers, mount points, override order.
* `fn-uasset/tests/verify_reports.rs` re-reads the packages behind `day-postprocess.json`, `sun-shadow-lighting.json` (incl. `.uexp` tag offsets),
  `camera-modes.json` (all modes with Blueprint inheritance, weapon selection, camera manager), `movement-overrides.json` (51 float tags at the
  same `.uexp` offsets) and `animation-state-machines.json` (baked state machines, CDO defaults, `CoreAnims_Male` map, and all 416
  Blueprint-variable assignments of the 47 KB `ExecuteUbergraph` bytecode: same statement offsets and same substituted expression text).
* `fn-uasset/tests/sweep.rs`: full-install sweeps (1.8 and 1.10) + mutation fuzzing on real packages; `tests/synthetic.rs`: offline package builder.
### Parse coverage (`fn-inspect sweep`, 20 threads, full pipeline incl. class/function/enum/table decoders and a Kismet pass over every function)
| install | packages (.uasset+.umap) | exports | hard failures | fully clean packages | property values decoded | time |
|---|---|---|---|---|---|---|
| Fortnite 1.8 (pak v3, 238,484 entries) | 113,538 | 1,448,905 | **0** | 99.821% | 99.993% of 19.7 M | ~0.7-1.6 s (2.4 s process wall incl. mount) |
| Fortnite 1.10 (pak v4, `--ue4-version 514`) | 115,818 | 1,411,828 | **0** | 99.716% | 99.972% of 18.1 M | ~1.0-1.6 s |

What is left (kept as `Value::Unparsed { ty, bytes, reason }`, never guessed; 203 packages in 1.8): Sequencer `MovieSceneGenerationLedger`
(UMG animations, 1,069 values), `ConnectivityCube` and `FortActorRecord` (Fort building patterns/instructions: opaque native blobs), and TMap
values whose struct type is not named in the property tag (14 values; would need class reflection). Native payloads of textures, meshes, sounds,
materials, animations stay raw in `ExportObject::trailing` (those belong to the texture/mesh/anim crates).