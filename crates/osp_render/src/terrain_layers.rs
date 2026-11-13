//! Terrain chunks' data textures as layers of shared array textures, and the rest of their data
//! in one buffer, so a body's chunks share one material and are drawn together, not in a draw
//! batch each (`planet`, `terrain_material`, `shaders/terrain.wesl`).
//!
//! The textures come in pages of [`PAGE_LAYERS`] layers: colour, surface, land cover and night
//! lights, an array texture each. A chunk takes the same layer in all four textures of a page,
//! whose material (one per body and page) binds them, and gives it back when it goes. Its texels
//! are written straight into its layer on the render thread: changing the page's `Image` would
//! upload the whole array again. The rest of a chunk's data ([`ChunkData`]) is in one storage
//! buffer for every page, at the chunk's slot (its page's first slot plus its layer), which is
//! also its mesh's tag, so the shader finds it there.

use std::sync::{
    Mutex,
    mpsc::{Receiver, Sender, channel},
};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::MeshTag,
    platform::collections::HashMap,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{
            Extent3d, Origin3d, TexelCopyBufferLayout, TextureFormat, TextureUsages,
            TextureViewDescriptor, TextureViewDimension,
        },
        renderer::RenderQueue,
        storage::ShaderBuffer,
        texture::GpuImage,
    },
};
use bytemuck::{Pod, Zeroable};
use osp_sim::body::BodyId;
use osp_terrain::{TILE_CELLS, TILE_SAMPLES};

use crate::{planet::LIGHTS_DEEPER, terrain_material::TerrainMaterial};

/// Chunks in a page. More pages are added as they fill, each with its own material per body (and
/// so its own draw batch); emptied pages are kept for the next chunks.
pub const PAGE_LAYERS: u32 = 128;
/// Night lights' side, texels: finer than the rest (see `planet::lights_texels`).
pub const LIGHTS_SIDE: usize = TILE_CELLS << LIGHTS_DEEPER;

pub struct TerrainLayersPlugin;

impl Plugin for TerrainLayersPlugin {
    fn build(&self, app: &mut App) {
        let (sender, receiver) = channel();
        let chunks = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::new(
                vec![GpuChunk::zeroed()],
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            ));
        app.insert_resource(ChunkLayers {
            pages: Vec::new(),
            chunks,
            uploads: sender,
        })
        .add_observer(give_back);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(Uploads {
                    from_game: Mutex::new(receiver),
                    waiting: Vec::new(),
                })
                .add_systems(Render, write_layers.in_set(RenderSystems::PrepareResources));
        }
    }
}

/// A chunk's data, as the shader reads it (`Chunk` in `shaders/terrain.wesl`), at its slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkData {
    /// xyz: planet centre in chunk-local coordinates; w: mean radius.
    pub planet: Vec4,
    /// xyz: chunk centre in body-fixed axes modulo `planet::DETAIL_PERIOD`.
    pub detail_origin: Vec4,
    /// x: chunk level, y: colour data level, z: cover data level, w: 1 if the body has air.
    pub levels: Vec4,
    /// Its layer in its page's data textures.
    pub layer: u32,
    /// Whether it has night lights (the dark ones' layer of the lights is left as it was).
    pub lights: bool,
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct GpuChunk {
    planet: [f32; 4],
    detail_origin: [f32; 4],
    levels: [f32; 4],
    /// x: layer, y: 1 with night lights.
    layer: [u32; 4],
}

impl From<ChunkData> for GpuChunk {
    fn from(chunk: ChunkData) -> Self {
        Self {
            planet: chunk.planet.to_array(),
            detail_origin: chunk.detail_origin.to_array(),
            levels: chunk.levels.to_array(),
            layer: [chunk.layer, u32::from(chunk.lights), 0, 0],
        }
    }
}

/// A chunk's texels, as built (`planet::build_chunk`).
pub struct ChunkTexels {
    /// `TILE_SAMPLES`² sRGB colours, RGBA.
    pub color: Vec<u8>,
    /// `TILE_SAMPLES`² of four f16: normal east and north, elevation, water.
    pub surface: Vec<u8>,
    /// `TILE_SAMPLES`² land-cover classes.
    pub cover: Vec<u8>,
    /// [`LIGHTS_SIDE`]² with every mip level after it, or `None` where it is dark.
    pub lights: Option<Vec<u8>>,
}

