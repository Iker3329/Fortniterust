//! The edit-mode state machine: one `UBuildingEditModeSupport` (+ the controller side of `AFortPlayerController` /
//! `AFortWeap_EditingTool`) re-expressed in Rust. Exe sources are cited per function (Ghidra names / addresses in
//! `C:\FortniteRef\decomp\named`).
//!
//! Flow in 1.8:
//! * Edit key (`PerformBuildingEditInteraction`, pressed; `AFortPlayerController::SetupInputComponent` @14084a570) on
//!   the focused building -> `FUN_140833550`: building not already being edited, pawn within
//!   [`EDIT_MODE_CANCEL_DISTANCE`] of its centroid -> `ServerBeginEditingBuildingActor` (permission:
//!   `ABuildingSMActor::vfunc_331` @1404646f0) -> edit tool equipped -> `LocalEnterEditMode`.
//! * Fire pressed -> `BeginEditAction` (support slot 73: `+0x240 |= 4`; stairs also clear the path). Every tick the
//!   `EditModeTrace` hits feed `vfunc_76` @1404bdb30: hover highlight, and while the action is active each newly hovered
//!   tile gets `PerformEditTileInteraction` (drag select). Fire released -> `EndEditAction` (slot 74).
//! * `BuildingEditReset` (pressed) -> reset (slot 75 per type).
//! * `CompleteBuildingEditInteraction` (pressed) / `GamepadEditBuilding` (released) -> `FUN_140837210`:
//!   `LocalExitEditMode(bCancel = false)` -> `ABuildingSMActor::vfunc_338` @140482180: if a valid state that differs
//!   from the actor (`vfunc_336` @14047b1b0) -> `ServerEditBuildingActor(class, RotationIterations, bMirrored)`,
//!   else `ServerEndEditingBuildingActor`. There is no confirm-on-release in 1.8: confirm is its own action.
//! * Unequipping the edit tool without confirming (`AFortWeap_EditingTool::OnUnEquip` @140932760:
//!   `LocalExitEditMode(PC, !bEditConfirmed)`) cancels. Leaving the range or the pawn dying cancels
//!   (`FUN_14084f8a0` from `AFortPlayerController::PlayerTick`).

use glam::{Quat, Vec3};

use crate::edit::data::{self as edit_data, stair, EditClass, EditDb, TileLayout, EDIT_MODE_CANCEL_DISTANCE};
use crate::edit::pattern::{find_match, EditMatch, Pattern, BTV_ACTIVE, BTV_EITHER, BTV_INACTIVE};
use crate::pieces::PieceKind;
use crate::world::{BuildingWorld, EditState, PieceId, PlacedPiece};

/// What the edit trace is over (`vfunc_76` resolves a hit component to an index with slot 81; stairs also index their
/// auxiliary indicator components).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Tile(usize),
    /// Stair auxiliary indicator 0..4.
    Aux(usize),
}

/// Sound / UI feedback events produced by the session (drained by the app; names are `FortUIFeedback` entries).
#[derive(Clone, Debug, PartialEq)]
pub enum EditEvent {
    /// `(feedback name, sound cue path)`.
    Sound(&'static str, &'static str),
    /// The pattern now matches a class (`vfunc_90` plays the `SuccessfulPatternMatchCurve` flash).
    MatchFlash,
    /// The tile pattern changed: `vfunc_80` @1404bc220 ends with `FUN_142213b10(+0x280, actor)`, the delegate bound to
    /// `AFortPlayerController::LocalOnBlueprintEdit` @140834c70 (`LocalEnterEditMode` vfunc_684 @140836d70 binds it); the app
    /// plays the edit montage from it (`build_anim::on_edit_tile_changed`).
    PatternChanged,
    /// Session ended; `applied` = the building was replaced.
    Ended { applied: bool },
}

/// Why an edit could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeginError {
    NoSuchPiece,
    /// Class is not a bIsPlayerBuildable class with `EditModePatternData` (`ABuildingSMActor::vfunc_330` @140494860).
    NotEditable,
    /// Another player edits it (`EditingPlayer` must be null or the caller, `vfunc_331`).
    BeingEdited,
    /// Pawn further than 750 from the centroid (`FUN_140832610`).
    OutOfRange,
    /// Team check of `vfunc_331`.
    WrongTeam,
    /// `vfunc_331` -> `FUN_140816020` @140816020 (input mode): see [`EditInputMode::allows_edit`].
    InputMode,
    /// `vfunc_330` @140494860: `vfunc_363(2)` - some `AttachmentPlacementBlockingActors` (+0xd90) entry is set (a trap or other
    /// attachment on the piece).
    HasAttachment,
    /// `vfunc_330`: the structural support system has a pending operation on the piece (`FUN_1404f1310`).
    StructuralPending,
}

/// The input state `FUN_140816020` @140816020 reads (called from `ABuildingSMActor::vfunc_331`, the edit permission):
/// returns true on a dedicated server, or when `UFortGlobals::bIsUsingGamepad` (+0x68) is false (keyboard / mouse), or when the
/// player's `UFortPlayerInputSettings::bEditModeUsableFromCombatMode` (+0x75, via `UFortPlayerInput` +0x4b8) is set, or when the
/// focused quickbar is the Secondary (building) bar (`FUN_1406865d0(+0x1a88) == 1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EditInputMode {
    pub using_gamepad: bool,
    pub edit_mode_usable_from_combat_mode: bool,
    pub building_bar_focused: bool,
}

impl EditInputMode {
    pub fn allows_edit(&self) -> bool { !self.using_gamepad || self.edit_mode_usable_from_combat_mode || self.building_bar_focused }
}

