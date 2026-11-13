//! Serializable record definitions (the moddable data formats).
//!
//! The game loads these from mod collections (`bodies`, `engines`, `tank_types`, `craft`,
//! `scenarios`, ...); every record has a string `id`. They are turned into runtime types
//! (`SolarSystem`, `Vessel`) which precompute derived values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    assembly::Side,
    atmosphere::Atmosphere,
    clouds::CloudsDef,
    component::{ComponentDef, PaintDef, StructureDef},
    geometry::{
        ShapeDef,
        preset::{ModelDef, NodeDef},
    },
    mission::GoalDef,
    ocean::OceanDef,
    scattering::ScatteringDef,
    sizing::ProceduralDef,
    skip::*,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDef {
    /// In any order; parents are resolved by id.
    pub bodies: Vec<BodyDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodyDef {
    /// Unique id, also the display name.
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// Gravitational parameter GM, m³/s².
    pub gm: f64,
    /// Mean radius, m.
    pub radius: f64,
    /// Highest terrain above `radius`, m. Below this (plus a margin) vessels are integrated
    /// so they can collide with the ground.
    #[serde(default)]
    pub max_terrain_height: f64,
    /// Baked terrain dataset (a directory relative to the mod root, e.g.
    /// `"terrain/earth"`). Without one the body is a smooth sphere.
    #[serde(default)]
    pub terrain: Option<String>,
    pub rotation: RotationDef,
    #[serde(default)]
    pub atmosphere: Option<Atmosphere>,
    /// How its air scatters sunlight, for the sky and the haze. Air without this is not drawn.
    #[serde(default)]
    pub scattering: Option<ScatteringDef>,
    /// Its clouds, for the eye and for the sunlight that gets through.
    #[serde(default)]
    pub clouds: Option<CloudsDef>,
    /// The waves on its seas.
    #[serde(default)]
    pub ocean: Option<OceanDef>,
    #[serde(default)]
    pub orbit: Option<OrbitDef>,
    /// Linear RGB used until bodies get real textures.
    pub color: [f32; 3],
    /// The share of the sunlight on it that it reflects (its Bond albedo); the rest it takes in
    /// and radiates back as heat.
    #[serde(default)]
    pub albedo: f64,
    /// The radiation around it, which only rules with `radiation` count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radiation: Option<BodyRadiationDef>,
    /// Where its science situations change ([`crate::science::Situation`]).
    #[serde(default)]
    pub science: BodyScienceDef,
}

/// Where an experiment's situation changes over a body, m above its sea level, and what science
/// done there is worth. A size it does not give is worked out ([`crate::body::Body::flying_high`],
/// [`crate::body::Body::space_high`]).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyScienceDef {
    /// Flying above this is flying high.
    #[serde(default)]
    pub flying_high: Option<f64>,
    /// In space above this is in space high.
    #[serde(default)]
    pub space_high: Option<f64>,
    /// What a whole set done in each situation over it is worth, times its experiment's `value`.
    #[serde(default)]
    pub values: SituationValues,
}

/// A figure for each situation ([`crate::science::Situation`]), 1 where it is not given.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationValues {
    #[serde(default = "one_value")]
    pub landed: f64,
    #[serde(default = "one_value")]
    pub splashed: f64,
    #[serde(default = "one_value")]
    pub flying_low: f64,
    #[serde(default = "one_value")]
    pub flying_high: f64,
    #[serde(default = "one_value")]
    pub space_low: f64,
    #[serde(default = "one_value")]
    pub space_high: f64,
}

fn one_value() -> f64 {
    1.0
}

impl Default for SituationValues {
    fn default() -> Self {
        Self {
            landed: 1.0,
            splashed: 1.0,
            flying_low: 1.0,
            flying_high: 1.0,
            space_low: 1.0,
            space_high: 1.0,
        }
    }
}

impl SituationValues {
    /// Its figure for `situation`.
    pub fn of(&self, situation: crate::science::Situation) -> f64 {
        use crate::science::Situation::*;
        match situation {
            Landed => self.landed,
            Splashed => self.splashed,
            FlyingLow => self.flying_low,
            FlyingHigh => self.flying_high,
            SpaceLow => self.space_low,
            SpaceHigh => self.space_high,
        }
    }
}

/// The radiation a body brings. Rates are what an unshielded person takes, mSv/h.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyRadiationDef {
    /// Galactic cosmic rays all through the system whose root it is (the Sun's), mSv/h.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cosmic: f64,
    /// The share of cosmic rays its magnetic field keeps out at its surface, less with the square
    /// of the distance further out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub magnetosphere: f64,
    /// Particles its field holds, along the field lines of a dipole on its spin axis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub belts: Vec<BeltDef>,
    /// Below this height (in its radii) the air takes the belts' particles, and from it to twice as
    /// high they come back in full.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub floor: f64,
    /// From its ground, on the surface, mSv/h; less as it fills less of the sky.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub surface: f64,
}

/// A belt of trapped particles: on the field lines that cross the magnetic equator `inner` to
/// `outer` body radii out, most at `peak`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeltDef {
    pub name: String,
    pub inner: f64,
    pub peak: f64,
    pub outer: f64,
    /// At its peak on the magnetic equator, mSv/h.
    pub rate: f64,
    /// The magnetic latitude, degrees, at which it is down to 1/e of that.
    #[serde(default = "default_belt_latitude")]
    pub latitude: f64,
}

fn default_belt_latitude() -> f64 {
    30.0
}

/// What radiation does (collection `radiation`): how much shielding stops each kind, and the dose
/// at which people sicken. The rules name the one a game uses ([`RulesDef::radiation`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadiationProfileDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The shielding, kg/m², that cuts each kind to 1/e: cosmic rays go through a great deal, the
    /// belts' particles very little.
    pub shielding: RadiationKindsDef,
    /// The dose, Sv, at which a person is sick, weak, critical, and dies, before their resilience.
    pub sick: f64,
    pub weak: f64,
    pub critical: f64,
    pub dies: f64,
}

/// A figure for each kind of radiation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadiationKindsDef {
    pub cosmic: f64,
    pub trapped: f64,
    pub surface: f64,
}

/// IAU/WGCCRE rotation model, referenced to the ICRF (J2000 equator and equinox).
///
/// The body's north pole points at right ascension `pole_ra` and declination `pole_dec`.
/// The prime meridian's angle W = `prime_meridian` + `rate` · d, where d is days since
/// J2000, measured along the body's equator from its ascending node on the ICRF equator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationDef {
    /// Degrees.
    pub pole_ra: f64,
    pub pole_dec: f64,
    /// W at J2000, degrees.
    pub prime_meridian: f64,
    /// Degrees per day; negative for retrograde rotation.
    pub rate: f64,
}

/// Keplerian elements in degrees, referenced to the J2000 ecliptic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitDef {
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub longitude_of_ascending_node: f64,
    pub argument_of_periapsis: f64,
    pub mean_anomaly_at_epoch: f64,
    /// Seconds since J2000.
    #[serde(default)]
    pub epoch: f64,
}

/// How much of the real thing a game asks for where that is not a matter of its parts' data (the
/// record `game` of the collection `rules`). Without a record, all of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesDef {
    pub id: String,
    /// Control takes avionics rated for the vessel's mass and where it is, or a crew for a small
    /// one by hand. Without this, any pod or avionics unit flies whatever it is on, and a vessel
    /// with neither does nothing it is told.
    pub avionics: bool,
    /// The record of `life_support` that people aboard parts with life support live by: what each
    /// needs and how long they last without it. None: nobody needs anything.
    pub life_support: Option<String>,
    /// The record of `radiation` people take dose by. None: radiation does nothing.
    pub radiation: Option<String>,
    /// The record of `tech_trees` a science game unlocks parts on.
    pub tech_tree: Option<String>,
    /// Data goes home at the rate a link carries, slower further away and shared through relays.
    /// Without this, all of a vessel's data goes at once whenever it has a link.
    #[serde(default)]
    pub data_rates: bool,
}

impl Default for RulesDef {
    fn default() -> Self {
        Self {
            id: "game".to_owned(),
            avionics: true,
            life_support: Some("realistic".to_owned()),
            radiation: Some("realistic".to_owned()),
            tech_tree: Some("rp1".to_owned()),
            data_rates: true,
        }
    }
}