/// The page and layer a chunk's textures are in; given back when the chunk goes.
#[derive(Component, Clone, Copy, Debug)]
pub struct ChunkLayer {
    pub page: usize,
    pub layer: u32,
}

impl ChunkLayer {
    /// Where its data is in the chunk buffer, and its mesh's tag.
    pub fn slot(self) -> u32 {
        self.page as u32 * PAGE_LAYERS + self.layer
    }

    pub fn tag(self) -> MeshTag {
        MeshTag::new(self.slot())
    }
}

/// A page's array textures: colour, surface, cover and lights.
#[derive(Clone)]
pub struct PageTextures {
    pub color: Handle<Image>,
    pub surface: Handle<Image>,
    pub cover: Handle<Image>,
    pub lights: Handle<Image>,
}

#[derive(Resource)]
pub struct ChunkLayers {
    pages: Vec<Page>,
    /// Every slot's `GpuChunk`.
    pub chunks: Handle<ShaderBuffer>,
    uploads: Sender<Upload>,
}

struct Page {
    textures: PageTextures,
    /// Free layers, the lowest last.
    free: Vec<u32>,
    materials: HashMap<BodyId, Handle<TerrainMaterial>>,
}

impl ChunkLayers {
    /// A free layer, in a new page if every page is full.
    pub fn take(
        &mut self,
        images: &mut Assets<Image>,
        buffers: &mut Assets<ShaderBuffer>,
    ) -> ChunkLayer {
        let page = match self.pages.iter().position(|p| !p.free.is_empty()) {
            Some(page) => page,
            None => {
                if let Some(mut chunks) = buffers.get_mut(self.chunks.id()) {
                    let slots = (self.pages.len() + 1) * PAGE_LAYERS as usize;
                    let have = chunks.cast_slice::<GpuChunk>().map_or(0, <[GpuChunk]>::len);
                    chunks.extend((have..slots).map(|_| GpuChunk::zeroed()));
                }
                self.pages.push(Page {
                    textures: PageTextures {
                        color: images.add(page_image(Kind::Color)),
                        surface: images.add(page_image(Kind::Surface)),
                        cover: images.add(page_image(Kind::Cover)),
                        lights: images.add(page_image(Kind::Lights)),
                    },
                    free: (0..PAGE_LAYERS).rev().collect(),
                    materials: HashMap::default(),
                });
                self.pages.len() - 1
            }
        };
        let layer = self.pages[page]
            .free
            .pop()
            .expect("a page with a free layer");
        ChunkLayer { page, layer }
    }

    /// Put `data` in the chunk's slot, and `texels` in its layer (on the render thread, before it
    /// is drawn).
    pub fn write(
        &self,
        chunk: ChunkLayer,
        data: ChunkData,
        texels: ChunkTexels,
        buffers: &mut Assets<ShaderBuffer>,
    ) {
        if let Some(mut chunks) = buffers.get_mut(self.chunks.id())
            && let Some(slot) = chunks
                .cast_slice_mut::<GpuChunk>()
                .and_then(|slots| slots.get_mut(chunk.slot() as usize))
        {
            *slot = data.into();
        }
        let textures = &self.pages[chunk.page].textures;
        let upload = Upload {
            images: [
                textures.color.id(),
                textures.surface.id(),
                textures.cover.id(),
                textures.lights.id(),
            ],
            layer: chunk.layer,
            texels,
        };
        // The render world only goes when the app does.
        let _ = self.uploads.send(upload);
    }

    /// The material for `body`'s chunks in `page`, made by `make` from the page's textures the
    /// first time.
    pub fn material(
        &mut self,
        page: usize,
        body: BodyId,
        materials: &mut Assets<TerrainMaterial>,
        make: impl FnOnce(PageTextures) -> TerrainMaterial,
    ) -> Handle<TerrainMaterial> {
        let page = &mut self.pages[page];
        page.materials
            .entry(body)
            .or_insert_with(|| materials.add(make(page.textures.clone())))
            .clone()
    }
}

