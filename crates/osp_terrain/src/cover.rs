//! Land-cover classes: one compact scheme that global (ESA CCI) and regional (ESA
//! WorldCover) maps are both translated into. The renderer picks surface materials and tree
//! types from it.

/// Surface class stored in the cover layer. The numeric values are part of the baked data
/// format and of the terrain shader; append, never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cover {
    Ocean = 0,
    Lake = 1,
    Ice = 2,
    Barren = 3,
    Sand = 4,
    Tundra = 5,
    Grassland = 6,
    Shrubland = 7,
    Savanna = 8,
    Cropland = 9,
    Wetland = 10,
    Mangrove = 11,
    NeedleleafForest = 12,
    TropicalForest = 13,
    DeciduousForest = 14,
    MixedForest = 15,
    Urban = 16,
    /// Concrete laid by launch sites (never baked; added at runtime).
    Concrete = 17,
    /// Asphalt laid by launch sites (never baked; added at runtime).
    Asphalt = 18,
}

impl Cover {
    pub const COUNT: usize = 19;

    pub fn from_u8(v: u8) -> Cover {
        use Cover::*;
        match v {
            0 => Ocean,
            1 => Lake,
            2 => Ice,
            3 => Barren,
            4 => Sand,
            5 => Tundra,
            6 => Grassland,
            7 => Shrubland,
            8 => Savanna,
            9 => Cropland,
            10 => Wetland,
            11 => Mangrove,
            12 => NeedleleafForest,
            13 => TropicalForest,
            14 => DeciduousForest,
            15 => MixedForest,
            16 => Urban,
            17 => Concrete,
            18 => Asphalt,
            _ => Grassland,
        }
    }

    pub fn is_water(self) -> bool {
        matches!(self, Cover::Ocean | Cover::Lake)
    }

    pub fn is_forest(self) -> bool {
        matches!(
            self,
            Cover::NeedleleafForest
                | Cover::TropicalForest
                | Cover::DeciduousForest
                | Cover::MixedForest
        )
    }

    /// ESA CCI Land Cover (UN-LCCS) code. Water is reported as [`Cover::Lake`]; the bake
    /// decides which water is ocean from the elevation model. No-data (0) is open ocean.
    pub fn from_cci(code: u8) -> Cover {
        use Cover::*;
        match code {
            0 => Ocean,
            10..=30 => Cropland,
            40 => Shrubland,
            50 => TropicalForest,
            60..=62 => DeciduousForest,
            70..=82 => NeedleleafForest,
            90 => MixedForest,
            100 => Savanna,
            110 => Grassland,
            120..=122 => Shrubland,
            130 => Grassland,
            140 => Tundra,
            150..=153 => Barren,
            160 | 180 => Wetland,
            170 => Mangrove,
            190 => Urban,
            200..=202 => Barren,
            210 => Lake,
            220 => Ice,
            _ => Grassland,
        }
    }

    /// ESA WorldCover code, refined with what the global map says at the same place
    /// (WorldCover only says "tree cover", not which kind), the elevation (water at sea
    /// level is ocean or lagoon) and whether open water is close (bare ground by the water
    /// is beach; elsewhere it is sparse scrub and soil).
    pub fn from_worldcover(
        code: u8,
        global: Cover,
        height: f64,
        near_water: bool,
        latitude_deg: f64,
    ) -> Option<Cover> {
        use Cover::*;
        Some(match code {
            10 => {
                if global.is_forest() {
                    global
                } else {
                    forest_for_latitude(latitude_deg)
                }
            }
            20 => Shrubland,
            30 => Grassland,
            40 => Cropland,
            50 => Urban,
            60 => {
                if near_water && height < 8.0 {
                    Sand
                } else {
                    Barren
                }
            }
            70 => Ice,
            80 => {
                if height < 1.5 {
                    Ocean
                } else {
                    Lake
                }
            }
            90 => Wetland,
            95 => Mangrove,
            100 => Tundra,
            _ => return None,
        })
    }
}

/// Plausible forest type where the global map does not name one.
pub fn forest_for_latitude(latitude_deg: f64) -> Cover {
    match latitude_deg.abs() {
        l if l < 24.0 => Cover::TropicalForest,
        l if l < 42.0 => Cover::MixedForest,
        l if l < 55.0 => Cover::DeciduousForest,
        _ => Cover::NeedleleafForest,
    }
}