/// What people need to live (collection `life_support`), for a part with life support. The rules
/// name the one a game uses ([`RulesDef::life_support`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifeSupportProfileDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub needs: Vec<NeedDef>,
    /// What a cabin's air is made of and kept at. Without it a cabin has no air of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air: Option<AirDef>,
    /// What the air has to be for people to be well; out of it, they go short as of a need.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<AirLimitDef>,
    /// Cabin per seat, m³, where a part does not say.
    #[serde(default = "default_room_per_seat")]
    pub room_per_seat: f64,
    /// Processes every cabin with life support is fitted with, so many per seat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fitted: Vec<FittedDef>,
    /// What people make that is let go of when there is nowhere to keep it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dump: Vec<String>,
}

fn default_room_per_seat() -> f64 {
    2.0
}

impl LifeSupportProfileDef {
    /// What a need for `resource` is met from in the lockers and tanks: a gas of the cabin's air
    /// comes from what its regulator tops it up with.
    pub fn supply_of<'a>(&'a self, resource: &'a str) -> &'a str {
        (self.air.iter().flat_map(|air| &air.gases))
            .find(|gas| gas.resource == resource)
            .and_then(|gas| gas.from.as_deref())
            .unwrap_or(resource)
    }

    /// Whether `resource` is a gas of the cabin's air.
    pub fn is_air(&self, resource: &str) -> bool {
        (self.air.iter().flat_map(|air| &air.gases)).any(|gas| gas.resource == resource)
    }
}

/// A cabin's air: the gases in it, each a resource only cabins keep, at partial pressures from what
/// there is of each in the cabin's volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirDef {
    /// K, until cabins have a temperature of their own.
    #[serde(default = "default_cabin_temperature")]
    pub temperature: f64,
    pub gases: Vec<GasDef>,
    /// How long the regulator takes to fill an empty cabin, s: it tops each gas up at its `keep`
    /// over this long, and gives no more than that against a leak.
    #[serde(default = "default_refill")]
    pub refill: f64,
    /// The valve a cabin is let down through, m².
    #[serde(default = "default_vent")]
    pub vent: f64,
}

fn default_cabin_temperature() -> f64 {
    294.0
}

fn default_refill() -> f64 {
    60.0
}

fn default_vent() -> f64 {
    1e-3
}

/// One gas of a cabin's air.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasDef {
    /// Its resource, kept nowhere but in cabins.
    pub resource: String,
    /// What people call it; the resource's id if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// kg/mol.
    pub molar_mass: f64,
    /// The partial pressure a cabin starts at and its regulator keeps it at, Pa, from `from`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub keep: f64,
    /// What the regulator tops it up from; it is not topped up without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The most there can be of it, Pa: `keep` if left out. With it full no more goes in: of CO2,
    /// people cannot breathe out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub most: Option<f64>,
}

impl GasDef {
    /// What people call it.
    pub fn called(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.resource)
    }

    /// The most there can be of it, Pa.
    pub fn most(&self) -> f64 {
        self.most.unwrap_or(self.keep)
    }
}

/// A bound on a cabin's air for people to be well: a gas's partial pressure, or the whole pressure,
/// kept at or above `least` and at or below `most`. Out of it, people go short of it as of a need,
/// by stages, and die after `lasts`; back in it, they recover as fast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirLimitDef {
    /// What people go short of: "air pressure", "clean air".
    pub name: String,
    /// The gas's resource; the whole pressure if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas: Option<String>,
    /// Pa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub least: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub most: Option<f64>,
    /// s, before each person's resilience.
    pub lasts: f64,
    #[serde(default = "default_weak")]
    pub weak: f64,
    #[serde(default = "default_critical")]
    pub critical: f64,
}

impl AirLimitDef {
    /// It as a need people go short of, which the crew's stages count like any other.
    pub fn need(&self) -> NeedDef {
        NeedDef {
            resource: self.name.clone(),
            name: Some(self.name.clone()),
            daily: 0.0,
            lasts: Some(self.lasts),
            makes: BTreeMap::new(),
            weak: self.weak,
            critical: self.critical,
        }
    }

    /// Whether air at `pressure` (Pa), whose gases are at `partial` (resource, Pa), is out of it.
    pub fn broken_by(&self, pressure: f64, partial: &[(String, f64)]) -> bool {
        let value = match &self.gas {
            None => pressure,
            Some(gas) => (partial.iter())
                .find(|(resource, _)| resource == gas)
                .map_or(0.0, |(_, p)| *p),
        };
        self.least.is_some_and(|least| value < least) || self.most.is_some_and(|most| value > most)
    }
}

/// A process every cabin has: `process`, at a scale of `per_seat` for each of its seats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittedDef {
    pub process: String,
    #[serde(default = "one")]
    pub per_seat: f64,
}

/// A way of turning some resources into others (collection `processes`), which a part with a
/// `process` component runs: electrolysis, a water recycler, a converter. What it makes cannot
/// weigh more than what it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// What it takes at full speed, kg/s.
    #[serde(default)]
    pub inputs: BTreeMap<String, f64>,
    /// What it makes at full speed, kg/s.
    #[serde(default)]
    pub outputs: BTreeMap<String, f64>,
    /// Outputs it lets go of when there is no room for them, instead of slowing down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dump: Vec<String>,
    /// Electric power it draws at full speed, W.
    #[serde(default)]
    pub power: f64,
}

impl ProcessDef {
    /// Why it cannot be run, if it cannot.
    pub fn problem(&self) -> Option<String> {
        let rates = self.inputs.values().chain(self.outputs.values());
        if !(self.power >= 0.0 && rates.clone().all(|r| *r >= 0.0 && r.is_finite())) {
            return Some(format!(
                "process `{}` has a negative or endless rate",
                self.id
            ));
        }
        let (taken, made): (f64, f64) = (self.inputs.values().sum(), self.outputs.values().sum());
        (made > taken * (1.0 + 1e-6)).then(|| {
            format!(
                "process `{}` makes {made} kg/s from {taken} kg/s: more than it takes",
                self.id
            )
        })
    }
}

/// One thing a person uses, from the tanks their part can reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedDef {
    pub resource: String,
    /// What people call it when they go short of it ("air"); the resource's id if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What each person uses a day, kg.
    pub daily: f64,
    /// How long people last with none of it, s, before their own resilience (1 to 1.2 times as
    /// long). Without it, going short does not matter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lasts: Option<f64>,
    /// What each person makes of it a day, kg: breathing oxygen makes CO2.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub makes: BTreeMap<String, f64>,
    /// The share of `lasts` without it after which a person is weak, and then critical.
    #[serde(default = "default_weak")]
    pub weak: f64,
    #[serde(default = "default_critical")]
    pub critical: f64,
}

impl NeedDef {
    /// What people call it.
    pub fn called(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.resource)
    }
}

fn default_weak() -> f64 {
    1.0 / 3.0
}

fn default_critical() -> f64 {
    2.0 / 3.0
}

/// Names for the crew (collection `crew_names`): every record's first and last names go into one
/// pool, and each person is given one of each. A mod adds a record, or replaces `stock`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewNamesDef {
    pub id: String,
    #[serde(default)]
    pub first: Vec<String>,
    #[serde(default)]
    pub last: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartCatalog {
    #[serde(default)]
    pub rules: RulesDef,
    pub engines: Vec<EngineDef>,
    #[serde(default)]
    pub resources: Vec<ResourceDef>,
    #[serde(default)]
    pub materials: Vec<MaterialDef>,
    #[serde(default)]
    pub tank_types: Vec<TankTypeDef>,
    #[serde(default)]
    pub reaction_wheels: Vec<ReactionWheelDef>,
    #[serde(default)]
    pub rcs: Vec<RcsDef>,
    #[serde(default)]
    pub pods: Vec<PodDef>,
    #[serde(default)]
    pub batteries: Vec<BatteryDef>,
    #[serde(default)]
    pub avionics: Vec<AvionicsDef>,
    /// Paint schemes the stage converter can give a stage.
    #[serde(default)]
    pub liveries: Vec<LiveryDef>,
    /// What people can be given to live on; the rules pick one.
    #[serde(default)]
    pub life_support: Vec<LifeSupportProfileDef>,
    /// What radiation can do; the rules pick one.
    #[serde(default)]
    pub radiation: Vec<RadiationProfileDef>,
    /// What `process` components can run.
    #[serde(default)]
    pub processes: Vec<ProcessDef>,
    /// What the crew can be called.
    #[serde(default)]
    pub crew_names: Vec<CrewNamesDef>,
    /// Engines that are configurations of one engine, which the parts list shows as one part.
    #[serde(default)]
    pub engine_families: Vec<EngineFamilyDef>,
    /// What scientific instruments can do.
    #[serde(default)]
    pub experiments: Vec<ExperimentDef>,
    /// The parts that carry them.
    #[serde(default)]
    pub instruments: Vec<InstrumentDef>,
}

