//! Paint layers ([`LayerDef`]) for the vessel shader (`shaders/vessel.wesl`). Each layer is
//! four vectors in its material's uniform, laid out round the part's axis. Bands, stripes,
//! roll patterns and stringers are worked out per pixel, so their edges stay sharp however
//! close the camera is; lettering and pictures are rasterised into one mipmapped texture
//! per material, finely enough for a close look.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use osp_sim::component::{Finish, LayerDef};
use resvg::{tiny_skia, usvg};

use crate::vessel_material::{self, MAX_LAYERS, VesselLayers};

const BAND: f32 = 1.0;
const STRIPE: f32 = 2.0;
const CHECKER: f32 = 3.0;
const DECAL: f32 = 4.0;
const DECAL_VERTICAL: f32 = 5.0;
const STRINGERS: f32 = 6.0;

/// Texels to the height of a capital letter.
const CAP_TEXELS: f32 = 160.0;
/// Texels to the metre of a picture, at most this many across.
const PICTURE_DENSITY: f64 = 320.0;
const PICTURE_TEXELS: f64 = 2048.0;
/// Clear texels round each decal, so its edges filter to nothing.
const PAD: u32 = 6;

/// How light plays on a finish: what its standard material takes from it.
pub struct Surface {
    pub kind: vessel_material::Finish,
    pub metallic: f32,
    pub roughness: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub reflectance: f32,
}

pub fn surface(finish: Finish) -> Surface {
    let plain = |kind, roughness| Surface {
        kind,
        metallic: 0.0,
        roughness,
        clearcoat: 0.0,
        clearcoat_roughness: 0.5,
        reflectance: 0.5,
    };
    use vessel_material::Finish as Kind;
    match finish {
        Finish::Paint => Surface {
            clearcoat: 0.6,
            clearcoat_roughness: 0.25,
            ..plain(Kind::Paint, 0.42)
        },
        Finish::Composite => Surface {
            reflectance: 0.35,
            ..plain(Kind::Composite, 0.68)
        },
        Finish::Metal => Surface {
            metallic: 0.85,
            ..plain(Kind::Metal, 0.48)
        },
        Finish::Char => plain(Kind::Char, 0.9),
        Finish::Foam => Surface {
            reflectance: 0.3,
            ..plain(Kind::Foam, 0.88)
        },
        Finish::Cork => Surface {
            reflectance: 0.3,
            ..plain(Kind::Cork, 0.92)
        },
        Finish::Stainless => Surface {
            metallic: 0.92,
            ..plain(Kind::Stainless, 0.32)
        },
    }
}

/// A decal rasterised, waiting for its place in the texture: premultiplied linear RGBA.
struct Picture {
    width: u32,
    height: u32,
    texels: Vec<[f32; 4]>,
}

