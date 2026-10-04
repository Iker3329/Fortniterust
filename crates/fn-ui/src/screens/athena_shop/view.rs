//! Plain data the Item Shop screen is driven by. In the 1.8 client this is what the native `FortMcpContext` / storefront objects hand the Blueprint widgets
//! (`GetOfferInfo()` -> `GetName`, `GetShortDescription`, `GetDescription`, `GetPriceItem`, `GetNormalPrice`, `GetTileImage`, `GetFirstGrantItemDefinition`,
//! `GetDetailsAttributes`, `IsOwned`, `CanPurchaseOffer`). The widgets only read them; where they come from (the Epic storefront online, the offline substitute
//! in `fn-app/src/lobby_shop.rs` offline) is the caller's business.

/// One offer tile (`AthenaDirectAcquisitionOfferWidget`) / the offer shown by the details and purchased widgets.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferView {
    /// stable offer id (`GetOfferId`)
    pub offer_id: String,
    /// `GetName` (the widgets upper-case it with `TextToUpper`)
    pub name: String,
    /// `GetShortDescription` (the item's category line, e.g. "Outfit")
    pub short_description: String,
    /// `GetDescription`
    pub description: String,
    /// `EFortRarity` as the index into `/Game/Balance/RarityData.RarityCollection` (0 Common, 1 Uncommon, 2 Rare, 3 Epic, 4 Legendary, ...)
    pub rarity: usize,
    /// V-Bucks price (`GetNormalPrice(0)`); the offline substitute never has a sale price
    pub price: i64,
    /// `UFortMcpContext::IsOwned(offerId)`
    pub owned: bool,
    /// texture object path of the tile art: the offer display asset's `TileImage` when it has one (`HasDisplayAsset`), else the first grant item's
    /// `LargePreviewImage` (`LargeImageLoaded` -> `MainImage.SetBrushFromTexture`)
    pub tile_image: Option<String>,
    /// texture object path of the item icon (`GetFirstGrantItemDefinition().GetLargePreviewImage()`), drawn by the details / purchased `ItemCard`
    pub icon: Option<String>,
    /// `GetDetailsAttributes` rows (Name, Value) shown by `StoreMain_OfferDetailsAttribute`
    pub attributes: Vec<(String, String)>,
    /// `AthenaGliderItemDefinition` cast of `GetFirstGrantItemDefinition` (the details widget shows `NotUmbrellaIcon` for gliders)
    pub is_glider: bool,
    /// the granted item is an `AthenaCharacterItemDefinition` (the native item card draws it with `bIsHero = 1`)
    pub is_outfit: bool,
}

/// The two storefronts of `AthenaDirectAcquisitionScreen.StorefrontNames`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopView {
    /// `BRWeeklyStorefront`: large tiles (`LargeTileHB`, header "WEEKLY ITEMS")
    pub weekly: Vec<OfferView>,
    /// `BRDailyStorefront`: small tiles (`SmallTileGP`, header "DAILY ITEMS")
    pub daily: Vec<OfferView>,
    /// seconds until `GetWeeklyStoreEndDate` / `GetDailyStoreEndDate`
    pub week_ends_in: i64,
    pub day_ends_in: i64,
}

impl ShopView {
    pub fn find(&self, offer_id: &str) -> Option<&OfferView> { self.weekly.iter().chain(self.daily.iter()).find(|o| o.offer_id == offer_id) }
}

/// `CanPurchaseOffer(offerId, priceIndex)`: 0 = can purchase (button "PURCHASE"), 2 = "Cannot Afford" (button disabled); owned offers show `OwnedSwitcher` index 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affordability { Can, CannotAfford, Owned }

impl Affordability {
    pub fn of(offer: &OfferView, balance: i64) -> Affordability {
        if offer.owned { Affordability::Owned } else if balance >= offer.price { Affordability::Can } else { Affordability::CannotAfford }
    }
}

/// The full-screen widgets `AthenaDirectAcquisitionOfferWidget::OnClicked` / `HandlePurchaseOfferComplete` push on the UI manager stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    None,
    /// `AthenaDirectAcquisitionOfferDetailsWidget` (offer id into the [`ShopView`])
    Details { offer: OfferView, purchase_hovered: bool },
    /// `AthenaDirectAcquisitionOfferPurchasedWidget`
    Purchased { offer: OfferView },
}

/// Runtime-driven parts of the screen (everything else is the widget data).
#[derive(Clone, Debug)]
pub struct ShopState {
    /// index into the visible tab registrations (3 = "Item Shop" with the 1.8 registrations: Lobby, Locker, Item Shop, ...)
    pub selected_tab: usize,
    pub hovered_tab: Option<usize>,
    /// `TopBar.MTXButton.Amount` (the offline profile's balance)
    pub mtx_balance: i64,
    pub view: ShopView,
    pub hovered_offer: Option<String>,
    pub dialog: Dialog,
    /// pointer over the bottom bar "Back" button
    pub back_hovered: bool,
    /// animation clock for the tiles' `Hover` animation / rarity border (seconds); 0 = settled frame
    pub time: f32,
}

impl Default for ShopState {
    fn default() -> Self { ShopState { selected_tab: 0, hovered_tab: None, mtx_balance: 0, view: ShopView::default(), hovered_offer: None, dialog: Dialog::None, back_hovered: false, time: 0.0 } }
}