/// An experiment (collection `experiments`): what an instrument collects, over how long, and where
/// it works. A whole set is collected once in each place ([`crate::science::subject`]): its body
/// and situation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Seconds it takes to collect a whole set in one place, running all the time.
    pub duration: f64,
    /// Bytes a whole set comes to, for a measurement.
    #[serde(default)]
    pub size: f64,
    /// kg a whole set weighs, for a physical sample (soil, film) that has to come home: then it
    /// has no `size`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<f64>,
    /// W while it runs.
    #[serde(default)]
    pub power: f64,
    /// Science a whole set is worth, before where it was done counts.
    #[serde(default)]
    pub value: f64,
    /// Where it works.
    pub situations: Vec<crate::science::Situation>,
    /// Only over these bodies; over any if none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<String>,
    /// Only in sunlight.
    #[serde(default)]
    pub sunlight: bool,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

impl ExperimentDef {
    /// What is wrong with it as a record, if anything.
    pub fn problem(&self) -> Option<String> {
        let id = &self.id;
        if !(self.duration > 0.0 && self.duration.is_finite()) {
            return Some(format!("experiment `{id}` takes some time"));
        }
        let nothing = |v: f64| v.is_nan() || v <= 0.0;
        match self.sample {
            Some(kg) if nothing(kg) => Some(format!(
                "experiment `{id}` collects a sample that weighs something"
            )),
            None if nothing(self.size) => Some(format!("experiment `{id}` makes some data")),
            _ if self.situations.is_empty() => Some(format!("experiment `{id}` works somewhere")),
            _ if self.power < 0.0 || self.value < 0.0 => {
                Some(format!("experiment `{id}` has no negative power or value"))
            }
            _ => None,
        }
    }

    /// What the player reads.
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// A scientific instrument (collection `instruments`): a part that carries experiments, put on a
/// craft's side like an antenna.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The `experiments` it runs.
    pub experiments: Vec<String>,
    /// kg.
    pub mass: f64,
    /// Its box, m: across, deep, and out from what it is on.
    pub size: [f64; 3],
    /// How many whole sets of each of its experiments it keeps on a drive of its own, data and
    /// samples, until they are moved or sent.
    #[serde(default = "one_set")]
    pub holds: f64,
    /// Linear RGB.
    #[serde(default = "instrument_color")]
    pub color: [f64; 3],
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

fn one_set() -> f64 {
    1.0
}

fn instrument_color() -> [f64; 3] {
    [0.75, 0.62, 0.3]
}

impl PartCatalog {
    /// The configurations of the engine `engine` is one of, in its family's order, those this
    /// catalogue has: `engine` alone if it has no family.
    pub fn configurations(&self, engine: &str) -> Vec<&EngineDef> {
        let find = |id: &str| self.engines.iter().find(|e| e.id == id);
        match self.family_of(engine) {
            Some(family) => family.configs.iter().filter_map(|id| find(id)).collect(),
            None => find(engine).into_iter().collect(),
        }
    }

    /// The family `engine` is a configuration of, if it names one that lists it.
    pub fn family_of(&self, engine: &str) -> Option<&EngineFamilyDef> {
        let named = (self.engines.iter().find(|e| e.id == engine))?
            .family
            .as_deref()?;
        (self.engine_families.iter())
            .find(|f| f.id == named && f.configs.iter().any(|c| c == engine))
    }

    /// The configuration of `engine`'s family a new one is: the family's default, or its first.
    pub fn first_configuration(&self, engine: &str) -> Option<&EngineDef> {
        let configs = self.configurations(engine);
        let default = (self
            .family_of(engine)
            .and_then(|f| f.default_config.as_deref()))
        .and_then(|d| configs.iter().find(|c| c.id == d).copied());
        default.or(configs.first().copied())
    }

    /// A name for whoever sits in seat `seat` of the part called `part`: always the same one for
    /// the same seat of the same part, from the `crew_names` records.
    pub fn crew_name(&self, part: &str, seat: u32) -> String {
        let first: Vec<&String> = self.crew_names.iter().flat_map(|n| &n.first).collect();
        let last: Vec<&String> = self.crew_names.iter().flat_map(|n| &n.last).collect();
        if first.is_empty() || last.is_empty() {
            return format!("Crew member {}", seat + 1);
        }
        let pick =
            |salt: &str, of: usize| (fnv(&format!("{part}/{seat}/{salt}")) % of as u64) as usize;
        format!(
            "{} {}",
            first[pick("first", first.len())],
            last[pick("last", last.len())]
        )
    }

    /// The life support the rules ask for: none if they name no profile, an error if they name one
    /// nobody wrote.
    pub fn life_support_profile(&self) -> Result<Option<&LifeSupportProfileDef>, String> {
        let Some(id) = &self.rules.life_support else {
            return Ok(None);
        };
        (self.life_support.iter())
            .find(|profile| profile.id == *id)
            .map(Some)
            .ok_or_else(|| {
                format!("the rules' life support `{id}` has no record in `life_support`")
            })
    }

    /// What radiation does under the rules: nothing if they name no record, an error if they name
    /// one nobody wrote.
    pub fn radiation_profile(&self) -> Result<Option<&RadiationProfileDef>, String> {
        let Some(id) = &self.rules.radiation else {
            return Ok(None);
        };
        (self.radiation.iter())
            .find(|profile| profile.id == *id)
            .map(Some)
            .ok_or_else(|| format!("the rules' radiation `{id}` has no record in `radiation`"))
    }

    /// What people aboard a part with life support need, under the rules.
    pub fn life_support_needs(&self) -> Result<Option<&[NeedDef]>, String> {
        Ok(self.life_support_profile()?.map(|p| p.needs.as_slice()))
    }
}

/// A paint scheme for a stage (collection `liveries`), such as a rocket's historical one.
/// The stage converter paints the stage's body, tanks, shells and fairing with it, its
/// layers' heights from the stage's base; a craft keeps the paint, not the livery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveryDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub paint: crate::component::PaintDef,
}

/// A propellant or other stored fluid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDef {
    pub id: String,
    /// Density as stored, kg/m³.
    pub density: f64,
    /// Storage temperature, K. Only tanks insulated down to it can hold it.
    #[serde(default = "room_temperature")]
    pub temperature: f64,
    /// Storage pressure, Pa (compressed gases). Only tanks rated for it can hold it.
    #[serde(default)]
    pub pressure: f64,
}

/// A structural material for tanks and fins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDef {
    pub id: String,
    /// kg/m³.
    pub density: f64,
    /// Stress at which it yields, Pa.
    pub yield_strength: f64,
    /// Young's modulus, Pa.
    pub elastic_modulus: f64,
    /// How hot it can get before a part made of it fails, K.
    #[serde(default = "default_max_temperature")]
    pub max_temperature: f64,
    /// J/kg K.
    #[serde(default = "default_specific_heat")]
    pub specific_heat: f64,
}

fn default_max_temperature() -> f64 {
    1200.0
}

fn default_specific_heat() -> f64 {
    900.0
}