/// Per-piece state the edit permission reads that the [`BuildingWorld`] does not hold (`ABuildingSMActor::vfunc_330` @140494860).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EditBlockers {
    /// `AttachmentPlacementBlockingActors` has an entry (traps / attachments).
    pub has_attachment: bool,
    /// The structural module has a pending check on the piece (`crate::structural::Structural::pending`).
    pub structural_pending: bool,
}

/// `ABuildingSMActor::vfunc_331` @1404646f0 (can this controller edit this building): the building's team must be the
/// controller's team unless it is team 103 (`'g'`), the building must not be edited by someone else
/// (`EditingPlayer` +0xca0 null or the caller's PlayerState), not be destroyed (+0x554 bit 0) and pass `vfunc_330`
/// (player buildable + pattern data + no attachment in slot 2 + not in a pending structural operation).
pub fn can_edit(building_team: u8, player_team: u8, editing_player: Option<u32>, me: u32, editable: bool) -> Result<(), BeginError> {
    if building_team != 103 && building_team != player_team { return Err(BeginError::WrongTeam); }
    if let Some(p) = editing_player { if p != me { return Err(BeginError::BeingEdited); } }
    if !editable { return Err(BeginError::NotEditable); }
    Ok(())
}

/// Building centroid in world space (`ABuildingActor::GetCentroid` @14046ced0: actor transform * `CentroidOffset`).
/// `CentroidOffset` (+0x6ec) defaults per native class: Wall (0,0,192) (@14049fe20, DAT_14401ee38), Floor and Roof
/// (0,256,0) (@140459d10 / @14045b660, DAT_143ff903c), Stairs (0,256,192) (@14049f370). No player BP overrides it.
pub fn centroid_offset(kind: PieceKind) -> Vec3 {
    match kind {
        PieceKind::Wall => Vec3::new(0., 0., 192.),
        PieceKind::Floor | PieceKind::Roof => Vec3::new(0., 256., 0.),
        PieceKind::Stairs => Vec3::new(0., 256., 192.),
    }
}

/// `BaseLocToPivotOffset` (+0x6f8): every native ctor writes only Y = -256 (0xc3800000 at +0x6fc; Stairs inherits it
/// from `ABuildingFloor`). Unused by walls: they set `+0x554 |= 0x10` and rotate in place.
pub const BASE_LOC_TO_PIVOT_OFFSET: Vec3 = Vec3::new(0., -256., 0.);

fn yaw_quat(yaw_deg: f32) -> Quat { Quat::from_rotation_z(yaw_deg.to_radians()) }

pub fn centroid(p: &PlacedPiece) -> Vec3 { p.location + yaw_quat(p.yaw_deg) * centroid_offset(p.kind) }

/// UE `FRotator::NormalizeAxis`-style clamp to (-180, 180].
fn normalize_yaw(y: f32) -> f32 {
    let mut a = y % 360.;
    if a < 0. { a += 360.; }
    if a > 180. { a -= 360.; }
    a
}

/// New actor transform after `RotationIterations` quarter turns (`ABuildingSMActor::vfunc_329` @14048afb0,
/// `ReplaceBuildingActor`): walls (`+0x554 & 0x10`) add `iterations * 90` to the yaw in place; other pieces rotate
/// about the pivot (`FUN_1404642c0` @1404642c0: `loc' = loc - R*(-pivot) + R'*(-pivot)` written as
/// `loc + (-(R*pivot)) + R'*pivot`). Returns (location, yaw_deg).
pub fn rotated_transform(kind: PieceKind, location: Vec3, yaw_deg: f32, iterations: i32) -> (Vec3, f32) {
    let new_yaw = normalize_yaw(yaw_deg + iterations as f32 * 90.);
    if kind == PieceKind::Wall {
        return (location, new_yaw);
    }
    let pivot = BASE_LOC_TO_PIVOT_OFFSET;
    let loc = location - yaw_quat(yaw_deg) * pivot + yaw_quat(new_yaw) * pivot;
    (loc, new_yaw)
}

/// `FEditModeState` resolved to a class path.
#[derive(Clone, Debug, PartialEq)]
pub struct EditModeState {
    pub class_path: String,
    pub rotation_iterations: i32,
    pub mirrored: bool,
}

/// Per tile visual state (one `EditTileMeshComponents` entry with its `MGreen` MID).
#[derive(Clone, Debug, PartialEq)]
pub struct TileVisual {
    pub index: usize,
    /// Relative location in the edited actor's space, and relative scale.
    pub local_position: Vec3,
    pub local_scale: Vec3,
    /// Binary: tile is `BTV_Active` (texture `BlueprintTest1`, `isX` 0) vs inactive (`BlueprintTestX`, `isX` 1).
    /// Stairs: tile is on the path (value != 0).
    pub active: bool,
    /// Stair path step (0 = off); binary pieces 0.
    pub step: i32,
    /// `InteractParameter` = 1 (hovered, `vfunc_96`).
    pub highlighted: bool,
    /// The tile MID's `InteractParameter` as the exe last wrote it: 1 / 0 on hover on / off (`vfunc_96` @1404c4aa0; a stair
    /// indicator hover highlights its two tiles, `Stair::vfunc_96` @1404c4b20), the match-flash curve value while the flash
    /// timeline plays (every tile except the hovered one, `OnSuccessfulMatchInteractParamUpdate` @1404bd9a0). `None` = never
    /// written (material default). Renderers write this, not `highlighted`.
    pub interact: Option<f32>,
    /// `BlueColor` vector.
    pub color: [f32; 4],
    pub texture: &'static str,
}