/// `layers` for the shader, with their lettering in `font` and pictures from the files
/// `file` reads (by a mod's path), and the texture those go in, if any.
pub fn pack(
    layers: &[LayerDef],
    font: Option<&FontArc>,
    file: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> (VesselLayers, Option<Image>) {
    let mut out = VesselLayers::default();
    let mut pictures: Vec<(usize, Picture)> = Vec::new();
    let mut count = 0;
    for layer in layers {
        if count == MAX_LAYERS {
            warn!("a part has more than {MAX_LAYERS} paint layers; the rest are left off");
            break;
        }
        let mut v = [Vec4::ZERO; 4];
        let ink = |finish: Option<Finish>, color: [f32; 3]| {
            let (code, metallic, roughness, coat) = match finish {
                Some(f) => {
                    let s = surface(f);
                    (s.kind.code(), s.metallic, s.roughness, s.clearcoat)
                }
                None => (-1.0, 0.0, 0.0, 0.0),
            };
            (
                Vec4::new(0.0, code, metallic, roughness),
                Vec4::new(color[0], color[1], color[2], coat),
            )
        };
        match layer {
            LayerDef::Band {
                along,
                color,
                finish,
            } => {
                (v[0], v[1]) = ink(*finish, *color);
                v[0].x = BAND;
                v[2] = Vec4::new(along[0] as f32, along[1] as f32, 0.0, 0.0);
            }
            LayerDef::Stripe {
                around,
                along,
                color,
                finish,
            } => {
                (v[0], v[1]) = ink(*finish, *color);
                v[0].x = STRIPE;
                let [low, high] = along.unwrap_or([-1e9, 1e9]);
                v[2] = Vec4::new(around[0] as f32, around[1] as f32, low as f32, high as f32);
            }
            LayerDef::Checker {
                around,
                along,
                first,
                color,
                finish,
            } => {
                (v[0], v[1]) = ink(*finish, *color);
                v[0].x = CHECKER;
                let rows: Vec<f32> = along.iter().take(6).map(|&z| z as f32).collect();
                let mut at = [0.0f32; 6];
                at[..rows.len()].copy_from_slice(&rows);
                let sectors = (*around).max(1) as f32;
                v[2] = Vec4::new(
                    if *first { -sectors } else { sectors },
                    rows.len() as f32,
                    at[0],
                    at[1],
                );
                v[3] = Vec4::new(at[2], at[3], at[4], at[5]);
            }
            LayerDef::Text {
                text,
                at,
                height,
                vertical,
                color,
            } => {
                let Some((picture, (width, tall))) = font.and_then(|f| lettering(f, text, *color))
                else {
                    continue;
                };
                let scale = *height as f32;
                v[0] = Vec4::new(decal_kind(*vertical), -1.0, 0.0, 0.0);
                v[2] = Vec4::new(at[0] as f32, at[1] as f32, width * scale, tall * scale);
                pictures.push((count, picture));
            }
            LayerDef::Image {
                file: path,
                at,
                size,
                vertical,
            } => {
                let Some(picture) = file(path).and_then(|svg| picture(&svg, size[0])) else {
                    warn!("paint layer picture `{path}` could not be read");
                    continue;
                };
                v[0] = Vec4::new(decal_kind(*vertical), -1.0, 0.0, 0.0);
                v[2] = Vec4::new(at[0] as f32, at[1] as f32, size[0] as f32, size[1] as f32);
                pictures.push((count, picture));
            }
            LayerDef::Stringers {
                along,
                count: n,
                depth,
            } => {
                v[0] = Vec4::new(STRINGERS, -1.0, 0.0, 0.0);
                v[2] = Vec4::new(along[0] as f32, along[1] as f32, *n as f32, *depth as f32);
            }
        }
        out.data[4 * count..4 * count + 4].copy_from_slice(&v);
        count += 1;
    }
    out.count.x = count as f32;
    let atlas = (!pictures.is_empty()).then(|| {
        let (image, rects) = atlas(pictures.iter().map(|(_, p)| p));
        for ((k, _), rect) in pictures.iter().zip(rects) {
            out.data[4 * k + 3] = rect;
        }
        image
    });
    (out, atlas)
}

fn decal_kind(vertical: bool) -> f32 {
    if vertical { DECAL_VERTICAL } else { DECAL }
}

/// `text` in `color`, and its size in capital heights: across, and from the capitals'
/// middle out to either edge and back (the box is centred on them).
fn lettering(font: &FontArc, text: &str, color: [f32; 3]) -> Option<(Picture, (f32, f32))> {
    let probe = font.outline_glyph(font.glyph_id('H').with_scale(100.0))?;
    let em = 100.0 * CAP_TEXELS / probe.px_bounds().height();
    let scaled = font.as_scaled(PxScale::from(em));
    let descent = -scaled.descent();
    let half = CAP_TEXELS / 2.0 + descent + PAD as f32;
    let height = (2.0 * half).ceil() as u32;
    // Baseline: the capitals' middle is the box's.
    let baseline = half + CAP_TEXELS / 2.0;
    let mut glyphs = Vec::new();
    let mut x = PAD as f32;
    let mut last = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = last {
            x += scaled.kern(prev, id);
        }
        glyphs.push(id.with_scale_and_position(em, point(x, baseline)));
        x += scaled.h_advance(id);
        last = Some(id);
    }
    let width = (x + PAD as f32).ceil() as u32;
    let mut texels = vec![[0.0f32; 4]; (width * height) as usize];
    for glyph in glyphs {
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, cover| {
            let (px, py) = (
                bounds.min.x as i32 + gx as i32,
                bounds.min.y as i32 + gy as i32,
            );
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                return;
            }
            let t = &mut texels[(py as u32 * width + px as u32) as usize];
            let a = (t[3] + cover).min(1.0);
            *t = [color[0] * a, color[1] * a, color[2] * a, a];
        });
    }
    let size = (width as f32 / CAP_TEXELS, height as f32 / CAP_TEXELS);
    Some((
        Picture {
            width,
            height,
            texels,
        },
        size,
    ))
}

