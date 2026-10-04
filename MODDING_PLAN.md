# Fortnite 1.8 offline lobby plan

- Install: `C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame` (user-provided local 1.8 build).
- Engine/data: cooked Unreal Engine 4.16-era package data; this work reads static local package and texture data only.
- Code/online boundary: the original executable is never launched. No encryption keys are recovered, no protected content is bypassed, and the new executable makes no network, account, payment, matchmaking, or purchase calls.
- Existing route: pure-Rust reimplementation in `C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime-rs`. `um` is not installed on this machine, so its read-only knowledge-base scan cannot run; the package inspection route is used instead.
- Package anchors recovered so far: `/Game/Athena/UI/Frontend/AthenaLobby`, `AthenaLobbyPlayerPanel`, `AthenaLobbyActions`, `AthenaLobbyDetails`, `/Game/Maps/UI/Frontend_Lobby`, and `FrontendCamera_Main`. The `AthenaLobby` widget tree contains `Launch`, `PlayerPanel`, `PlayerEmblemAndXp`, and `DailyQuestsPanel`; `Launch` contains `PlaylistRotator`, `TeamFillRotator`, `PlayButton`, and `CancelButton`.
- Chosen route: build a separate `fortnite-lobby.exe` from the existing Rust workspace. It will load local 1.8 UI textures/fonts and implement the serialized lobby controls as an offline state machine.
- Offline product boundary: local playlist and team-fill selection, ready/cancel state, locker/loadout selection, and catalog inspection can work locally. A live item shop’s offers, ownership, currency, purchases, friends, matchmaking, and service-backed quests cannot be reconstructed from static client packages and will not be imitated as functional services.
- Lab plan: do not modify the original install. Build into Cargo's `target\release` and launch only the newly built Rust binary. Verify package paths with `fn-inspect`, unit-test state transitions, and run the separate executable against the install path.
- Unknowns to resolve first: whether this build embeds an offline catalog or only a dynamic backend shop; authored widget geometry and all cosmetic metadata that are not serialized in the local packages.

## 2026-10-04 correction

The first standalone window incorrectly used `T_BattleRoyaleKeyArt` (the mode-picker art) as a backdrop. It is not the Athena lobby and is not a valid lobby deliverable. The actual static lobby world is `/Game/Maps/UI/Frontend_Lobby_Default`: it contains the streamed front-end map plus `S_UI_Sphere_noLOD`, `SM_Lobby_Floor`, `SM_Lobby_Halo`, `SM_Lobby_Room`, and `SM_Lobby_wall`; its lighting/fog/cubemap are serialized in the same map. The next vertical slice must render those world actors and the lobby pawn/podium before recreating any UI overlay.

## 2026-10-04 Halloween/profile slice

- The reference-screen world is present locally as `/Game/Maps/UI/Frontend_Lobby_Halloween`, with the Halloween LUT, candle actors, pumpkins, fog and graveyard meshes serialized in that map. It replaces the default-world target for this build.
- `fortnite-lobby.exe` now uses an offline profile at `%LOCALAPPDATA%\FortniteRuntime-rs\offline-lobby-profile.json`: a first-join display-name prompt, local level/XP display, and locally persisted outfit selection. It does not read, claim, or change an Epic account.
- The initial outfit choices are only the verified local hero packages `/Game/Athena/Heroes/HID_005_Athena_Commando_M` and `/Game/Athena/Heroes/HID_030_Athena_Commando_M_Halloween`; the latter matches the Halloween world. The runner starts the real `LobbyAthena` cue from the local paks.
- The cooked map and pawn are not rendered yet. The next slice must consume the map's world actors and load the selected `BrCharacter` into the existing Rust scene before calling the screen a lobby recreation.

## 2026-10-04 map extraction

- `crates/fn-app/src/lobby_world.rs` now reads `/Game/Maps/UI/Frontend_Lobby_Halloween.umap` directly from the user-provided 1.8 paks. The verified first pass recovers 88 directly placed static-mesh components, including `SM_Terrain_HalloweenLobby` terrain and Gothic Ruins grave meshes, plus their exact component transforms and material overrides. Blueprint child components (candles, lighting and particles) remain a separate class-default/template merge step; they are not replaced with hand-authored substitutes.