/// Stair auxiliary indicator (`Stair::vfunc_102` @1404c8d60 + `vfunc_103`).
#[derive(Clone, Debug, PartialEq)]
pub struct AuxVisual {
    pub index: usize,
    pub local_position: Vec3,
    /// `vfunc_103`: 90 * index, + 180 when [`reversed`](Self::reversed).
    pub local_yaw_deg: f32,
    /// 0 = hint arrow, 1 = path arrow, 2 = idle grid, 3 = active/flat grid.
    pub mode: u8,
    pub texture: &'static str,
    /// The three scalar parameters `vfunc_103` writes, named by `edit_data::material_params::AUX_SCALARS`
    /// (`sineOffset`, `isblinkingSign`, `isX`).
    pub scalars: [f32; 3],
    /// Arrow direction flag (`vfunc_102` -> `vfunc_103` param 3): from `stair::ADJACENCY_REVERSED` for arrow modes 0 / 1
    /// (inverted on a mirrored actor where `stair::ADJACENCY_MIRROR_FLIPS` is set), always false for the grid modes 2 / 3.
    pub reversed: bool,
}

/// The matched class shown with `PreviewMaterial`.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewVisual {
    pub class_path: String,
    pub static_mesh: Option<String>,
    pub location: Vec3,
    pub yaw_deg: f32,
    /// X scale -1 (`vfunc_90`: `RelativeScale3D.X = mirrored ? -|X| : |X|`).
    pub mirrored: bool,
    pub visible: bool,
    pub material: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditVisuals {
    pub piece: PieceId,
    pub actor_location: Vec3,
    pub actor_yaw_deg: f32,
    pub actor_mirrored: bool,
    pub tile_mesh: &'static str,
    pub tile_material: &'static str,
    pub tiles: Vec<TileVisual>,
    pub aux: Vec<AuxVisual>,
    pub preview: Option<PreviewVisual>,
}

/// One active edit (client + server halves merged: single player, no replication).
#[derive(Clone, Debug)]
pub struct EditSession {
    pub piece: PieceId,
    pub kind: PieceKind,
    /// Class in effect when the edit started.
    pub owner_class: String,
    pub owner_mirrored: bool,
    /// `PreviewMetadata` (the tile state being edited).
    pub preview: Pattern,
    /// `PossibleEditClasses` (class paths, candidate order).
    candidates: Vec<String>,
    candidate_patterns: Vec<Option<Pattern>>,
    candidate_meshes: Vec<Option<String>>,
    /// Build numbers of each candidate (the edit rebuilds the new actor, [`apply_edit`]).
    candidate_builds: Vec<Option<edit_data::ClassBuild>>,
    /// `SuccessfulPatternMatchCurve` and its `FTimeline` (+0x160) position while playing (`None` = stopped).
    flash_curve: edit_data::FlashCurve,
    flash_pos: Option<f32>,
    /// Per tile MID `InteractParameter` (see [`TileVisual::interact`]).
    tile_interact: Vec<Option<f32>>,
    /// Roof: each tile's current RelativeLocation (eased to its target by `Roof::vfunc_76`).
    roof_pos: Vec<Vec3>,
    /// `PreviewEditModeState` + `+0x135` (has a valid match).
    pub state: EditModeState,
    pub state_valid: bool,
    /// `+0x240` bit 2 (`bEditActionInProgress`).
    pub action_in_progress: bool,
    /// `LastInteractedComp` / `LastHighlightedComp`.
    pub last_interacted: Option<EditTarget>,
    pub highlighted: Option<EditTarget>,
    /// `CurToggleAction` (+0x298): the value the first tile of a drag was set to.
    cur_toggle_action: u8,
    /// Stair: `+0x338` step count, `ActivatedAuxIndicatorComponent` (+0x2d0), `LastValidMetadataConfiguration`.
    pub stair_steps: i32,
    pub activated_aux: Option<usize>,
    last_valid: Pattern,
    /// Last colour pushed to the tiles (`BlueColor`).
    tile_color: [f32; 4],
    layout: TileLayout,
    aux_bounds: (Vec3, Vec3),
    events: Vec<EditEvent>,
}

impl EditSession {
    /// Edit-key press on `piece`: range + permission checks, then enter edit mode (`UBuildingEditModeSupport::vfunc_71`
    /// @1404bc2a0: support init, `PossibleEditClasses` (`vfunc_83`), tiles (`vfunc_84`), preview (`vfunc_86`), Enter
    /// sound). `pawn_location` is the controlled pawn's location. Pieces in the [`BuildingWorld`] are the player's own
    /// builds, so the team test of [`can_edit`] passes; nobody else edits in single player.
    pub fn begin(db: &EditDb, world: &BuildingWorld, piece: PieceId, pawn_location: Vec3) -> Result<EditSession, BeginError> {
        Self::begin_checked(db, world, piece, pawn_location, EditInputMode::default(), EditBlockers::default())
    }

    /// [`begin`](Self::begin) with the permission inputs the world does not hold: the range test of `FUN_140833550`
    /// @140833550, then `ABuildingSMActor::vfunc_331` @1404646f0 (input mode `FUN_140816020`) and `vfunc_330` @140494860
    /// (attachments `vfunc_363(2)`, pending structural operation `FUN_1404f1310`).
    pub fn begin_checked(db: &EditDb, world: &BuildingWorld, piece: PieceId, pawn_location: Vec3, input: EditInputMode, blockers: EditBlockers) -> Result<EditSession, BeginError> {
        let p = world.get(piece).ok_or(BeginError::NoSuchPiece)?;
        let cls = db.class(p.effective_class()).ok_or(BeginError::NotEditable)?;
        let pat = db.pattern_of(cls).ok_or(BeginError::NotEditable)?.clone();
        // FUN_140832610: |pawn - centroid|^2 <= EditModeCancelDistance^2.
        if (pawn_location - centroid(p)).length_squared() > EDIT_MODE_CANCEL_DISTANCE * EDIT_MODE_CANCEL_DISTANCE {
            return Err(BeginError::OutOfRange);
        }
        // vfunc_331: FUN_140816020 (input mode) before the team / editing-player / vfunc_330 checks.
        if !input.allows_edit() { return Err(BeginError::InputMode); }
        // vfunc_330: `!vfunc_363(2)` (no attachment) and `!FUN_1404f1310(structural system, actor)`.
        if blockers.has_attachment { return Err(BeginError::HasAttachment); }
        if blockers.structural_pending { return Err(BeginError::StructuralPending); }
        Ok(Self::enter(db, p, cls, pat))
    }

