//! Single source of truth for the Fortnite build this runtime is built from (user-visible text must use these, never a literal).
//! Keep in sync with tools/fnroot.cjs / tools/fnroot.ps1 (VERSION) and README "Targeting Fortnite 1.8".
//! Evidence: <install>/Engine/Build/Build.version = UE 4.16.0, CL 3724489, branch ++Fortnite+Release-Live; the install's
//! FortniteClient-Win64-Shipping.exe is stamped 2017-10-28 (Manifest_DebugFiles_Win64.txt). The install carries no "1.8" string,
//! so the version name comes from the install folder / task brief.
pub const FORTNITE_VERSION: &str = "1.8";
#[allow(dead_code)] pub const BUILD_CL: &str = "3724489";
#[allow(dead_code)] pub const ENGINE_VERSION: &str = "4.16.0";
