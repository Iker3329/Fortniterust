//! F2 command bar of the Fortnite 1.8 port, as a pure (UI-free, window-free) library.
//!
//! * [`registry`]  the command table (names, aliases, usage, help, the real `UFortCheatManager` / engine `UCheatManager` function each one corresponds to)
//! * [`catalog`]   the items the sandbox can actually hold (built by the front-end from the real 1.8 item definitions) and the name / id resolution
//! * [`exec`]      parse + dispatch: movement / world commands act on the [`fn_game::GameSim`] directly, player-state commands come back as [`exec::Request`]s the front-end applies
//! * [`complete`]  tab-completion of command names and arguments (item ids / display names, cosmetic ids / display names)
//! * [`editor`]    the line editor: text input, scrollback, history (Up / Down), Tab cycling, open / close state
//!
//! The front-end (`fn-app/src/console_hook.rs` + `console_ui.rs`) owns the key capture, the drawing and the application of the requests to the live `Arms`.
pub mod catalog;
pub mod complete;
pub mod editor;
pub mod exec;
pub mod registry;

pub use catalog::{rarity_name, Catalog, ItemInfo, ItemKind, Resource, WeaponEntry};
pub use complete::{complete, Completion, Suggestion};
pub use editor::{Console, Key, View};
pub use exec::{execute, CosmeticKind, CosmeticSource, Effects, Env, Flags, LineKind, PakCosmetics, Request, Vitals};
pub use registry::{find as find_command, Arg, Command, COMMANDS};