/// How a procedural tank is built. Its wall is sized from these (see [`crate::structure`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TankTypeDef {
    pub id: String,
    /// Id of a `materials` record.
    pub material: String,
    /// Operating (ullage) pressure, Pa.
    pub pressure: f64,
    /// Thinnest wall that can be built, m.
    pub min_gauge: f64,
    /// Mass of the stiffening (isogrid ribs, stringers, liner) as a multiple of the bare wall.
    #[serde(default = "one")]
    pub stiffening: f64,
    /// How much more axial load the stiffened wall carries before buckling than a bare
    /// wall of the same thickness.
    #[serde(default = "one")]
    pub buckling_factor: f64,
    /// Insulation, kg per m² of tank surface.
    #[serde(default)]
    pub insulation: f64,
    /// Coldest propellant it can hold, K.
    #[serde(default = "uninsulated")]
    pub min_temperature: f64,
    /// Share of the internal volume that holds propellant; the rest is ullage, baffles and
    /// plumbing.
    #[serde(default = "default_utilization")]
    pub utilization: f64,
    /// Yield strength over the design stress.
    #[serde(default = "default_safety_factor")]
    pub safety_factor: f64,
    /// Acceleration, in g, at which a full tank's hydrostatic pressure is designed for.
    #[serde(default = "default_design_load")]
    pub design_load: f64,
    /// If given, only these resources (on top of the temperature and pressure limits).
    #[serde(default)]
    pub propellants: Option<Vec<String>>,
    /// Its contents are cast solid in a motor's case, which the engine record weighs: they
    /// press on no wall, and the case, built for the chamber pressure, carries the stack.
    /// Its walls are only the case's liner, off the load path.
    #[serde(default)]
    pub cast: bool,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A procedural tank in a stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TankDef {
    /// Id of a `tank_types` record.
    #[serde(rename = "type")]
    pub tank_type: String,
    #[serde(default)]
    pub shape: TankShape,
    /// Cylinder length including its domes, m. Cylinders stack up the stage from its base
    /// in the order listed.
    #[serde(default)]
    pub length: f64,
    /// Height of its base above the stage base, m, to place it higher than it would stack
    /// (a cylinder must still clear the one before it). A sphere sits on the stage base
    /// without it.
    #[serde(default)]
    pub position: Option<f64>,
    /// Outer diameter, m. Cylinders default to the stage diameter; spheres need it.
    #[serde(default)]
    pub diameter: Option<f64>,
    /// Acceleration, in g, at which the full tank's hydrostatic pressure is designed for;
    /// defaults to the tank type's. An upper stage rides its booster's peak acceleration
    /// with full tanks, so it needs more than a first stage, which is only full at liftoff.
    #[serde(default)]
    pub design_load: Option<f64>,
    /// What it holds: resource -> mass ratio (normalised). It is filled to its usable
    /// volume in this ratio.
    pub propellants: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TankShape {
    /// A cylinder with elliptical (√2:1) domes, carrying the stack's load in its wall.
    #[default]
    Cylinder,
    /// A sphere mounted inside the stage (e.g. a pressurant bottle), off the load path.
    Sphere,
}

/// A set of identical procedural fins around the base of a stage: flat, solid, trapezoidal
/// plates with their trailing edge at the stage base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinDef {
    #[serde(default = "default_fin_count")]
    pub count: u32,
    /// Chord along the hull and at the tip, m.
    pub root_chord: f64,
    pub tip_chord: f64,
    /// How far the fin sticks out from the hull, m.
    pub span: f64,
    /// How far the tip's leading edge sits behind the root's, m.
    #[serde(default)]
    pub sweep: f64,
    pub thickness: f64,
    /// Id of a `materials` record.
    pub material: String,
}

/// A lifting surface on a stage: a wing, tailplane or tail fin. Unlike rocket fins it has
/// an airfoil (so it stalls), can be set at an angle to the hull, and can carry a control
/// surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WingDef {
    #[serde(default)]
    pub name: String,
    /// Height of the root's leading edge above the stage base, m.
    pub position: f64,
    /// Where around the hull the root stands, degrees from the pilot's right (vessel +X)
    /// towards their feet (+Y): 0 a right wing, 270 a fin on top.
    #[serde(default)]
    pub angle: f64,
    /// Chord at the root and at the tip, m.
    pub root_chord: f64,
    pub tip_chord: f64,
    /// From the hull to the tip, m.
    pub span: f64,
    /// How far the tip's leading edge sits behind the root's, m.
    #[serde(default)]
    pub sweep: f64,
    /// Degrees the panel turns about the hull's axis towards smaller `angle` (a right
    /// wing's tip goes up).
    #[serde(default)]
    pub dihedral: f64,
    /// Degrees the leading edge turns towards the lifting side (up, for a wing).
    #[serde(default)]
    pub incidence: f64,
    /// Degrees of angle of attack its camber and twist are worth: it lifts as if turned this
    /// much more.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub camber_incidence: f64,
    /// Airfoil thickness over chord.
    #[serde(default = "default_wing_thickness")]
    pub thickness: f64,
    /// All panels together, kg.
    pub mass: f64,
    /// A second panel mirrored across the vessel's plane of symmetry (a left wing for a
    /// right one).
    #[serde(default)]
    pub mirror: bool,
    #[serde(default)]
    pub control: Option<ControlSurfaceDef>,
}

/// A hinged trailing-edge surface: aileron, elevator, rudder, or a mix such as elevons.
/// Which way it moves for a command follows from where it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlSurfaceDef {
    /// The rotation commands that move it.
    pub axes: Vec<ControlAxis>,
    /// Share of the chord, from the trailing edge.
    #[serde(default = "default_control_chord")]
    pub chord: f64,
    /// Where it runs, as shares of the span from the root.
    #[serde(default = "default_control_span")]
    pub span: [f64; 2],
    /// Largest deflection each way, degrees.
    #[serde(default = "default_control_deflection")]
    pub deflection: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlAxis {
    Pitch,
    Yaw,
    Roll,
}

impl ControlAxis {
    /// The vessel axis it turns about: X, Y or Z.
    pub fn index(self) -> usize {
        match self {
            ControlAxis::Pitch => 0,
            ControlAxis::Yaw => 1,
            ControlAxis::Roll => 2,
        }
    }
}

fn default_wing_thickness() -> f64 {
    0.12
}

fn default_control_chord() -> f64 {
    0.25
}

fn default_control_span() -> [f64; 2] {
    [0.0, 1.0]
}

fn default_control_deflection() -> f64 {
    20.0
}

/// A landing gear leg: a sprung, damped strut standing out from the hull with a wheel at
/// its end. It rolls freely along the vessel's nose, grips sideways like a tyre, and can
/// brake and steer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GearDef {
    #[serde(default)]
    pub name: String,
    /// Height of the strut's mount above the stage base, m.
    pub position: f64,
    /// Where around the hull it stands, degrees from the pilot's right (vessel +X) towards
    /// their feet (+Y): 90 straight down, less to splay a right-hand leg outwards.
    #[serde(default = "default_gear_angle")]
    pub angle: f64,
    /// From the hull to the wheel's axle, strut fully extended, m.
    pub length: f64,
    /// Wheel radius, m.
    #[serde(default = "default_wheel_radius")]
    pub wheel_radius: f64,
    /// Strut travel, m.
    #[serde(default = "default_stroke")]
    pub stroke: f64,
    /// Mass the leg is sprung for: standing still under it, the strut sits a third of the
    /// way into its travel, kg.
    pub load: f64,
    /// All legs together (with `mirror`), kg.
    pub mass: f64,
    /// A second leg mirrored across the vessel's plane of symmetry.
    #[serde(default)]
    pub mirror: bool,
    /// The wheel has brakes.
    #[serde(default)]
    pub brakes: bool,
    /// The wheel steers with the yaw command, this many degrees each way.
    #[serde(default)]
    pub steering: f64,
    /// It ends in a foot, which does not roll: `wheel_radius` is the pad's.
    #[serde(default)]
    pub foot: bool,
}

fn default_gear_angle() -> f64 {
    90.0
}

fn default_wheel_radius() -> f64 {
    0.3
}

fn default_stroke() -> f64 {
    0.3
}

