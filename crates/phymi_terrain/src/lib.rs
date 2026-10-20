//! Planet surface data for Phymi: cube-sphere addressing, land-cover classes and the baked
//! tile format (height, colour, cover, night lights) shared by the bake tool and the game.
//!
//! No Bevy and no simulation types: the game adapts a [`Dataset`] to the simulation's
//! height interface and to render textures.

pub mod cover;
pub mod cube;
pub mod dataset;
pub mod noise;
pub mod pack;

pub use cover::Cover;
pub use cube::{TILE_CELLS, TILE_SAMPLES, TileKey};
pub use dataset::{Dataset, Layer, Meta, TileView};