    fn enter(db: &EditDb, p: &PlacedPiece, cls: &EditClass, pat: Pattern) -> EditSession {
        let cands = db.possible_edit_classes(cls);
        let candidates: Vec<String> = cands.iter().map(|c| c.class_path.clone()).collect();
        let candidate_patterns: Vec<Option<Pattern>> = cands.iter().map(|c| db.pattern_of(c).cloned()).collect();
        let candidate_meshes: Vec<Option<String>> = cands.iter().map(|c| c.static_mesh.clone()).collect();
        let candidate_builds: Vec<Option<edit_data::ClassBuild>> = cands.iter().map(|c| c.build).collect();
        let owner_mirrored = p.edit.mirrored;
        let layout = db.layout(p.kind);
        let n = pat.len();
        let mut s = EditSession {
            piece: p.id,
            kind: p.kind,
            owner_class: cls.class_path.clone(),
            owner_mirrored,
            preview: pat.clone(),
            candidates,
            candidate_patterns,
            candidate_meshes,
            candidate_builds,
            flash_curve: db.match_flash.clone(),
            flash_pos: None,
            tile_interact: vec![None; n],
            // Roof::vfunc_84 @1404b8a70: the tiles spawn at their base position (BinaryToggle::vfunc_84), targets differ.
            roof_pos: (0..n).map(|i| layout.tile_position(i)).collect(),
            // vfunc_86: initial preview state = { owner class, 0, owner bMirrored }, shown via vfunc_90 (valid = 1).
            state: EditModeState { class_path: cls.class_path.clone(), rotation_iterations: 0, mirrored: owner_mirrored },
            state_valid: true,
            action_in_progress: false,
            last_interacted: None,
            highlighted: None,
            cur_toggle_action: BTV_ACTIVE,
            stair_steps: 0,
            activated_aux: None,
            last_valid: pat,
            tile_color: edit_data::DEFAULT_BLUE_TILE_COLOR,
            layout,
            aux_bounds: db.stair_aux_bounds,
            events: Vec::new(),
        };
        // vfunc_71 @1404bc2a0 -> vfunc_86 @1404b82e0 shows the owner state through vfunc_90, which restarts the flash timeline
        // (PlayFromStart); vfunc_88 binds its track afterwards, so the flash plays from the first tick (no update at 0 here).
        s.flash_pos = Some(0.0);
        s.events.push(EditEvent::MatchFlash);
        s.events.push(EditEvent::Sound(edit_data::sounds::ENTER.0, edit_data::sounds::ENTER.1));
        s
    }

    /// `vfunc_90` @1404c5bb0 tail: `FUN_1424da730(+0x160)` = `FTimeline::PlayFromStart`: `SetPlaybackPosition(0, false, true)`
    /// (fires the update with the value at 0 once the track is bound) and starts playing. At enter the track is bound later
    /// (`vfunc_88` runs after `vfunc_86`), so nothing is written until the first tick.
    fn play_flash(&mut self) {
        self.flash_pos = Some(0.0);
        let v = self.flash_curve.eval(0.0);
        for i in 0..self.tile_interact.len() {
            if self.highlighted != Some(EditTarget::Tile(i)) { self.tile_interact[i] = Some(v); }
        }
        self.events.push(EditEvent::MatchFlash);
    }

    /// Per-frame part of `UBuildingEditModeSupport::vfunc_76` @1404bdb30 (`FUN_1424e3730` = `FTimeline::TickTimeline(dt)`) and
    /// `Roof::vfunc_76` @1404bdc90 (tile easing).
    /// * Flash: position += dt, clamped to the `TL_LastKeyFrame` length where it stops; every tick writes the curve value at the new
    ///   position to every tile's `InteractParameter` except the hovered tile.
    /// * Roof: each tile's RelativeLocation = `VInterpTo(current, target, dt, 4)`, target = base + (0,0,32) when the tile is
    ///   `BTV_Active` (`Roof::vfunc_97` @1404c4480).
    pub fn tick(&mut self, dt: f32) {
        if let Some(pos) = self.flash_pos {
            let len = self.flash_curve.length();
            let mut np = pos + dt;
            let mut finished = false;
            if dt > 0.0 && np > len { np = len; finished = true; }
            let v = self.flash_curve.eval(np);
            for i in 0..self.tile_interact.len() {
                if self.highlighted != Some(EditTarget::Tile(i)) { self.tile_interact[i] = Some(v); }
            }
            self.flash_pos = if finished { None } else { Some(np) };
        }
        if self.kind == PieceKind::Roof {
            for i in 0..self.roof_pos.len() {
                let target = self.roof_target(i);
                self.roof_pos[i] = edit_data::vinterp_to(self.roof_pos[i], target, dt, edit_data::ROOF_TILE_INTERP_SPEED);
            }
        }
    }

    /// `Roof::vfunc_84` / `vfunc_97`: base position, + `ROOF_ACTIVE_TILE_RAISE` while the tile is `BTV_Active`.
    fn roof_target(&self, i: usize) -> Vec3 {
        let base = self.layout.tile_position(i);
        if self.preview.binary(i) == Some(BTV_ACTIVE) { base + edit_data::ROOF_ACTIVE_TILE_RAISE } else { base }
    }

    /// The tile's current local position (roof tiles ease, the others are fixed).
    pub fn tile_local_position(&self, i: usize) -> Vec3 {
        if self.kind == PieceKind::Roof { self.roof_pos.get(i).copied().unwrap_or_else(|| self.layout.tile_position(i)) } else { self.layout.tile_position(i) }
    }