/// A crew cabin (capsule or cockpit). It sits at the top of its stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodDef {
    pub id: String,
    /// Mass with crew and consumables, kg.
    pub mass: f64,
    pub crew: u32,
    /// Base diameter and height, m.
    pub diameter: f64,
    pub height: f64,
    /// Diameter at the top, m, for the aerodynamics: the pod is a frustum to the air. A
    /// third of the base by default.
    #[serde(default)]
    pub top_diameter: Option<f64>,
    /// glTF model, relative to the mod root.
    #[serde(default)]
    pub model: Option<String>,
    /// Where other parts join it, in the model's frame: origin at the base centre, +Z up
    /// (sim axes).
    #[serde(default)]
    pub nodes: Vec<NodeDef>,
    /// Stored electricity, kJ (counted in `mass` with its batteries).
    #[serde(default)]
    pub electric_charge: f64,
    /// Days of what its crew need that it carries in its lockers (counted in `mass`), under the
    /// rules' life support. None: it has no life support.
    #[serde(default)]
    pub supplies: f64,
    /// Its drive, for what experiments make: data and samples to bring home. A pod with seats and
    /// none has [`POD_DRIVE_PER_SEAT`] for each seat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<crate::component::DriveDef>,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// The drive a pod has for each seat when its record gives it none: 2 MB of data and 5 kg of
/// samples.
pub const POD_DRIVE_PER_SEAT: crate::component::DriveDef = crate::component::DriveDef {
    data: 2e6,
    samples: 5.0,
};

impl PodDef {
    pub fn top_diameter(&self) -> f64 {
        self.top_diameter.unwrap_or(self.diameter / 3.0)
    }

    /// Its drive, if it has one: its own, or [`POD_DRIVE_PER_SEAT`] for each seat.
    pub fn drive(&self) -> Option<crate::component::DriveDef> {
        let seats = f64::from(self.crew);
        self.drive.or_else(|| {
            (self.crew > 0).then_some(crate::component::DriveDef {
                data: POD_DRIVE_PER_SEAT.data * seats,
                samples: POD_DRIVE_PER_SEAT.samples * seats,
            })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineDef {
    pub id: String,
    /// Dry mass, kg.
    pub mass: f64,
    /// Vacuum thrust, N.
    pub thrust_vac: f64,
    /// Specific impulse in vacuum and at 1 atm, s.
    pub isp_vac: f64,
    pub isp_sl: f64,
    /// Lowest thrust share while running, 0..1: throttle settings below it run the engine
    /// at this floor. 1.0 means not throttleable.
    #[serde(default = "one")]
    pub min_throttle: f64,
    /// How the thrust follows the throttle: start-up, throttling and shutdown.
    #[serde(default)]
    pub response: EngineResponse,
    /// Number of ignitions available; `None` = unlimited.
    #[serde(default)]
    pub ignitions: Option<u32>,
    /// Whether the engine needs settled propellant (positive acceleration) to ignite.
    #[serde(default)]
    pub requires_ullage: bool,
    /// How far the engine can swivel from the vessel axis to steer, degrees. 0 = fixed.
    #[serde(default)]
    pub gimbal_range: f64,
    /// Propellants consumed: resource -> mass ratio (normalised at load).
    pub propellants: BTreeMap<String, f64>,
    /// How its thrust follows what is left of its propellant, as a solid's does while its
    /// grain burns back: (share of the propellant feeding it left, thrust and mass flow as
    /// a share of `thrust_vac`) points, linear between them. Without one it gives full
    /// thrust to the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thrust_curve: Option<Vec<[f64; 2]>>,
    /// From the gimbal pivot to the nozzle exit, m.
    #[serde(default)]
    pub length: f64,
    /// Nozzle exit diameter, m.
    #[serde(default)]
    pub exit_diameter: f64,
    /// glTF model, relative to the mod root: origin at the gimbal pivot, nozzle down −Y.
    #[serde(default)]
    pub model: Option<String>,
    /// Scales the model evenly, to make a smaller or larger engine from it (`length` and
    /// `exit_diameter` are the scaled engine's).
    #[serde(default = "one")]
    pub model_scale: f64,
    /// Where other parts join it, in the unscaled model's frame: origin at the gimbal pivot,
    /// +Z up towards the stage above (sim axes).
    #[serde(default)]
    pub nodes: Vec<NodeDef>,
    /// Pressure of the exhaust leaving the nozzle at full thrust, Pa. Against the ambient
    /// pressure it decides how the plume expands; estimated from the Isp lost at sea level
    /// if missing.
    #[serde(default)]
    pub exit_pressure: Option<f64>,
    /// Look of the exhaust: id of a `plumes` record. Without one the engine gets the look for
    /// the propellants it burns, if there is one.
    #[serde(default)]
    pub plume: Option<String>,
    /// Where an open-cycle (gas-generator) engine dumps the exhaust that drove its turbine,
    /// which shows beside its plume. Closed-cycle engines burn it in the chamber.
    #[serde(default)]
    pub turbine_exhaust: Option<TurbineExhaust>,
    /// The engine family it is a configuration of (e.g. `"AJ10_Early"`).
    #[serde(default)]
    pub family: Option<String>,
    /// `liquid`, `solid` or `electric`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Draws its propellant from tanks pressurised by gas instead of pumping it.
    #[serde(default)]
    pub pressure_fed: bool,
    /// Electric power drawn at full thrust, kW (electric thrusters).
    #[serde(default)]
    pub electric_power: Option<f64>,
    #[serde(default)]
    pub reliability: Option<ReliabilityDef>,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// How often an engine fails, TestFlight style: each share rises from `*_start` for a
/// new engine to `*_end` for a mature one with flight experience.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReliabilityDef {
    /// Chance an ignition succeeds.
    pub ignition_start: Option<f64>,
    pub ignition_end: Option<f64>,
    /// Chance a burn of `rated_burn_time` runs without failing.
    pub cycle_start: Option<f64>,
    pub cycle_end: Option<f64>,
    /// Longest burn it is designed for, and the longest it was tested to, s.
    pub rated_burn_time: Option<f64>,
    pub tested_burn_time: Option<f64>,
}

/// What every catalogue record can say about itself besides its physics: names, the year
/// it first flew and the tech tree node that unlocks it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogInfo {
    pub title: Option<String>,
    pub manufacturer: Option<String>,
    pub description: Option<String>,
    /// First year it flew or was in service, AD. With no year of its own it is available
    /// once its `tech` node is.
    pub year: Option<i32>,
    /// Id of a `tech_nodes` record.
    pub tech: Option<String>,
    /// Vehicles it flew on.
    pub used_on: Option<String>,
    /// Where the data came from.
    pub source: Option<String>,
}

/// Where a gas-generator engine's turbine exhaust leaves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TurbineExhaust {
    /// Out of a pipe of its own (the Merlin's): the centre of its outlet in the engine's
    /// axes (from the gimbal pivot, x and z across, the nozzle down −y) and its diameter,
    /// m, at the scale the model is drawn. It points along the nozzle.
    Pipe { exit: [f64; 3], diameter: f64 },
    /// Into the nozzle, where it lines the wall (the F-1's and J-2's extensions).
    Nozzle,
}

impl EngineDef {
    /// Whether it is a solid: it burns at full thrust until its grain is gone, whatever it is told.
    pub fn is_solid(&self) -> bool {
        self.kind.as_deref() == Some("solid")
    }

    /// How far the nozzle reaches out from its mount, m. Most of a solid's is sunk into the
    /// case (its `length` is the whole of it), so only a short, wide stub shows.
    pub fn nozzle_length(&self) -> f64 {
        if self.is_solid() {
            self.length.min(self.exit_diameter)
        } else {
            self.length
        }
    }

    /// Nozzle exit pressure at full thrust, Pa: the given one, or a guess from how much
    /// Isp the engine loses at sea level (sea-level engines exit near 1 atm, vacuum engines
    /// at about 1 kPa).
    pub fn exit_pressure(&self) -> f64 {
        self.exit_pressure.unwrap_or_else(|| {
            let kept = self.isp_sl / self.isp_vac.max(1.0);
            101_325.0 * ((kept - 0.4) * 1.6).clamp(0.01, 1.0)
        })
    }
}

/// How an engine's thrust follows its throttle, in shares of full thrust. All zero (the
/// default) is an ideal engine that does whatever the throttle says at once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineResponse {
    /// From ignition to full thrust, s: turbopumps spinning up, chamber pressure building.
    pub spool_up: f64,
    /// Shape of the start as `[time, thrust]` points, both as shares (time of `spool_up`,
    /// thrust of full), from `[0, 0]` to `[1, 1]`, piecewise linear. A start throttled
    /// below full follows it until it reaches the setting.
    pub startup: Vec<[f64; 2]>,
    /// From cutoff until the thrust has tailed off to 1%, s.
    pub spool_down: f64,
    /// Fastest throttling: time to sweep the thrust from zero to full, s.
    pub throttle_time: f64,
}

impl Default for EngineResponse {
    fn default() -> Self {
        Self {
            spool_up: 0.0,
            startup: vec![[0.0, 0.0], [1.0, 1.0]],
            spool_down: 0.0,
            throttle_time: 0.0,
        }
    }
}

impl EngineResponse {
    /// Thrust share `elapsed` seconds into a start.
    pub fn startup_level(&self, elapsed: f64) -> f64 {
        if self.spool_up <= 0.0 || elapsed >= self.spool_up {
            return 1.0;
        }
        let x = (elapsed / self.spool_up).max(0.0);
        let points = &self.startup;
        let Some(end) = points.iter().position(|p| p[0] >= x) else {
            return 1.0;
        };
        if end == 0 {
            return points[0][1].clamp(0.0, 1.0);
        }
        let ([x0, y0], [x1, y1]) = (points[end - 1], points[end]);
        let f = if x1 > x0 { (x - x0) / (x1 - x0) } else { 1.0 };
        (y0 + (y1 - y0) * f).clamp(0.0, 1.0)
    }
}

/// Torque without propellant, for turning while the engines are off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionWheelDef {
    pub id: String,
    pub mass: f64,
    /// Torque about each axis (pitch, yaw and roll) at full command, N·m.
    pub torque: f64,
    /// glTF model drawn for it, relative to the mod root: origin at the centre of its base, +Z up,
    /// 0.5 m across and 0.2 m tall. A plain drum without one.
    #[serde(default)]
    pub model: Option<String>,
}

