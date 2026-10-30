//! `.svg` images: the asset server loads them as ordinary [`Image`]s, so `image` nodes and
//! window icons can name an SVG as they would a PNG.
//!
//! The drawing is rasterised once, well above the size it is shown at (at least
//! [`MIN_TEXELS`] on its long side and [`SUPERSAMPLE`] texels per SVG unit), with a full mip
//! chain and trilinear filtering, so it stays crisp and free of shimmer at any size and
//! UI scale. Draw in white where the theme should tint (`color`), in colour where it
//! should not. Text in SVGs is not rendered: convert it to paths.

use bevy::{
    asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader},
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use resvg::{tiny_skia, usvg};

/// Texels per SVG unit at the base level. Units are logical pixels at 1x.
const SUPERSAMPLE: f32 = 4.0;
/// Smallest long side of the base level, texels (small icons).
const MIN_TEXELS: f32 = 256.0;
/// Largest side of the base level, texels.
const MAX_TEXELS: f32 = 4096.0;

#[derive(Default, TypePath)]
pub struct SvgLoader;

#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error("could not read SVG: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid SVG: {0}")]
    Parse(#[from] usvg::Error),
    #[error("SVG has no area")]
    Empty,
}

impl AssetLoader for SvgLoader {
    type Asset = Image;
    type Settings = ();
    type Error = SvgError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Image, SvgError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        rasterize(&bytes)
    }

    fn extensions(&self) -> &[&str] {
        &["svg"]
    }
}

/// Rasterise an SVG document into a mipmapped sRGB image (straight alpha).
pub fn rasterize(svg: &[u8]) -> Result<Image, SvgError> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())?;
    let size = tree.size();
    let long = size.width().max(size.height());
    let scale = (long * SUPERSAMPLE).clamp(MIN_TEXELS, MAX_TEXELS) / long;
    let (width, height) = (
        (size.width() * scale).round().max(1.0) as u32,
        (size.height() * scale).round().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(SvgError::Empty)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Box-filter the mip chain in premultiplied alpha, then unpremultiply every level.
    let mut levels = vec![(width, height, pixmap.data().to_vec())];
    while let Some((w, h, data)) = levels.last()
        && (*w > 1 || *h > 1)
    {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u32::from(data[i + c]);
                    }
                }
                let o = ((y * nw + x) * 4) as usize;
                for c in 0..4 {
                    next[o + c] = ((sum[c] + 2) / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
    }
    let mip_count = levels.len() as u32;
    let mut data = Vec::new();
    for (_, _, mut level) in levels {
        for px in level.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            if a > 0 && a < 255 {
                for c in &mut px[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        data.extend_from_slice(&level);
    }

    let mut image = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mip_count;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..default()
    });
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_icons_are_supersampled_with_mips() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
            <circle cx="12" cy="12" r="8" fill="#fff"/></svg>"##;
        let image = rasterize(svg).unwrap();
        let (w, h) = (image.width(), image.height());
        assert_eq!((w, h), (256, 256));
        assert_eq!(image.texture_descriptor.mip_level_count, 9);
        let data = image.data.as_ref().unwrap();
        // Every level is there: 256² + 128² + ... + 1².
        let texels: u32 = (0..9).map(|l| (256u32 >> l).pow(2)).sum();
        assert_eq!(data.len(), texels as usize * 4);
        // The middle of the circle is opaque white, the corner clear.
        let at = |x: u32, y: u32| &data[((y * w + x) * 4) as usize..][..4];
        assert_eq!(at(128, 128), &[255, 255, 255, 255]);
        assert_eq!(at(0, 0)[3], 0);
    }

    #[test]
    fn large_drawings_keep_their_aspect() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 100" width="300" height="100">
            <rect width="300" height="100" fill="#123"/></svg>"##;
        let image = rasterize(svg).unwrap();
        assert_eq!((image.width(), image.height()), (1200, 400));
    }
}