    /// `vfunc_96` (+ `Stair::vfunc_96` @1404c4b20 for an indicator: its two tiles) with `on`.
    fn set_hover_param(&mut self, t: EditTarget, on: bool) {
        let v = if on { 1.0 } else { 0.0 };
        match t {
            EditTarget::Tile(i) => { if let Some(x) = self.tile_interact.get_mut(i) { *x = Some(v); } }
            EditTarget::Aux(a) => {
                let (x, y) = stair::AUX_PAIRS[a];
                for i in [x, y] { if let Some(s) = self.tile_interact.get_mut(i) { *s = Some(v); } }
            }
        }
    }

    pub fn drain_events(&mut self) -> Vec<EditEvent> { std::mem::take(&mut self.events) }

    /// Fire pressed: `BeginEditAction` (slot 73). Binary @1404bbeb0: `+0x240 |= 4`. Stair @1404bbec0: also clears the
    /// activated indicator, the step counter and every tile (slot 100 @1404a9fd0).
    pub fn begin_action(&mut self) {
        self.action_in_progress = true;
        if self.kind == PieceKind::Stairs {
            self.activated_aux = None;
            self.stair_steps = 0;
            for i in 0..self.preview.len() { self.preview.set_stair(i, 0); }
        }
    }

    /// Fire released: `EndEditAction` (slot 74 @1404bbcd0). Applies the end-of-action tile colour / preview visibility
    /// (slot 92: BinaryToggle @1404ca580 = valid ? `DefaultBlueTileColor` : `InvalidEditPatternColor`; stairs @1404ca560
    /// only toggle the preview), clears the action. Stairs (@1404bbd00) then restore `LastValidMetadataConfiguration`
    /// into the tiles and re-show its state (slot 101 @1404c2a90).
    pub fn end_action(&mut self) {
        self.apply_end_colour();
        self.action_in_progress = false;
        self.last_interacted = None;
        if self.kind == PieceKind::Stairs {
            self.activated_aux = None;
            self.preview = self.last_valid.clone();
            // Stair::vfunc_101 @1404c2a90: restore LastValidMetadataConfiguration into the tiles, then vfunc_90 @1404c5bb0 with
            // the stored PreviewEditModeState (+0x128, the last successful match): `+0x135 = 1` (valid again) and the match
            // flash restarts; then vfunc_102 (indicators) and vfunc_92.
            self.state_valid = true;
            self.play_flash();
            self.apply_end_colour();
        }
    }

    fn apply_end_colour(&mut self) {
        if self.kind != PieceKind::Stairs {
            self.tile_color = if self.state_valid { edit_data::DEFAULT_BLUE_TILE_COLOR } else { edit_data::INVALID_EDIT_PATTERN_COLOR };
        }
    }

    /// One tick of `vfunc_76` with the target under the edit trace (`None` = nothing). Updates the hover highlight and,
    /// during an action, interacts with a newly hovered target (drag select). Returns true if the pattern changed.
    pub fn update_hover(&mut self, target: Option<EditTarget>) -> bool {
        let target = target.filter(|t| self.valid_target(*t));
        if self.highlighted != target {
            // vfunc_76 @1404bdb30: vfunc_96(old, false), LastHighlightedComp = new, vfunc_96(new, true).
            if let Some(old) = self.highlighted { self.set_hover_param(old, false); }
            self.highlighted = target;
            if let Some(new) = target { self.set_hover_param(new, true); }
        }
        let mut changed = false;
        if self.action_in_progress {
            if let Some(t) = target {
                if self.last_interacted != Some(t) {
                    if self.perform_interaction(t) {
                        self.on_pattern_changed();
                        changed = true;
                    }
                    self.last_interacted = Some(t);
                }
            }
        }
        changed
    }

    fn valid_target(&self, t: EditTarget) -> bool {
        match t {
            EditTarget::Tile(i) => i < self.preview.len(),
            EditTarget::Aux(i) => self.kind == PieceKind::Stairs && i < stair::AUX_PAIRS.len(),
        }
    }

    /// Convenience for a click without drag: press, interact with `target`, release.
    pub fn click(&mut self, target: EditTarget) -> bool {
        self.begin_action();
        let c = self.update_hover(Some(target));
        self.end_action();
        c
    }

    /// `PerformEditTileInteraction` (slot 79).
    fn perform_interaction(&mut self, t: EditTarget) -> bool {
        match self.kind {
            PieceKind::Stairs => self.stair_interaction(t),
            _ => {
                let EditTarget::Tile(i) = t else { return false };
                // BinaryToggle::vfunc_79 @1404be8f0: first tile of the action toggles it and records the new value;
                // later tiles of the same drag are forced to that value.
                if self.last_interacted.is_none() {
                    let v = self.preview.binary(i).unwrap_or(BTV_ACTIVE);
                    let changed = self.set_tile_active(i, v != BTV_ACTIVE);
                    self.cur_toggle_action = self.preview.binary(i).unwrap_or(BTV_ACTIVE);
                    changed
                } else {
                    self.set_tile_active(i, self.cur_toggle_action == BTV_ACTIVE)
                }
            }
        }
    }

    /// `SetTileActive` (slot 97). BinaryToggle @1404c43a0: Either always changes, Active only to inactive, Inactive only
    /// to active. Roof @1404c4480: Either never changes.
    fn set_tile_active(&mut self, i: usize, active: bool) -> bool {
        let Some(v) = self.preview.binary(i) else { return false };
        let change = match v {
            BTV_EITHER => self.kind != PieceKind::Roof,
            BTV_ACTIVE => !active,
            BTV_INACTIVE => active,
            _ => false,
        };
        if change { self.preview.set_binary(i, if active { BTV_ACTIVE } else { BTV_INACTIVE }); }
        change
    }

