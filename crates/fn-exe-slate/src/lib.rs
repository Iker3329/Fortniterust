//! Static, read-only analysis of the 1.8 client EXE (UE 4.16, x64 PE). The EXE is NEVER executed; it is only parsed and disassembled.
//!
//! * [`pe::Pe`]      sections, image base, RVA <-> file offset, `.pdata` function table, string reads (ASCII / UTF-16).
//! * [`xref`]        RIP-relative reference finder, call graph, function lookup.
//! * [`emu`]         emulator-lite: straight-line constant/register tracking over a function so Slate builder calls print their arguments.
pub mod emu;
pub mod findings;
pub mod pe;
pub mod report;
pub mod slate;
pub mod xref;

pub use pe::{Pe, Section};
pub use xref::{Xref, XrefKind, XrefIndex};

/// Default location of the analysed binary (override with `FN_EXE`).
pub const DEFAULT_EXE: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe";

/// Path of the EXE: `FN_EXE` env var or [`DEFAULT_EXE`].
pub fn exe_path() -> std::path::PathBuf { std::env::var_os("FN_EXE").map(Into::into).unwrap_or_else(|| DEFAULT_EXE.into()) }

/// The string FILE OFFSETS the user supplied (name, offset, hint of encoding is probed at runtime).
pub const USER_STRING_OFFSETS: &[(&str, u64)] = &[
    ("SInitialFortniteLoadingScreen", 0x44B7AB8),
    ("FortniteLoadingScreen.cpp", 0x44B7B20),
    ("FORT_Load_Atlas", 0x44B73A0),
    ("T-Loading-EarlyAccessPlate", 0x44B7440),
    ("Burbank", 0x44B7510),
    ("ProgressBar_Background", 0x44B7570),
    ("ProgressBar_Fill", 0x44B7600),
    ("ProgressBar_Marquee", 0x44B7680),
    ("EARLY ACCESS", 0x44B7B00),
    ("LOADING", 0x44B7C18),
    ("SFortLoadingScreen", 0x4AED1F8),
    ("SFortLoadingScreen.cpp", 0x4B3F980),
    ("FortLoadingScreenManager.cpp", 0x412D5F0),
];



