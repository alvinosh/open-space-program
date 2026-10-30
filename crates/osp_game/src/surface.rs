//! CPU-side sampling of a body's surface: land cover, colour and water, combining baked
//! data with launch-site paving. Shared by terrain chunk building and tree placement so
//! they agree on where forests and water are.

use std::sync::Arc;

use bevy::math::DVec3;
use osp_sim::{
    atmosphere::AtmosphereModel,
    body::Body,
    terrain::{Paving, Terrain},
};
use osp_terrain::{Cover, Dataset, Layer, TILE_CELLS, TileView, noise::cover_warp};

/// Everything needed to sample one body's surface off the main thread.
pub struct Surface {
    pub radius: f64,
    pub terrain: Arc<Terrain>,
    pub data: Option<Arc<Dataset>>,
    /// sRGB colour where there is no imagery.
    pub fallback_color: [u8; 3],
    /// Class where there is no land-cover data.
    pub fallback_cover: Cover,
    /// Earth-like air: scattering, and the night lights mean something.
    pub earthlike: bool,
}

impl Surface {
    pub fn new(body: &Body, data: Option<Arc<Dataset>>) -> Self {
        let srgb = |c: f32| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
        let earthlike = Self::is_earthlike(body);
        Self {
            radius: body.radius,
            terrain: body.terrain.clone(),
            data,
            fallback_color: body.color.map(srgb),
            fallback_cover: if earthlike {
                Cover::Grassland
            } else {
                Cover::Barren
            },
            earthlike,
        }
    }

    /// Bodies whose air scatters light like Earth's (the only model the sky shader has).
    pub fn is_earthlike(body: &Body) -> bool {
        body.atmosphere
            .as_ref()
            .is_some_and(|a| matches!(a.model, AtmosphereModel::Us1976))
    }

    /// A view of the cover layer for a chunk or tile key.
    pub fn cover_view(&self, key: osp_terrain::TileKey) -> Option<TileView> {
        self.data.as_ref()?.view(Layer::Cover, key)
    }

    /// Land-cover class and water fraction from a cover view at requested-grid sample
    /// coordinates. Where the data is coarser than requested, class boundaries are warped
    /// by noise so upsampled data reads as natural outlines instead of square cells.
    pub fn cover_from_view(&self, view: &TileView, i: f64, j: f64, dir: DVec3) -> (Cover, f32) {
        let (u, v) = view.to_source(i, j);
        let texel = view.source.edge_length(self.radius) / TILE_CELLS as f64;
        // Noise in units of data texels: one texel and a third of one. Always applied (not
        // only when upsampling) so boundaries do not shift as chunks change level.
        let (du, dv) = cover_warp(dir * (self.radius / texel));
        let class = view.byte_nearest_source(0, u + du, v + dv);
        let water = view.byte_linear(1, i, j) / 255.0;
        (Cover::from_u8(class), water)
    }

    /// Land-cover class and water fraction at a body-fixed direction, from data no
    /// deeper than `level`, with launch-site paving applied.
    pub fn cover_at(&self, dir: DVec3, level: u8) -> (Cover, f32) {
        let natural = self
            .data
            .as_ref()
            .and_then(|data| {
                let (view, u, v) = data.locate(Layer::Cover, dir, level)?;
                // `locate` returns source coordinates directly (identity mapping).
                Some(self.cover_from_view(&view, u, v, dir))
            })
            .unwrap_or((self.fallback_cover, 0.0));
        self.paved(dir, natural)
    }

    /// Apply launch-site paving over a natural class.
    pub fn paved(&self, dir: DVec3, (class, water): (Cover, f32)) -> (Cover, f32) {
        match self.terrain.paving(dir) {
            Some((kind, cover)) if cover >= 0.5 => (
                match kind {
                    Paving::Concrete => Cover::Concrete,
                    Paving::Asphalt => Cover::Asphalt,
                },
                0.0,
            ),
            Some((_, cover)) => (class, water * (1.0 - cover as f32)),
            None => (class, water),
        }
    }
}