    /// `UBuildingEditModeSupport_Stair::vfunc_79` @1404be9a0 (path drawing).
    fn stair_interaction(&mut self, t: EditTarget) -> bool {
        match t {
            EditTarget::Aux(a) => {
                if self.stair_steps != 0 {
                    match self.activated_aux {
                        None => return false,
                        Some(x) if x == a => return false,
                        _ => {}
                    }
                }
                self.stair_steps += 1;
                let step = self.stair_steps;
                if self.activated_aux.is_none() {
                    let (x, y) = stair::AUX_PAIRS[a];
                    self.preview.set_stair(x, step);
                    self.preview.set_stair(y, step);
                } else {
                    self.fill_unset(step);
                }
                self.activated_aux = Some(a);
                true
            }
            EditTarget::Tile(i) => {
                let v = self.preview.stair(i).unwrap_or(0);
                if self.stair_steps >= stair::MAX_STEPS || v > 0 || (self.activated_aux.is_none() && !self.stair_can_select(i)) {
                    return false;
                }
                self.stair_steps += 1;
                let step = self.stair_steps;
                self.preview.set_stair(i, step);
                if self.activated_aux.is_some() { self.fill_unset(step); }
                true
            }
        }
    }

    fn fill_unset(&mut self, step: i32) {
        for j in 0..self.preview.len() {
            if self.preview.stair(j) == Some(0) { self.preview.set_stair(j, step); }
        }
    }

    /// `Stair::vfunc_104` @1404a9a00: after the first step (and with no indicator active) a tile must be adjacent to the
    /// tile holding the current step.
    fn stair_can_select(&self, i: usize) -> bool {
        if self.stair_steps > 0 && self.activated_aux.is_none() {
            for j in 0..self.preview.len() {
                if self.preview.stair(j) == Some(self.stair_steps) {
                    return stair::ADJACENCY[j][i] != -1;
                }
            }
        }
        true
    }

    /// `vfunc_80` @1404bc220: tile sound, then find the matching class (`vfunc_93`); a match becomes the preview
    /// (`vfunc_90`, tiles blue, flash), no match only clears `+0x135` (slot 91 @1404bb3c0) - the colour turns red at
    /// the end of the action.
    fn on_pattern_changed(&mut self) {
        self.events.push(EditEvent::Sound(edit_data::sounds::TILE.0, edit_data::sounds::TILE.1));
        let cands: Vec<Option<&Pattern>> = self.candidate_patterns.iter().map(|p| p.as_ref()).collect();
        match find_match(self.kind, &self.preview, self.owner_mirrored, &cands) {
            Some(EditMatch { class_index, rotation_iterations, mirrored }) => {
                self.state = EditModeState { class_path: self.candidates[class_index].clone(), rotation_iterations, mirrored };
                self.state_valid = true;
                if self.kind == PieceKind::Stairs {
                    // Stair::vfunc_93 @1404ae170: LastValidMetadataConfiguration <- StairPreviewMetadata.
                    self.last_valid = self.preview.clone();
                } else {
                    self.tile_color = edit_data::DEFAULT_BLUE_TILE_COLOR;
                }
                // vfunc_90: PlayFromStart of the match flash.
                self.play_flash();
            }
            None => self.state_valid = false,
        }
        // vfunc_80 tail: FUN_142213b10(+0x280) -> LocalOnBlueprintEdit.
        self.events.push(EditEvent::PatternChanged);
    }

    /// `BuildingEditReset`: slot 75. Binary @1404bbd40: every tile active. Roof @1404bbdb0: that, then every tile
    /// inactive. Stairs @1404bbe20: begin action, tiles (2,3) = 1, (0,1) = 2 (the straight ramp). Each runs
    /// `vfunc_80` and `EndEditAction`.
    pub fn reset(&mut self) {
        match self.kind {
            PieceKind::Stairs => {
                self.begin_action();
                for i in stair::RESET_ONE { self.preview.set_stair(i, 1); }
                let (a, b) = stair::AUX_PAIRS[0];
                self.preview.set_stair(a, 2);
                self.preview.set_stair(b, 2);
                self.on_pattern_changed();
                self.end_action();
            }
            _ => {
                for i in 0..self.preview.len() { self.set_tile_active(i, true); }
                self.on_pattern_changed();
                self.end_action();
                if self.kind == PieceKind::Roof {
                    for i in 0..self.preview.len() { self.set_tile_active(i, false); }
                    self.on_pattern_changed();
                    self.end_action();
                }
            }
        }
    }

    /// `ABuildingSMActor::vfunc_336` @14047b1b0: valid and different from the actor's current class / mirror, or rotated.
    pub fn differs(&self) -> bool {
        self.state_valid
            && (self.state.class_path != self.owner_class || self.state.rotation_iterations != 0 || self.state.mirrored != self.owner_mirrored)
    }

    /// Confirm (`CompleteBuildingEditInteraction`): `LocalExitEditMode(false)`. Replaces the piece when [`differs`];
    /// returns whether it did. Plays the Exit sound (support teardown `vfunc_72` @1404bc470).
    pub fn confirm(mut self, world: &mut BuildingWorld) -> (bool, Vec<EditEvent>) {
        let build = self.candidates.iter().position(|c| *c == self.state.class_path).and_then(|i| self.candidate_builds[i]);
        let applied = self.differs() && apply_edit(world, self.piece, &self.state, build);
        self.events.push(EditEvent::Sound(edit_data::sounds::EXIT.0, edit_data::sounds::EXIT.1));
        self.events.push(EditEvent::Ended { applied });
        (applied, self.events)
    }

