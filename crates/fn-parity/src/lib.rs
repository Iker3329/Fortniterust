//! fn-parity: the parity dashboard between the old prototype (`..\FortniteRuntime`, verified against Fortnite 1.8 data) and the pure-Rust rewrite.
//!
//! * [`run`]      builds, prepares a scratch copy of the old exe, runs both executables' scripted modes with timeouts ([`exec`])
//! * [`scenarios`] the matching new-game scripts for the old exe's fixed modes
//! * [`img`], [`logs`], [`perf`], [`consts`]  the comparers
//! * [`analyze`]  turns the outputs into scored items per area, [`report`] writes `parity/report.html` + `summary.json`
pub mod analyze;
pub mod compare;
pub mod config;
pub mod consts;
pub mod exec;
pub mod img;
pub mod logs;
pub mod model;
pub mod perf;
pub mod regen;
pub mod report;
pub mod run;
pub mod scenarios;
pub mod winproc;