/// A block of reaction control thrusters: four small fixed nozzles pushing forward, back and
/// both ways around the vessel axis. They turn the vessel and push it along, e.g. to settle
/// propellant before an engine start, burning their stage's propellant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcsDef {
    pub id: String,
    /// Dry mass, kg.
    pub mass: f64,
    /// Vacuum thrust of each nozzle, N.
    pub thrust: f64,
    /// Specific impulse in vacuum and at 1 atm, s.
    pub isp_vac: f64,
    pub isp_sl: f64,
    /// Propellants consumed: resource -> mass ratio (normalised at load).
    pub propellants: BTreeMap<String, f64>,
    /// glTF model, relative to the mod root: origin on the hull, +X outward, +Y forward.
    #[serde(default)]
    pub model: Option<String>,
}

/// A craft (collection `craft`): plain parts joined into a tree, and what the stage key
/// does. How the parts were placed (with symmetry, as a cluster, from an assembly, converted
/// from stages) leaves no trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CraftDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    /// In any order. Exactly one, the root, is on nothing.
    pub parts: Vec<CraftPartDef>,
    /// What each press of the stage key does, in order: lists of activations written
    /// `"<what>:<part id>"`, where `<what>` is `engines` (light the engines of the part's
    /// stage), `decoupler` (release the joint above its stage, dropping it and everything
    /// below), `fairing` (jettison its stage's fairing) or `rcs` (switch RCS on). Without
    /// it the sequences are made from the stages: see `Sequences::default_for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequences: Option<Vec<Vec<String>>>,
    /// What its avionics do by themselves, once each, when the time comes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerDef>,
    /// RCS blocks, by part id, whose valves stay shut until an `rcs` activation on their stage
    /// opens them: a spacecraft's thrusters while a booster's fly the stack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rcs_shut: Vec<String>,
    /// What the abort does: steps in order, each `after` seconds from the abort.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abort: Vec<AbortStepDef>,
}

/// One step of an abort, written as `sequences` writes an activation. Parts that have left by then
/// are passed over, so one abort does for the whole flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbortStepDef {
    /// Seconds after the abort.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub after: f64,
    pub fire: Vec<String>,
}

/// Something a powered avionics unit does by itself when one of its conditions comes true: a
/// time, a propellant level or an acceleration limit, written as `sequences` writes activations.
/// It cannot let anything go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerDef {
    /// Seconds after liftoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    /// When the stage of part `of` has this share of its propellant left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propellant_below: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    /// When the proper acceleration passes this, m/s².
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceleration_above: Option<f64>,
    /// `<kind>:<part id>[:<setting>]`.
    pub fire: Vec<String>,
}

/// One part of a craft: a catalogue record (`template`) or a shape with components, and
/// the joint that holds it towards the root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CraftPartDef {
    /// Unique within the craft, without dots; kept when the craft splits.
    pub id: String,
    /// Display name; by default the template's id, or the part id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `"<part id>.<connector>"`: the part it hangs from, towards the root, and the node or
    /// surface of that part it is held at. The root has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Its own node or surface that `on` holds; its `bottom` node by default.
    #[serde(default = "bottom", skip_serializing_if = "is_bottom")]
    pub by: String,
    /// Where on the surface, when one end of the joint is a surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<SurfaceAtDef>,
    /// Degrees it is turned about the line the joint holds it along; for the root, about the way
    /// it `points`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f64,
    /// The root only: which way its own +Z points in the craft's frame, and so how everything held
    /// by it is turned there. Along +Z by default, as a rocket stands; see
    /// [`crate::craft::root_rotation`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<[f64; 3]>,
    /// Id of an `engines`, `pods`, `rcs` or `reaction_wheels` record: its model, nodes and
    /// what it does. Instead of `shape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeDef>,
    /// What it must do, instead of a `template` or a `shape`: a sizer makes the design
    /// ([`crate::sizing`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedural: Option<ProceduralDef>,
    /// Drawn instead of its shape or sized design, which stays what it holds and weighs, and
    /// what the air sees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelDef>,
    /// Inside another part and not drawn.
    #[serde(default, skip_serializing_if = "is_default")]
    pub unseen: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub structure: StructureDef,
    /// What it does, besides what its template does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentDef>,
    /// Nodes besides its shape's, in its own frame: where other parts are mounted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
    /// How it looks; a template's model looks as it is made.
    #[serde(default, skip_serializing_if = "is_default")]
    pub paint: PaintDef,
    /// Where its mass sits, in its own frame, if not where its shape puts it: what is inside a
    /// capsule sits low and to one side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_center: Option<[f64; 3]>,
}

/// A group of parts placed together (collection `assemblies`): a spacecraft, a booster with
/// its decoupler, an engine cluster. Placing one copies its parts into a craft, held at its
/// free connector (`CraftDef::place`), and nothing remembers the assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// As a craft's: exactly one, the assembly's root, is on nothing.
    pub parts: Vec<CraftPartDef>,
    /// `"<part id>.<connector>"`: where it is held when placed.
    pub free: String,
}

/// Where on a surface a joint holds: `u` around it and `v` along it, each 0 to 1 (see
/// `Geometry::surface_point`), on its `front` (outside) or `back` (inside, in a container),
/// `standoff` metres off it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceAtDef {
    pub u: f64,
    pub v: f64,
    #[serde(default, skip_serializing_if = "is_front")]
    pub side: Side,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub standoff: f64,
}

fn bottom() -> String {
    "bottom".into()
}

fn is_bottom(by: &str) -> bool {
    by == "bottom"
}

fn is_front(side: &Side) -> bool {
    *side == Side::Front
}