    /// Cancel (`LocalExitEditMode(true)`: tool unequipped without confirm, out of range, death).
    pub fn cancel(mut self) -> Vec<EditEvent> {
        self.events.push(EditEvent::Sound(edit_data::sounds::EXIT.0, edit_data::sounds::EXIT.1));
        self.events.push(EditEvent::Ended { applied: false });
        self.events
    }

    /// PlayerTick check (`FUN_14084f8a0`): false = the edit must be cancelled (piece gone or pawn out of range).
    pub fn still_valid(&self, world: &BuildingWorld, pawn_location: Vec3) -> bool {
        match world.get(self.piece) {
            Some(p) => (pawn_location - centroid(p)).length_squared() <= EDIT_MODE_CANCEL_DISTANCE * EDIT_MODE_CANCEL_DISTANCE,
            None => false,
        }
    }

    pub fn layout(&self) -> &TileLayout { &self.layout }

    /// Snapshot for the renderer.
    pub fn visuals(&self, world: &BuildingWorld) -> Option<EditVisuals> {
        let p = world.get(self.piece)?;
        let tex_active = edit_data::assets::ACTIVE_TILE_TEXTURE;
        let tex_inactive = edit_data::assets::INACTIVE_TILE_TEXTURE;
        let tiles = (0..self.preview.len())
            .map(|i| {
                let (active, step) = match self.kind {
                    PieceKind::Stairs => { let s = self.preview.stair(i).unwrap_or(0); (s != 0, s) }
                    _ => (self.preview.binary(i) == Some(BTV_ACTIVE), 0),
                };
                TileVisual {
                    index: i,
                    // roof tiles ease to base + 32 Z while active (Roof::vfunc_76)
                    local_position: self.tile_local_position(i),
                    local_scale: self.layout.scale,
                    active,
                    step,
                    highlighted: self.highlighted == Some(EditTarget::Tile(i)),
                    interact: self.tile_interact.get(i).copied().flatten(),
                    color: self.tile_color,
                    texture: if active { tex_active } else { tex_inactive },
                }
            })
            .collect();
        let aux = if self.kind == PieceKind::Stairs { self.aux_visuals() } else { Vec::new() };
        let preview = self.candidates.iter().position(|c| *c == self.state.class_path).map(|ci| {
            let (loc, yaw) = rotated_transform(self.kind, p.location, p.yaw_deg, self.state.rotation_iterations);
            PreviewVisual {
                class_path: self.state.class_path.clone(),
                static_mesh: self.candidate_meshes[ci].clone(),
                location: loc,
                yaw_deg: yaw,
                mirrored: self.state.mirrored,
                visible: self.state_valid || self.action_in_progress,
                material: edit_data::assets::PREVIEW_MATERIAL,
            }
        });
        Some(EditVisuals {
            piece: self.piece,
            actor_location: p.location,
            actor_yaw_deg: p.yaw_deg,
            actor_mirrored: self.owner_mirrored,
            tile_mesh: TileLayout::tile_mesh(self.kind),
            tile_material: edit_data::assets::TILE_MATERIAL,
            tiles,
            aux,
            preview,
        })
    }

