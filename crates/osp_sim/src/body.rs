use std::f64::consts::{FRAC_PI_2, TAU};

use std::sync::Arc;

use glam::{DQuat, DVec3};

use crate::{
    atmosphere::Atmosphere,
    constants::{OBLIQUITY_J2000, SECONDS_PER_DAY},
    data::RotationDef,
    orbit::Orbit,
    terrain::Terrain,
    time::SimTime,
};

/// Clearance above the highest terrain below which vessels are integrated rather than
/// coasting analytically, so they can touch down.
pub const TERRAIN_MARGIN: f64 = 2_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId(pub usize);

/// Orientation of a body over time (IAU model, expressed in ecliptic axes).
#[derive(Debug, Clone, Copy)]
pub struct Rotation {
    /// Rotates body-equatorial axes (Z = north pole, X = node of the equator on the ICRF
    /// equator) into the inertial ecliptic frame.
    pole_frame: DQuat,
    /// Prime meridian angle at J2000, rad.
    prime_meridian: f64,
    /// rad/s, negative for retrograde.
    rate: f64,
}

impl Rotation {
    pub fn from_def(def: &RotationDef) -> Self {
        let ra = def.pole_ra.to_radians();
        let dec = def.pole_dec.to_radians();
        // ICRF -> ecliptic is a rotation about +X by the obliquity.
        let ecliptic_from_icrf = DQuat::from_rotation_x(-OBLIQUITY_J2000);
        let icrf_from_equatorial =
            DQuat::from_rotation_z(FRAC_PI_2 + ra) * DQuat::from_rotation_x(FRAC_PI_2 - dec);
        Self {
            pole_frame: ecliptic_from_icrf * icrf_from_equatorial,
            prime_meridian: def.prime_meridian.to_radians(),
            rate: def.rate.to_radians() / SECONDS_PER_DAY,
        }
    }

    /// Body-fixed (Z north, X prime meridian) to inertial at `time`.
    pub fn at(&self, time: SimTime) -> DQuat {
        let angle =
            (self.prime_meridian + self.rate * time.seconds_since(SimTime::J2000)).rem_euclid(TAU);
        self.pole_frame * DQuat::from_rotation_z(angle)
    }

    /// Body-equatorial frame without the daily spin (useful for orbit setup and display).
    pub fn equatorial_frame(&self) -> DQuat {
        self.pole_frame
    }

    pub fn pole(&self) -> DVec3 {
        self.pole_frame * DVec3::Z
    }

    /// Angular velocity, inertial frame, rad/s.
    pub fn angular_velocity(&self) -> DVec3 {
        self.pole() * self.rate
    }

    pub fn sidereal_period(&self) -> f64 {
        TAU / self.rate.abs()
    }
}

#[derive(Debug, Clone)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub parent: Option<BodyId>,
    pub children: Vec<BodyId>,
    /// Depth in the hierarchy (root = 0).
    pub depth: usize,
    pub gm: f64,
    pub radius: f64,
    pub max_terrain_height: f64,
    /// Ground shape; shared so renderers can sample it from worker threads.
    pub terrain: Arc<Terrain>,
    pub rotation: Rotation,
    pub atmosphere: Option<Atmosphere>,
    /// Orbit around `parent`. `None` for the root body.
    pub orbit: Option<Orbit>,
    /// Sphere of influence radius; infinite for the root body.
    pub soi: f64,
    pub color: [f32; 3],
}

impl Body {
    /// Body-fixed to inertial orientation at `time`.
    pub fn orientation(&self, time: SimTime) -> DQuat {
        self.rotation.at(time)
    }

    pub fn angular_velocity(&self) -> DVec3 {
        self.rotation.angular_velocity()
    }

    pub fn pole(&self) -> DVec3 {
        self.rotation.pole()
    }

    /// Unit vector in body-fixed coordinates for a latitude/longitude in radians.
    pub fn surface_normal_fixed(latitude: f64, longitude: f64) -> DVec3 {
        DVec3::new(
            latitude.cos() * longitude.cos(),
            latitude.cos() * longitude.sin(),
            latitude.sin(),
        )
    }

    /// (latitude, longitude) in radians of an inertial, body-relative position.
    pub fn lat_lon(&self, position: DVec3, time: SimTime) -> (f64, f64) {
        let fixed = self.orientation(time).inverse() * position;
        let lat = (fixed.z / fixed.length()).clamp(-1.0, 1.0).asin();
        (lat, fixed.y.atan2(fixed.x))
    }

    /// Velocity of the ground / co-rotating air at a body-relative position.
    pub fn surface_velocity(&self, position: DVec3) -> DVec3 {
        self.angular_velocity().cross(position)
    }

    /// Height of the ground (or sea surface) above the mean radius in a body-fixed unit
    /// direction. Shared by collision and rendering.
    pub fn terrain_height(&self, fixed_direction: DVec3) -> f64 {
        self.terrain.height(fixed_direction)
    }

    pub fn atmosphere_height(&self) -> f64 {
        self.atmosphere.as_ref().map_or(0.0, |a| a.height)
    }

    pub fn has_atmosphere_at(&self, altitude: f64) -> bool {
        self.atmosphere
            .as_ref()
            .is_some_and(|a| altitude < a.height)
    }

    /// Radius below which a vessel must be integrated (atmosphere or terrain proximity).
    /// Above it, gravity is the only force and motion is an exact conic.
    pub fn coast_floor(&self) -> f64 {
        self.radius
            + self
                .atmosphere_height()
                .max(self.max_terrain_height + TERRAIN_MARGIN)
    }
}