/// A craft described as a stack of stages: the shorthand the stage converter
/// (`crate::staged`) and the historical craft generator write parts from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedCraftDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    /// Stages in firing order: index 0 is the first stage (bottom of the stack).
    pub stages: Vec<StageDef>,
    /// As a craft's, with stage names in place of part ids.
    #[serde(default)]
    pub sequences: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageDef {
    pub name: String,
    /// Engine part names; repeat a name for clusters.
    #[serde(default)]
    pub engines: Vec<String>,
    /// Procedural tanks.
    #[serde(default)]
    pub tanks: Vec<TankDef>,
    #[serde(default)]
    pub fins: Option<FinDef>,
    /// Wings, tailplanes and tail fins.
    #[serde(default)]
    pub wings: Vec<WingDef>,
    /// Landing gear legs.
    #[serde(default)]
    pub gear: Vec<GearDef>,
    /// Reaction wheel part names.
    #[serde(default)]
    pub reaction_wheels: Vec<String>,
    /// RCS block part names; the blocks sit evenly around the stage's forward rim.
    #[serde(default)]
    pub rcs: Vec<String>,
    /// Crew pod part name, at the top of the stage.
    #[serde(default)]
    pub pod: Option<String>,
    /// A payload fairing around the stage, jettisoned by a sequence.
    #[serde(default)]
    pub fairing: Option<FairingDef>,
    /// Procedural shells stacked on top of the stage, bottom first: nose cones, size
    /// adapters, plain cylinders. Under a fairing they are the fairing's nose.
    #[serde(default)]
    pub shells: Vec<ShellDef>,
    /// Extra structural mass (interstage, thrust structure, fairings, avionics, payload),
    /// kg.
    #[serde(default)]
    pub structure_mass: f64,
    /// Outer diameter, m. Used for drag reference area, inertia and rendering.
    pub diameter: f64,
    /// Length of the hull (with its pod), m; the shells stack on top. Used for the centre
    /// of mass, inertia and rendering.
    pub length: f64,
    /// How the stage looks; no effect on physics.
    #[serde(default)]
    pub appearance: StageAppearance,
}

/// A payload fairing: two halves around a stage and the shells on it (its nose), jettisoned
/// in flight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairingDef {
    /// Both halves, kg.
    pub mass: f64,
}

/// A procedural shell: a body of revolution from one diameter to another, stacked on what
/// is below it. Closed to a point it is a nose cone, between two diameters an adapter, with
/// the same diameter at both ends a plain cylinder; a top left open at the top of the stack
/// is a flat face.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellDef {
    /// By default what it is: "Nose cone", "Adapter" or "Shell".
    #[serde(default)]
    pub name: Option<String>,
    /// m.
    pub length: f64,
    /// m; by default the diameter of what it stands on.
    #[serde(default)]
    pub bottom_diameter: Option<f64>,
    /// m; by default its bottom diameter. 0 closes it to a point.
    #[serde(default)]
    pub top_diameter: Option<f64>,
    /// How its side runs from one diameter to the other.
    #[serde(default)]
    pub profile: ShellProfile,
    /// kg, spread over its wall. Leave it out when the stage's `structure_mass` (or its
    /// fairing's mass) already counts it.
    #[serde(default)]
    pub mass: f64,
}

/// The curve of a shell's side. The curved ones leave the base parallel to the axis, so they
/// meet a cylinder below them smoothly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellProfile {
    /// Straight: a cylinder, frustum or cone.
    #[default]
    Conic,
    /// A circular arc (a tangent ogive), at least as long as the radius changes.
    Ogive,
    /// A quarter ellipse: blunt, a hemisphere when as long as its radius.
    Elliptical,
    /// A parabola.
    Parabolic,
    /// The von Kármán ogive (LD-Haack): the least wave drag for its length and diameter.
    Haack,
}

/// How a stage is drawn. The hull is built from the stage itself: its tanks are painted
/// barrels with weld seams, the structure above them is an interstage (dark, around the
/// next stage's engines) or more hull, and a top stage without tanks, engines or a pod is a
/// payload fairing. Colours are linear RGB.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StageAppearance {
    /// Hull paint; default white.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paint: Option<[f32; 3]>,
    /// A `liveries` record to paint the stage with instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub livery: Option<String>,
    /// The interstage; default black composite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interstage: Option<[f32; 3]>,
    /// Landing legs folded against the base of the stage (for show until legs are parts).
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub legs: u32,
    /// Grid fins stowed at the top of the stage.
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub grid_fins: u32,
    /// A cable raceway running up the side of the tanks.
    #[serde(skip_serializing_if = "is_false")]
    pub raceway: bool,
}

/// Where and what to spawn when a new game starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// A line for the main menu.
    #[serde(default)]
    pub description: String,
    /// Seconds since J2000.
    pub start_time: f64,
    pub craft: String,
    pub start: StartDef,
    /// How many of the craft's sequences have already fired when it starts, with the throttle
    /// shut: a lander on the ground has already used the one that lights its descent engine.
    #[serde(default)]
    pub sequences_done: u32,
    /// The player's vessel starts with RCS on.
    #[serde(default)]
    pub rcs: bool,
    /// Other vessels in the game from the start (stations, targets).
    #[serde(default)]
    pub others: Vec<VesselStartDef>,
    /// What the player's vessel starts targeting: a body id or another vessel's name.
    #[serde(default)]
    pub target: Option<String>,
    /// What there is to do (see [`crate::mission`]). The mission is passed when every goal is
    /// met.
    #[serde(default)]
    pub goals: Vec<GoalDef>,
    /// Ends the mission as failed when one holds; its `text` says why.
    #[serde(default)]
    pub fail: Vec<GoalDef>,
    /// The goals are done one after another, in the order listed, not in any order.
    #[serde(default)]
    pub ordered: bool,
}

/// A vessel placed at the start of a game besides the player's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VesselStartDef {
    pub craft: String,
    /// Overrides the craft's name.
    #[serde(default)]
    pub name: Option<String>,
    pub start: StartDef,
    /// Activation sequences it has already fired, as the player's vessel can have.
    #[serde(default)]
    pub sequences_done: u32,
    /// Starts with SAS holding its attitude and RCS on, as a vessel with someone aboard waiting
    /// to be docked with: what hits it does not leave it tumbling.
    #[serde(default)]
    pub holds_attitude: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StartDef {
    /// On the pad of a `launch_sites` record.
    LaunchSite(LaunchSiteStartDef),
    /// At the start of a `launch_sites` record's runway, lined up along it, on its wheels.
    Runway(LaunchSiteStartDef),
    /// Already in orbit. Angles in degrees relative to the body's equator; altitudes in m.
    Orbit(OrbitStartDef),
    /// Flying through the air, as if dropped from a carrier aircraft.
    Flight(FlightStartDef),
}

/// Nose along the flight path, feet towards the ground. Degrees and metres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightStartDef {
    pub body: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Above sea level.
    pub altitude: f64,
    /// Clockwise from north.
    #[serde(default)]
    pub heading: f64,
    /// Airspeed, m/s.
    pub speed: f64,
    /// Climb angle of the flight path.
    #[serde(default)]
    pub climb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSiteStartDef {
    /// Id of a `launch_sites` record.
    pub site: String,
}

/// A dish on a body that talks to vessels (collection `ground_stations`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundStationDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    pub body: String,
    /// Degrees.
    pub latitude: f64,
    pub longitude: f64,
    /// The dish's diameter, m.
    pub dish: f64,
    /// Its receiver's noise temperature, K.
    #[serde(default = "default_station_noise")]
    pub noise: f64,
}

fn default_station_noise() -> f64 {
    50.0
}

/// A place to launch from (collection `launch_sites`). Its pad and roads are built into
/// the terrain, so they exist for collision as well as rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSiteDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    pub body: String,
    /// Degrees. Vessels stand at this point, on the pad deck if there is one.
    pub latitude: f64,
    pub longitude: f64,
    /// Direction of the site layout's forward axis, degrees clockwise from north.
    #[serde(default)]
    pub heading: f64,
    #[serde(default)]
    pub pad: Option<PadDef>,
    #[serde(default)]
    pub roads: Vec<RoadDef>,
    #[serde(default)]
    pub runway: Option<RunwayDef>,
}

impl LaunchSiteDef {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// A level concrete deck on an embankment, centred on the site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadDef {
    /// Deck length along the site heading and width across it, m.
    pub size: [f64; 2],
    #[serde(default)]
    pub corner_radius: f64,
    /// Deck height above the mean radius (sea level), m. Defaults to 1 m above the ground
    /// at the centre.
    #[serde(default)]
    pub elevation: Option<f64>,
    /// Embankment steepness, rise over run.
    #[serde(default = "default_slope")]
    pub slope: f64,
    /// Height of the launch mount above the deck, m: vehicles stand on it with their
    /// engines over the flame trench.
    #[serde(default)]
    pub mount_height: f64,
}