    /// `Stair::vfunc_102` @1404c8d60 (indicator modes) + `vfunc_103` (parameters).
    fn aux_visuals(&self) -> Vec<AuxVisual> {
        let n = self.preview.len();
        let mut mode = [2u8; 4];
        // second byte of each per-indicator entry: the arrow direction flag (DAT_1440994c4, flipped on a bMirrored actor where
        // DAT_1440994c5 is set)
        let mut reversed = [false; 4];
        let dir = |a: usize, b: usize| stair::ADJACENCY_REVERSED[a][b] ^ (self.owner_mirrored && stair::ADJACENCY_MIRROR_FLIPS[a][b]);
        let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); n];
        for i in 0..n {
            let v = self.preview.stair(i).unwrap_or(0);
            if v >= 1 && (v as usize) <= n { buckets[v as usize - 1].push(i); }
        }
        for s in 0..n {
            if buckets[s].is_empty() { break; }
            let next = buckets.get(s + 1).filter(|b| !b.is_empty());
            match next {
                Some(nb) => {
                    for &a in &buckets[s] {
                        for &b in nb {
                            let ind = stair::ADJACENCY[a][b];
                            if ind >= 0 { mode[ind as usize] = 1; reversed[ind as usize] = dir(a, b); }
                        }
                    }
                }
                None => {
                    if self.action_in_progress {
                        for &a in &buckets[s] {
                            for u in 0..n {
                                let ind = stair::ADJACENCY[a][u];
                                if ind >= 0 && self.preview.stair(u) == Some(0) { mode[ind as usize] = 0; reversed[ind as usize] = dir(a, u); }
                            }
                        }
                    }
                    break;
                }
            }
        }
        (0..4)
            .map(|i| {
                let (x, y) = stair::AUX_PAIRS[i];
                let vx = self.preview.stair(x).unwrap_or(0);
                let vy = self.preview.stair(y).unwrap_or(0);
                if self.activated_aux == Some(i) || (vx >= 1 && vx == vy) { mode[i] = 3; }
                // vfunc_102 -> vfunc_103: arrow modes keep their direction flag, grid modes pass 0
                let texture = if mode[i] <= 1 { edit_data::assets::STAIR_AUX_ARROW_TEXTURE } else { edit_data::assets::STAIR_AUX_GRID_TEXTURE };
                let rev = mode[i] <= 1 && reversed[i];
                AuxVisual {
                    index: i,
                    local_position: Vec3::from(stair::AUX_LOCATIONS[i]),
                    // vfunc_103: rotator (0, 90, 0) * i, yaw + 180 (DAT_143ff9038) when the direction flag is set
                    local_yaw_deg: stair::AUX_YAW_STEP * i as f32 + if rev { stair::AUX_HIGHLIGHT_YAW } else { 0. },
                    mode: mode[i],
                    texture,
                    scalars: stair::aux_scalars(mode[i]),
                    reversed: rev,
                }
            })
            .collect()
    }

    /// Edit-trace picking (`AFortPlayerController` edit tick `FUN_14084f8a0`): the trace runs from the point of the view
    /// ray closest to the pawn to `camera + dir * EditModeDistance` (2048); the first tile (or stair indicator) box hit
    /// along it is the target. Boxes are the tile mesh bounds at the tile transform (occlusion by other geometry is not
    /// modelled).
    pub fn pick(&self, world: &BuildingWorld, camera: Vec3, dir: Vec3, pawn_location: Vec3) -> Option<EditTarget> {
        let p = world.get(self.piece)?;
        let dir = dir.normalize_or_zero();
        let t0 = (pawn_location - camera).dot(dir);
        let start = camera + dir * t0;
        let end = camera + dir * edit_data::EDIT_MODE_TRACE_DISTANCE;
        let q = yaw_quat(p.yaw_deg);
        let mirror = if self.owner_mirrored { Vec3::new(-1., 1., 1.) } else { Vec3::ONE };
        let to_local = |w: Vec3| (q.inverse() * (w - p.location)) * mirror;
        let (ls, le) = (to_local(start), to_local(end));
        let mut best: Option<(f32, EditTarget)> = None;
        let l = &self.layout;
        for i in 0..self.preview.len() {
            // the tile component's current RelativeLocation (roof tiles move)
            let c = self.tile_local_position(i) + l.mesh_origin * l.scale;
            let e = l.mesh_extent * l.scale;
            if let Some(t) = segment_aabb(ls, le, c - e, c + e) {
                if best.map_or(true, |(bt, _)| t < bt) { best = Some((t, EditTarget::Tile(i))); }
            }
        }
        if self.kind == PieceKind::Stairs {
            // Indicators: relative yaw 90 * i, so odd indices swap the X/Y extents.
            let (mo, me) = self.aux_bounds;
            for i in 0..stair::AUX_PAIRS.len() {
                let q = yaw_quat(stair::AUX_YAW_STEP * i as f32);
                let c = Vec3::from(stair::AUX_LOCATIONS[i]) + q * mo;
                let e = if i % 2 == 1 { Vec3::new(me.y, me.x, me.z) } else { me };
                if let Some(t) = segment_aabb(ls, le, c - e, c + e) {
                    if best.map_or(true, |(bt, _)| t < bt) { best = Some((t, EditTarget::Aux(i))); }
                }
            }
        }
        best.map(|(_, t)| t)
    }

    /// World-space centre of the box [`pick`](Self::pick) tests for `t` (a tile or a stair indicator), for aiming the view at it
    /// (scripted input). None for an index the piece does not have.
    pub fn target_center(&self, world: &BuildingWorld, t: EditTarget) -> Option<Vec3> {
        let p = world.get(self.piece)?;
        if !self.valid_target(t) { return None; }
        let l = &self.layout;
        let local = match t {
            EditTarget::Tile(i) => self.tile_local_position(i) + l.mesh_origin * l.scale,
            EditTarget::Aux(i) => Vec3::from(stair::AUX_LOCATIONS[i]) + yaw_quat(stair::AUX_YAW_STEP * i as f32) * self.aux_bounds.0,
        };
        let mirror = if self.owner_mirrored { Vec3::new(-1., 1., 1.) } else { Vec3::ONE };
        Some(p.location + yaw_quat(p.yaw_deg) * (local * mirror))
    }
}

/// Slab test; returns the entry fraction along `a -> b`.
fn segment_aabb(a: Vec3, b: Vec3, mn: Vec3, mx: Vec3) -> Option<f32> {
    let d = b - a;
    let (mut t0, mut t1) = (0f32, 1f32);
    for k in 0..3 {
        if d[k].abs() < 1e-6 {
            if a[k] < mn[k] || a[k] > mx[k] { return None; }
        } else {
            let inv = 1. / d[k];
            let (mut ta, mut tb) = ((mn[k] - a[k]) * inv, (mx[k] - a[k]) * inv);
            if ta > tb { std::mem::swap(&mut ta, &mut tb); }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 { return None; }
        }
    }
    Some(t0)
}

/// `AFortPlayerController::ServerEditBuildingActor_Implementation` @140846a20 -> `ReplaceBuildingActor`
/// (`ABuildingSMActor::vfunc_329` @14048afb0): the piece takes the new class, `bMirrored` = the state's flag, and the
/// rotated transform. `PlacedPiece::location` / `yaw_deg` stay the true actor transform (already rotated);
/// `EditState::rotation_iterations` records the iterations of this last edit only.
/// The new actor is then initialised with InitReason 2 (`vfunc_329` -> `vfunc_247` @14047bed0 with the old actor): it rebuilds
/// over its BuildTime with the build animation through the core build-up path ([`BuildingWorld::rebuild_for_edit`]), health from
/// the old actor's health fraction. `build` = the new class's numbers (`None`: data missing, health and build state are kept).
pub fn apply_edit(world: &mut BuildingWorld, piece: PieceId, state: &EditModeState, build: Option<edit_data::ClassBuild>) -> bool {
    let Some(p) = world.get(piece) else { return false };
    let (loc, yaw) = rotated_transform(p.kind, p.location, p.yaw_deg, state.rotation_iterations);
    if let Some(b) = build {
        world.rebuild_for_edit(piece, crate::world::EditRebuild { max_health: b.max_health, initial_health_pct: b.initial_health_pct, build_time: b.build_time, animates: b.animates });
    }
    if let Some(pm) = world.get_mut(piece) {
        pm.location = loc;
        pm.yaw_deg = yaw;
    }
    world.set_edit(piece, EditState { class_path: Some(state.class_path.clone()), rotation_iterations: state.rotation_iterations, mirrored: state.mirrored })
}
