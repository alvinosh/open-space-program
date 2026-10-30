//! Parts drawn from their geometry (`osp_sim::geometry::draw`): each piece in its paint,
//! the back of its walls in the part's inside look, as meshes in the part's own render axes
//! with surface coordinates in metres, which the vessel shader takes its detail from.

use std::collections::HashMap;

use ab_glyph::FontArc;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use osp_sim::{
    component::{BuiltPart, Finish, LayerDef, Look},
    geometry::draw::DrawMesh,
};

use crate::{paint_layers, vessel_material::VesselMaterial};

/// Walls are drawn at least this thick, so that their open edges show, m.
const DRAWN_WALL: f64 = 0.008;

/// A sim (Z-up) vector in render (Y-up) axes, as `frame::to_render` maps them.
pub fn axes(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[2], -v[1])
}

/// `mesh` in render axes, moved by `offset` (render axes), and kept on the CPU too so that
/// rays can hit it (the parts the player right-clicks).
pub fn bevy_mesh(mesh: &DrawMesh, offset: Vec3) -> Mesh {
    let positions: Vec<[f32; 3]> = mesh
        .positions
        .iter()
        .map(|p| (axes(*p) - offset).to_array())
        .collect();
    let normals: Vec<[f32; 3]> = mesh.normals.iter().map(|n| axes(*n).to_array()).collect();
    let tangents: Vec<[f32; 4]> = mesh
        .tangents
        .iter()
        .map(|t| axes([t[0], t[1], t[2]]).extend(t[3]).to_array())
        .collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, mesh.uvs.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
        .with_inserted_indices(Indices::U32(mesh.indices.clone()))
}

/// A part's meshes, each with the look it is drawn in and the paint layers over it: the
/// front and back of each piece (the layers are on the fronts only).
pub fn part_meshes(part: &BuiltPart) -> Vec<(DrawMesh, Look, &[LayerDef])> {
    let thickness = |element: &str| {
        part.pieces
            .iter()
            .find(|p| p.element == element)
            .map_or(0.0, |p| p.thickness)
            .max(DRAWN_WALL)
    };
    let mut out = Vec::new();
    for piece in part.geometry.draw(thickness, part.member) {
        let (front, back) = part.paint.of(&piece.element);
        let layers = part.paint.layers.as_slice();
        for (mesh, look, layers) in [(piece.mesh, front, layers), (piece.back, back, &[][..])] {
            if !mesh.is_empty() {
                out.push((mesh, look, layers));
            }
        }
    }
    out
}

/// A material for each look and its layers in use, shared.
#[derive(Resource, Default)]
pub struct Looks {
    made: HashMap<(Finish, [u32; 3], String), Handle<VesselMaterial>>,
    /// The lettering's typeface, once read.
    font: Option<Option<FontArc>>,
}

/// What a material needs besides the look: where its decals go, and how to read a mod's
/// pictures.
pub struct Canvas<'a> {
    pub materials: &'a mut Assets<VesselMaterial>,
    pub images: &'a mut Assets<Image>,
    pub file: &'a dyn Fn(&str) -> Option<Vec<u8>>,
}

impl Looks {
    pub fn material(
        &mut self,
        look: Look,
        layers: &[LayerDef],
        canvas: &mut Canvas,
    ) -> Handle<VesselMaterial> {
        let color = look.color();
        let key = (
            look.finish,
            color.map(f32::to_bits),
            serde_json::to_string(layers).unwrap_or_default(),
        );
        if let Some(made) = self.made.get(&key) {
            return made.clone();
        }
        let font = self
            .font
            .get_or_insert_with(|| {
                let path = crate::content::asset_root().join("core/fonts/Barlow-SemiBold.ttf");
                let font = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| FontArc::try_from_vec(bytes).ok());
                if font.is_none() {
                    warn!("no lettering: cannot read {}", path.display());
                }
                font
            })
            .as_ref();
        let surface = paint_layers::surface(look.finish);
        let base = StandardMaterial {
            base_color: Color::linear_rgb(color[0], color[1], color[2]),
            metallic: surface.metallic,
            perceptual_roughness: surface.roughness,
            clearcoat: surface.clearcoat,
            clearcoat_perceptual_roughness: surface.clearcoat_roughness,
            reflectance: surface.reflectance,
            ..default()
        };
        let (packed, decals) = paint_layers::pack(layers, font, canvas.file);
        let decals = decals.map(|image| canvas.images.add(image));
        let handle = canvas.materials.add(VesselMaterial {
            base,
            extension: surface
                .kind
                .extension()
                .with_surface_uvs()
                .with_layers(packed, decals),
        });
        self.made.insert(key, handle.clone());
        handle
    }
}