/// A level, paved runway centred on the site and running along its heading. Vessels start
/// at its near end (`start: { type: "Runway", site }`), facing along it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayDef {
    /// Length along the heading and width across it, m.
    pub size: [f64; 2],
    /// Surface height above the mean radius (sea level), m. Defaults to 1 m above the
    /// ground at its centre.
    #[serde(default)]
    pub elevation: Option<f64>,
}

/// A paved road, level across its width, following the ground along its length.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadDef {
    /// Centre line as `[right, forward]` metres from the site centre in the site frame.
    pub points: Vec<[f64; 2]>,
    /// m.
    pub width: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitStartDef {
    pub body: String,
    pub periapsis_altitude: f64,
    pub apoapsis_altitude: f64,
    #[serde(default)]
    pub inclination: f64,
    #[serde(default)]
    pub longitude_of_ascending_node: f64,
    /// Start in the orbital plane of this body (one that orbits `body`, like the Moon)
    /// instead of `inclination` and `longitude_of_ascending_node`.
    #[serde(default)]
    pub plane: Option<String>,
    #[serde(default)]
    pub argument_of_periapsis: f64,
    #[serde(default)]
    pub mean_anomaly: f64,
    /// Turning at the start, degrees per second about pitch, yaw and roll (vessel X, Y and
    /// Z).
    #[serde(default)]
    pub spin: [f64; 3],
}

fn one() -> f64 {
    1.0
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn default_slope() -> f64 {
    0.4
}

fn room_temperature() -> f64 {
    293.0
}

/// Bare metal holds liquid oxygen for a launch (it frosts over) but not liquid hydrogen.
fn uninsulated() -> f64 {
    77.0
}

fn default_utilization() -> f64 {
    0.97
}

fn default_safety_factor() -> f64 {
    1.25
}

fn default_design_load() -> f64 {
    2.5
}

fn default_fin_count() -> u32 {
    4
}

/// A node of the technology tree (collection `tech_nodes`): a step in rocketry,
/// materials, electronics, ... reached in `year`. Records name the node that unlocks them
/// in their `tech`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechNodeDef {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Year its technology became available, AD.
    #[serde(default)]
    pub year: Option<i32>,
    /// Last year of the span it covers, if it covers one.
    #[serde(default)]
    pub year_end: Option<i32>,
    #[serde(default)]
    pub category: Option<String>,
    /// Research cost in the source tree's units.
    #[serde(default)]
    pub cost: f64,
    /// Nodes it builds on: all of them are needed, or any one with `any_parent`.
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub any_parent: bool,
    /// Where it is drawn in the tree, in the source tree's layout units.
    #[serde(default)]
    pub position: Option<[f64; 2]>,
    /// Parts-list entries it unlocks besides the records whose `tech` names it: the procedural
    /// parts, and parts whose records say nothing of a tree (`tank`, `pod:C-1 Capsule`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// How big it lets procedural parts be made, as Procedural Parts' tech limits: a career may
    /// build to the biggest of the nodes it has unlocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizes: Option<SizeLimitsDef>,
}

/// The biggest a node lets procedural parts be made (tanks, structural tubes, nose cones), m. A
/// size it does not give is left to the other nodes.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeLimitsDef {
    /// The widest across: a diameter, a cone's top or bottom.
    #[serde(default)]
    pub diameter: Option<f64>,
    /// The longest.
    #[serde(default)]
    pub length: Option<f64>,
}

/// A tech tree a game can be played on (collection `tech_trees`): the `tech_nodes` reached from its
/// roots. The rules name the one a game uses ([`RulesDef::tech_tree`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TechTreeDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The nodes a science game starts with.
    pub roots: Vec<String>,
    /// How it is drawn.
    #[serde(default)]
    pub layout: TreeLayout,
    /// The funds a career on it starts with.
    #[serde(default)]
    pub funds: f64,
}

/// A facility a career launches or builds with (collection `facilities`), upgraded a level at a time
/// with money ([`crate::career::Career::upgrade`]). A sandbox has none of its limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// What it allows at each level, the first what a career starts with.
    pub levels: Vec<FacilityLevelDef>,
}

impl FacilityDef {
    /// What the player reads.
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// What a facility allows at one level, and what reaching it costs. A limit it does not give it
/// does not set.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityLevelDef {
    /// Funds to upgrade to it.
    #[serde(default)]
    pub price: f64,
    /// The most a rocket may weigh as it leaves the pad, kg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f64>,
    /// The tallest craft that can be built, m (a plane's length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// The widest, m.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// The most parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<u32>,
}

impl FacilityLevelDef {
    /// Its limits as the player reads them: "180 t", "50 m tall, 10 m wide, 100 parts".
    pub fn describe(&self) -> String {
        let mut out = Vec::new();
        if let Some(kg) = self.mass {
            out.push(format!("{} t", crate::career::money(kg / 1000.0)));
        }
        if let Some(m) = self.height {
            out.push(format!("{m:.0} m tall"));
        }
        if let Some(m) = self.width {
            out.push(format!("{m:.0} m wide"));
        }
        if let Some(parts) = self.parts {
            out.push(format!("{parts} parts"));
        } else if self.height.is_some() || self.width.is_some() {
            out.push("any number of parts".into());
        }
        if out.is_empty() {
            "no limits".into()
        } else {
            out.join(", ")
        }
    }
}

/// How a tech tree is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeLayout {
    /// Where each node's `position` says: [column, row].
    #[default]
    Positions,
    /// A column for each year, oldest on the left, and a band of rows for each `category`.
    Years,
}

/// A battery technology (collection `batteries`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryDef {
    pub id: String,
    #[serde(default)]
    pub chemistry: String,
    #[serde(default)]
    pub rechargeable: bool,
    /// Wh/kg and Wh/L for a whole battery.
    pub specific_energy: f64,
    #[serde(default)]
    pub energy_density: f64,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A level of guidance and control electronics (collection `avionics`): how much mass and
/// power it takes to control a vessel of a given mass, as RP-1's procedural avionics
/// (mass = factor · controlled_mass^exponent + constant, in kg and tonnes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvionicsDef {
    pub id: String,
    /// `Near-Earth`, `Deep Space` or `Science`.
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub mass_factor: f64,
    #[serde(default)]
    pub mass_exponent: f64,
    #[serde(default)]
    pub mass_constant: f64,
    #[serde(default)]
    pub power_factor: f64,
    #[serde(default)]
    pub power_exponent: f64,
    #[serde(default)]
    pub power_constant: f64,
    /// Works beyond Earth's neighbourhood.
    #[serde(default)]
    pub interplanetary: bool,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A solar cell technology level (collection `solar_panels`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolarPanelDef {
    pub id: String,
    #[serde(default)]
    pub level: u32,
    /// Electric power in full sunlight at 1 AU, W/m², and panel mass, kg/m².
    pub power_per_area: f64,
    pub mass_per_area: f64,
    #[serde(default)]
    pub retractable: bool,
    #[serde(default)]
    pub tracking: bool,
    /// Output share over days in service: `[days, share]` points.
    #[serde(default)]
    pub degradation: Vec<[f64; 2]>,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A jet, ramjet or propeller engine configuration (collection `air_breathing_engines`),
/// with Advanced Jet Engine's cycle parameters, for when the game flies them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirBreathingEngineDef {
    pub id: String,
    pub family: String,
    /// `turbine`, `ramjet` or `propeller`.
    #[serde(rename = "type")]
    pub engine_type: String,
    /// Dry mass, kg.
    pub mass: f64,
    #[serde(default)]
    pub propellants: BTreeMap<String, f64>,
    /// AJE parameters as given (`dryThrust`, `wetThrust` in kN, `BPR`, `CPR`, `TIT`, ...).
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamValue>,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A parameter value as a source config gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Number(f64),
    Flag(bool),
    Text(String),
}

/// An engine family (collection `engine_families`): the configurations of one engine and
/// the part that models them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineFamilyDef {
    pub id: String,
    /// Ids of its `engines` records, oldest first as the source lists them.
    #[serde(default)]
    pub configs: Vec<String>,
    #[serde(default)]
    pub default_config: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(flatten)]
    pub catalog: CatalogInfo,
}

/// A 64-bit FNV-1a hash: the same on every machine and every run.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
