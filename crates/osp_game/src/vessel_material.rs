//! The material for vessel surfaces: standard PBR plus procedural detail for the finish a
//! surface stands for (`shaders/vessel.wesl`). Parts drawn from their geometry get the
//! finish their paint asks for (see [`crate::part_mesh`]); the meshes of part models
//! (engines, pods, RCS blocks) get theirs from their glTF material names, which say what
//! they are made of (`K1_RegenNozzle_OxidisedBronze`).

use bevy::{
    asset::embedded_asset,
    gltf::GltfMaterialName,
    pbr::{ExtendedMaterial, MaterialExtension},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

pub type VesselMaterial = ExtendedMaterial<StandardMaterial, VesselFinish>;

pub struct VesselMaterialPlugin;

impl Plugin for VesselMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/vessel.wesl");
        app.add_plugins(MaterialPlugin::<VesselMaterial>::default())
            .add_systems(Update, finish_models);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finish {
    Paint,
    Composite,
    Metal,
    /// Metal under an oxide film from heating, between these thicknesses, nm.
    HeatTint(f32, f32),
    /// Charred ablator or soot.
    Char,
    Foam,
    Cork,
    /// Bare stainless steel rings with their weld seams.
    Stainless,
}

/// Steel and Inconel nozzles: straw to blue.
const STEEL_FILM: Finish = Finish::HeatTint(15.0, 90.0);
/// Niobium (columbium) nozzle extensions: gold, purple and blue.
const NIOBIUM_FILM: Finish = Finish::HeatTint(40.0, 130.0);
/// Oxidised copper alloys: bronze and brown.
const BRONZE_FILM: Finish = Finish::HeatTint(20.0, 60.0);

impl Finish {
    pub fn extension(self) -> VesselFinish {
        let (kind, film) = match self {
            Finish::Paint => (0.0, (0.0, 0.0)),
            Finish::Composite => (1.0, (0.0, 0.0)),
            Finish::Metal => (2.0, (0.0, 0.0)),
            Finish::HeatTint(thin, thick) => (3.0, (thin, thick)),
            Finish::Char => (4.0, (0.0, 0.0)),
            Finish::Foam => (5.0, (0.0, 0.0)),
            Finish::Cork => (6.0, (0.0, 0.0)),
            Finish::Stainless => (7.0, (0.0, 0.0)),
        };
        VesselFinish {
            params: Vec4::new(kind, film.0, film.1, 0.0),
            layers: VesselLayers::default(),
            decals: None,
        }
    }

    /// The code the shader knows the finish by.
    pub fn code(self) -> f32 {
        self.extension().params.x
    }

    /// The finish a model's material name asks for, or `None` to leave it as it is
    /// (glass).
    pub fn from_material_name(name: &str) -> Option<Finish> {
        let name = name.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
        if has(&["glass"]) {
            return None;
        }
        Some(if has(&["ablative", "charred", "sooted", "soot"]) {
            Finish::Char
        } else if has(&["niobium", "columbium"]) {
            NIOBIUM_FILM
        } else if has(&["oxidised", "oxidized"]) {
            BRONZE_FILM
        } else if has(&["heattinted", "heat_tinted"]) {
            STEEL_FILM
        } else if has(&["composite", "carbon"]) {
            Finish::Composite
        } else if has(&[
            "steel", "silver", "alumin", "inconel", "bronze", "gunmetal", "titanium", "copper",
            "chrome",
        ]) {
            Finish::Metal
        } else {
            Finish::Paint
        })
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct VesselFinish {
    /// x: finish, y and z: oxide film thickness range for heat tint, nm; w: 1 where the
    /// mesh's surface coordinates are metres along it, with tangents.
    #[uniform(100)]
    params: Vec4,
    /// Paint layers over the surface (see [`crate::paint_layers`]).
    #[uniform(101)]
    #[reflect(ignore)]
    layers: VesselLayers,
    /// The lettering and pictures among the layers, premultiplied linear colour.
    #[texture(102)]
    #[sampler(103)]
    decals: Option<Handle<Image>>,
}

/// Most paint layers one material holds.
pub const MAX_LAYERS: usize = 16;

/// Paint layers as the shader reads them: four vectors each (see
/// [`crate::paint_layers::pack`]).
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct VesselLayers {
    /// x: how many.
    pub count: Vec4,
    pub data: [Vec4; 4 * MAX_LAYERS],
}

impl Default for VesselLayers {
    fn default() -> Self {
        Self {
            count: Vec4::ZERO,
            data: [Vec4::ZERO; 4 * MAX_LAYERS],
        }
    }
}

impl VesselFinish {
    /// With paint `layers` over it, its lettering and pictures in `decals`.
    pub fn with_layers(mut self, layers: VesselLayers, decals: Option<Handle<Image>>) -> Self {
        self.layers = layers;
        self.decals = decals;
        self
    }

    /// For meshes drawn from part geometry, whose surface coordinates are metres along the
    /// surface: the detail follows them instead of being projected round the part's axis.
    pub fn with_surface_uvs(mut self) -> Self {
        self.params.w = 1.0;
        self
    }
}

impl MaterialExtension for VesselFinish {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/vessel.wesl".into()
    }
}

/// The root of a part model's scene: its meshes get vessel materials once spawned.
#[derive(Component)]
pub struct PartModel;

/// Swap the glTF materials of newly spawned part models for vessel materials with the
/// finish their names ask for, sharing one per source material.
fn finish_models(
    mut commands: Commands,
    meshes: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, &GltfMaterialName),
        Added<GltfMaterialName>,
    >,
    parents: Query<&ChildOf>,
    models: Query<(), With<PartModel>>,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<VesselMaterial>>,
    mut finished: Local<HashMap<AssetId<StandardMaterial>, Handle<VesselMaterial>>>,
) {
    for (entity, material, name) in &meshes {
        if !parents.iter_ancestors(entity).any(|e| models.contains(e)) {
            continue;
        }
        let Some(finish) = Finish::from_material_name(name) else {
            continue;
        };
        let Some(base) = standard.get(&material.0) else {
            continue;
        };
        let handle = finished
            .entry(material.id())
            .or_insert_with(|| {
                materials.add(VesselMaterial {
                    base: base.clone(),
                    extension: finish.extension(),
                })
            })
            .clone();
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_material_names_pick_their_finish() {
        let cases = [
            ("C1_Window_Glass", None),
            ("C4_HeatShield_Ablative", Some(Finish::Char)),
            ("A10_AblativeLiner_Charred", Some(Finish::Char)),
            ("K1_RegenNozzle_SootedInner", Some(Finish::Char)),
            ("K1V_NiobiumExtension_Outer", Some(NIOBIUM_FILM)),
            ("A10_ColumbiumExtension_Inner", Some(NIOBIUM_FILM)),
            ("K1_RegenNozzle_OxidisedBronze", Some(BRONZE_FILM)),
            ("K1_TurbineExhaust_HeatTinted", Some(STEEL_FILM)),
            ("H10_TubeWallNozzle_HeatTintedInner", Some(STEEL_FILM)),
            ("H10_TubeWallNozzle_BrightSteel", Some(Finish::Metal)),
            ("K1_NozzleBand_DarkBronze", Some(Finish::Metal)),
            ("C4_DockingRing_Aluminium", Some(Finish::Metal)),
            ("C1_Shingles_DarkInconel", Some(Finish::Metal)),
            ("K1V_StiffeningRing_Gunmetal", Some(Finish::Metal)),
            ("C4_Sidewall_White", Some(Finish::Paint)),
            ("K1_Actuator_Black", Some(Finish::Paint)),
            ("RCS_Housing_LightGrey", Some(Finish::Paint)),
        ];
        for (name, finish) in cases {
            assert_eq!(Finish::from_material_name(name), finish, "{name}");
        }
    }
}
