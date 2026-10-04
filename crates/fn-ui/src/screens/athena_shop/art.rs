//! Offer display assets (`/Game/Catalog/DisplayAssets/DA_*`, class `FortMtxOfferData`): the tile / details art the 1.8 storefront offers point at
//! (`AthenaDirectAcquisitionOfferWidget::OfferSet`: `HasDisplayAsset` -> `GetTileImage` -> `MainImage.SetBrush`). The packages ship with the game; which
//! offer uses which display asset was the server's offer data (not in the paks), so the offline substitute maps them by name (`fn-app/src/lobby_shop.rs`).
use crate::screens::athena_lobby::store::{jvec, Store};

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayArt {
    /// asset name (`DA_Featured_GliderVoyager`)
    pub name: String,
    /// texture object path of `TileImage.ResourceObject`
    pub tile_image: Option<String>,
    pub details_image: Option<String>,
    /// `TileImage.ImageSize` (`MainImage.SetBrush(GetTileImage())` takes the whole brush, size included)
    pub tile_image_size: Option<[f32; 2]>,
    /// `Background` colour of the offer data (linear RGBA)
    pub background: Option<[f32; 4]>,
}

/// Reads `/Game/Catalog/DisplayAssets/<name>` (None when the package is absent or is not an offer data asset).
pub fn display_asset(store: &Store, name: &str) -> Option<DisplayArt> {
    let pkg = store.pkg(&format!("/Game/Catalog/DisplayAssets/{name}"))?;
    let e = pkg.exports.iter().find(|e| e.class == "FortMtxOfferData")?;
    let res = |k: &str| e.props.get(k).and_then(|b| b.get("ResourceObject")).and_then(|r| r.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(DisplayArt { name: name.to_owned(), tile_image: res("TileImage"), details_image: res("DetailsImage"), tile_image_size: e.props.get("TileImage").and_then(|b| jvec::<2>(b.get("ImageSize"))), background: jvec::<4>(e.props.get("Background")) })
}

/// Every offer display asset package under `/Game/Catalog/DisplayAssets` (names only).
pub fn list_display_assets(store: &Store) -> Vec<String> {
    let mut v: Vec<String> = store.pak_set().glob("fortnitegame/content/catalog/displayassets/*.uasset").iter()
        .filter_map(|(_, e)| fn_pak::pak_path_to_game(e.rel_path()).or_else(|| fn_pak::pak_path_to_game(&e.path)))
        .filter_map(|g| g.trim_end_matches(".uasset").rsplit('/').next().map(str::to_owned)).collect();
    v.sort();
    v.dedup();
    v
}