fn give_back(
    remove: On<Remove<ChunkLayer>>,
    chunks: Query<&ChunkLayer>,
    mut layers: ResMut<ChunkLayers>,
) {
    if let Ok(chunk) = chunks.get(remove.entity)
        && let Some(page) = layers.pages.get_mut(chunk.page)
    {
        page.free.push(chunk.layer);
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Color,
    Surface,
    Cover,
    Lights,
}

impl Kind {
    const ALL: [Kind; 4] = [Kind::Color, Kind::Surface, Kind::Cover, Kind::Lights];

    fn format(self) -> TextureFormat {
        match self {
            Kind::Color => TextureFormat::Rgba8UnormSrgb,
            Kind::Surface => TextureFormat::Rgba16Float,
            Kind::Cover => TextureFormat::R8Uint,
            Kind::Lights => TextureFormat::R8Unorm,
        }
    }

    fn bytes_per_texel(self) -> u32 {
        match self {
            Kind::Color => 4,
            Kind::Surface => 8,
            Kind::Cover | Kind::Lights => 1,
        }
    }

    fn side(self) -> u32 {
        match self {
            Kind::Lights => LIGHTS_SIDE as u32,
            _ => TILE_SAMPLES as u32,
        }
    }

    /// Mip levels: the lights are filtered down with distance, as they are all that shows at
    /// night and would sparkle.
    fn mip_levels(self) -> u32 {
        match self {
            Kind::Lights => LIGHTS_SIDE.ilog2() + 1,
            _ => 1,
        }
    }

    fn sampler(self) -> ImageSampler {
        let clamp = ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            ..default()
        };
        ImageSampler::Descriptor(match self {
            Kind::Color | Kind::Surface => ImageSamplerDescriptor {
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                ..clamp
            },
            Kind::Cover => clamp,
            Kind::Lights => ImageSamplerDescriptor {
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 16,
                ..clamp
            },
        })
    }
}

/// An array texture of `PAGE_LAYERS` layers, empty: the layers are written as chunks take them.
fn page_image(kind: Kind) -> Image {
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: kind.side(),
        height: kind.side(),
        depth_or_array_layers: PAGE_LAYERS,
    };
    image.texture_descriptor.format = kind.format();
    image.texture_descriptor.mip_level_count = kind.mip_levels();
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.data = None;
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = kind.sampler();
    image
}

/// A chunk's texels on their way to its layer.
struct Upload {
    /// Colour, surface, cover, lights.
    images: [AssetId<Image>; 4],
    layer: u32,
    texels: ChunkTexels,
}

#[derive(Resource)]
struct Uploads {
    from_game: Mutex<Receiver<Upload>>,
    /// Those whose page isn't on the GPU yet.
    waiting: Vec<Upload>,
}

fn write_layers(
    mut uploads: ResMut<Uploads>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    let uploads = &mut *uploads;
    let from_game = uploads.from_game.get_mut().unwrap();
    uploads.waiting.extend(from_game.try_iter());
    uploads.waiting.retain(|upload| {
        let Some(textures) = upload
            .images
            .iter()
            .map(|id| images.get(*id))
            .collect::<Option<Vec<_>>>()
        else {
            return true;
        };
        let texels = &upload.texels;
        let data = [
            Some(&texels.color),
            Some(&texels.surface),
            Some(&texels.cover),
            texels.lights.as_ref(),
        ];
        for ((kind, texture), data) in Kind::ALL.into_iter().zip(textures).zip(data) {
            let Some(data) = data else { continue };
            let mut offset = 0;
            for level in 0..kind.mip_levels() {
                let side = (kind.side() >> level).max(1);
                let len = (side * side * kind.bytes_per_texel()) as usize;
                let mut copy = texture.texture.as_image_copy();
                copy.mip_level = level;
                copy.origin = Origin3d {
                    x: 0,
                    y: 0,
                    z: upload.layer,
                };
                queue.write_texture(
                    copy,
                    &data[offset..offset + len],
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * kind.bytes_per_texel()),
                        rows_per_image: Some(side),
                    },
                    Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
                offset += len;
            }
        }
        false
    });
}