/// An SVG drawn `width` m wide.
fn picture(svg: &[u8], width: f64) -> Option<Picture> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let across = (width * PICTURE_DENSITY).clamp(32.0, PICTURE_TEXELS) as f32;
    let scale = across / size.width();
    let (w, h) = (
        (size.width() * scale).round().max(1.0) as u32,
        (size.height() * scale).round().max(1.0) as u32,
    );
    let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale).post_translate(PAD as f32, PAD as f32),
        &mut pixmap.as_mut(),
    );
    // Premultiplied sRGB to premultiplied linear.
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let texels = pixmap
        .data()
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            if a <= 0.0 {
                return [0.0; 4];
            }
            let c = |k: usize| linear(p[k] as f32 / 255.0 / a) * a;
            [c(0), c(1), c(2), a]
        })
        .collect();
    Some(Picture {
        width: pw,
        height: ph,
        texels,
    })
}

/// The pictures one above another in a mipmapped texture, and where each is in it (offset
/// and size, in texture coordinates, inside its clear border).
fn atlas<'a>(pictures: impl Iterator<Item = &'a Picture> + Clone) -> (Image, Vec<Vec4>) {
    let width = pictures.clone().map(|p| p.width).max().unwrap_or(1);
    let height: u32 = pictures.clone().map(|p| p.height).sum::<u32>().max(1);
    let mut base = vec![[0.0f32; 4]; (width * height) as usize];
    let mut places = Vec::new();
    let mut y = 0;
    for p in pictures {
        for row in 0..p.height {
            let from = (row * p.width) as usize;
            let to = ((y + row) * width) as usize;
            base[to..to + p.width as usize]
                .copy_from_slice(&p.texels[from..from + p.width as usize]);
        }
        places.push(Vec4::new(
            (PAD as f32) / width as f32,
            (y + PAD) as f32 / height as f32,
            (p.width - 2 * PAD) as f32 / width as f32,
            (p.height - 2 * PAD) as f32 / height as f32,
        ));
        y += p.height;
    }
    // Box-filtered mip levels, premultiplied as they are.
    let mut levels = vec![(width, height, base)];
    while let Some((w, h, texels)) = levels.last()
        && (*w > 1 || *h > 1)
    {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![[0.0f32; 4]; (nw * nh) as usize];
        for ny in 0..nh {
            for nx in 0..nw {
                let mut sum = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((nx * 2 + dx).min(w - 1), (ny * 2 + dy).min(h - 1));
                    let t = texels[(sy * w + sx) as usize];
                    for c in 0..4 {
                        sum[c] += t[c] / 4.0;
                    }
                }
                next[(ny * nw + nx) as usize] = sum;
            }
        }
        levels.push((nw, nh, next));
    }
    let mut data = Vec::new();
    for (_, _, texels) in &levels {
        for t in texels {
            data.extend(t.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    let mut image = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels.len() as u32;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    (image, places)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> FontArc {
        let path = crate::content::asset_root().join("core/fonts/Barlow-SemiBold.ttf");
        FontArc::try_from_vec(std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn layers_pack_in_order_and_decals_get_their_place() {
        let layers = vec![
            LayerDef::Band {
                along: [1.0, 2.0],
                color: [0.0; 3],
                finish: Some(Finish::Composite),
            },
            LayerDef::Text {
                text: "UNITED STATES".into(),
                at: [0.25, 10.0],
                height: 0.8,
                vertical: true,
                color: [0.02; 3],
            },
            LayerDef::Checker {
                around: 4,
                along: vec![0.0, 3.0, 6.0],
                first: false,
                color: [0.0; 3],
                finish: None,
            },
        ];
        let (packed, image) = pack(&layers, Some(&font()), &|_| None);
        assert_eq!(packed.count.x, 3.0);
        assert_eq!(packed.data[0].x, BAND);
        assert_eq!(packed.data[0].y, vessel_material::Finish::Composite.code());
        assert_eq!(packed.data[4].x, DECAL_VERTICAL);
        // Thirteen letters of capitals 0.8 m tall: several metres long, a metre wide.
        let (long, wide) = (packed.data[6].z, packed.data[6].w);
        assert!(
            long > 6.0 && long < 12.0 && wide > 0.8 && wide < 1.6,
            "{long} {wide}"
        );
        let rect = packed.data[7];
        assert!(rect.z > 0.9 && rect.w > 0.9, "{rect}");
        assert_eq!(packed.data[8].x, CHECKER);
        assert_eq!(packed.data[10].y, 3.0);
        let image = image.unwrap();
        assert!(image.texture_descriptor.mip_level_count > 5);
    }
}
