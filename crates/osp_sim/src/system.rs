//! The hierarchy of celestial bodies and conversions between their reference frames.
//!
//! Every state in the simulation is relative to some body (its *frame*), always in the same
//! inertial ecliptic axes. Converting between frames walks the tree to the lowest common
//! ancestor, so a state near the Moon is never routed through heliocentric coordinates:
//! precision depends on the distances actually involved, not on distance from the Sun.

use thiserror::Error;

use crate::{
    body::{Body, BodyId, Rotation},
    data::SystemDef,
    orbit::{Elements, Orbit, StateVector},
    terrain::Terrain,
    time::SimTime,
};

#[derive(Debug, Error)]
pub enum SystemError {
    #[error("body `{body}` references unknown parent `{parent}`")]
    UnknownParent { body: String, parent: String },
    #[error("body `{0}` has a parent but no orbit")]
    MissingOrbit(String),
    #[error("system must have exactly one root body, found {0}")]
    RootCount(usize),
    #[error("duplicate body id `{0}`")]
    Duplicate(String),
    #[error("parent cycle involving `{0}`")]
    Cycle(String),
}

#[derive(Debug, Clone)]
pub struct SolarSystem {
    bodies: Vec<Body>,
}

/// Order definitions so every parent precedes its children.
fn parents_first(defs: &[crate::data::BodyDef]) -> Result<Vec<&crate::data::BodyDef>, SystemError> {
    let mut seen = std::collections::HashSet::new();
    for d in defs {
        if !seen.insert(d.id.as_str()) {
            return Err(SystemError::Duplicate(d.id.clone()));
        }
    }
    let mut placed: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut ordered = Vec::with_capacity(defs.len());
    let mut remaining: Vec<&crate::data::BodyDef> = defs.iter().collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        remaining.retain(|d| match &d.parent {
            Some(parent) if !placed.contains(parent.as_str()) => true,
            _ => {
                placed.insert(d.id.as_str());
                ordered.push(*d);
                false
            }
        });
        if remaining.len() == before {
            let stuck = remaining[0];
            let parent = stuck.parent.clone().unwrap_or_default();
            return Err(if seen.contains(parent.as_str()) {
                SystemError::Cycle(stuck.id.clone())
            } else {
                SystemError::UnknownParent {
                    body: stuck.id.clone(),
                    parent,
                }
            });
        }
    }
    Ok(ordered)
}

impl SolarSystem {
    pub fn from_def(def: &SystemDef) -> Result<Self, SystemError> {
        let ordered = parents_first(&def.bodies)?;
        let mut bodies: Vec<Body> = Vec::with_capacity(ordered.len());
        for (index, body_def) in ordered.into_iter().enumerate() {
            let id = BodyId(index);
            let parent = match &body_def.parent {
                None => None,
                Some(name) => Some(
                    bodies
                        .iter()
                        .find(|b| &b.name == name)
                        .map(|b| b.id)
                        .ok_or_else(|| SystemError::UnknownParent {
                            body: body_def.id.clone(),
                            parent: name.clone(),
                        })?,
                ),
            };
            let (orbit, soi, depth) = match parent {
                None => (None, f64::INFINITY, 0),
                Some(parent_id) => {
                    let parent_body = &bodies[parent_id.0];
                    let o = body_def
                        .orbit
                        .as_ref()
                        .ok_or_else(|| SystemError::MissingOrbit(body_def.id.clone()))?;
                    let elements = Elements {
                        semi_major_axis: o.semi_major_axis,
                        eccentricity: o.eccentricity,
                        inclination: o.inclination.to_radians(),
                        longitude_of_ascending_node: o.longitude_of_ascending_node.to_radians(),
                        argument_of_periapsis: o.argument_of_periapsis.to_radians(),
                        true_anomaly: 0.0,
                    };
                    // Relative two-body motion uses the combined gravitational parameter.
                    let orbit = Orbit::from_elements(
                        parent_body.gm + body_def.gm,
                        &elements,
                        o.mean_anomaly_at_epoch.to_radians(),
                        SimTime::from_seconds(o.epoch),
                    );
                    // Laplace sphere of influence.
                    let soi = o.semi_major_axis * (body_def.gm / parent_body.gm).powf(0.4);
                    (Some(orbit), soi, parent_body.depth + 1)
                }
            };
            if let Some(parent_id) = parent {
                bodies[parent_id.0].children.push(id);
            }
            bodies.push(Body {
                id,
                name: body_def.id.clone(),
                parent,
                children: Vec::new(),
                depth,
                gm: body_def.gm,
                radius: body_def.radius,
                max_terrain_height: body_def.max_terrain_height,
                terrain: Default::default(),
                rotation: Rotation::from_def(&body_def.rotation),
                atmosphere: body_def.atmosphere.clone(),
                orbit,
                soi,
                color: body_def.color,
            });
        }
        let roots = bodies.iter().filter(|b| b.parent.is_none()).count();
        if roots != 1 {
            return Err(SystemError::RootCount(roots));
        }
        Ok(Self { bodies })
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id.0]
    }

    /// Replace a body's terrain. Raises `max_terrain_height` to cover any earthworks.
    pub fn set_terrain(&mut self, id: BodyId, terrain: Terrain) {
        let body = &mut self.bodies[id.0];
        body.max_terrain_height = body.max_terrain_height.max(terrain.max_feature_height());
        body.terrain = std::sync::Arc::new(terrain);
    }

    pub fn root(&self) -> BodyId {
        BodyId(0)
    }

    pub fn find(&self, name: &str) -> Option<BodyId> {
        self.bodies.iter().find(|b| b.name == name).map(|b| b.id)
    }

    /// State of a body relative to its parent. Zero for the root.
    pub fn local_state(&self, id: BodyId, time: SimTime) -> StateVector {
        match &self.body(id).orbit {
            Some(orbit) => orbit.state_at(time),
            None => StateVector::default(),
        }
    }

    /// State of body `of` relative to body `frame`, computed through their lowest common
    /// ancestor.
    pub fn relative_state(&self, of: BodyId, frame: BodyId, time: SimTime) -> StateVector {
        let (mut a, mut b) = (of, frame);
        let (mut sa, mut sb) = (StateVector::default(), StateVector::default());
        while a != b {
            let (da, db) = (self.body(a).depth, self.body(b).depth);
            if da >= db {
                sa = sa + self.local_state(a, time);
                a = self.body(a).parent.expect("depth > 0 has a parent");
            } else {
                sb = sb + self.local_state(b, time);
                b = self.body(b).parent.expect("depth > 0 has a parent");
            }
        }
        sa - sb
    }

    /// Re-express a state given relative to `from` as relative to `to`.
    pub fn transform(
        &self,
        state: StateVector,
        from: BodyId,
        to: BodyId,
        time: SimTime,
    ) -> StateVector {
        if from == to {
            state
        } else {
            state + self.relative_state(from, to, time)
        }
    }

    /// The body whose sphere of influence actually contains `state` (given relative to
    /// `frame`), and the state relative to it.
    pub fn locate(
        &self,
        mut frame: BodyId,
        mut state: StateVector,
        time: SimTime,
    ) -> (BodyId, StateVector) {
        // Up while outside the current SOI.
        while state.position.length() > self.body(frame).soi {
            let Some(parent) = self.body(frame).parent else {
                break;
            };
            state = state + self.local_state(frame, time);
            frame = parent;
        }
        // Down while inside a child's SOI.
        'descend: loop {
            for &child in &self.body(frame).children {
                let relative = state - self.local_state(child, time);
                if relative.position.length() < self.body(child).soi {
                    frame = child;
                    state = relative;
                    continue 'descend;
                }
            }
            break;
        }
        (frame, state)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use glam::DVec3;

    use super::*;

    pub fn rss() -> SolarSystem {
        let bodies = crate::test_data::records(include_str!(
            "../../../assets/core/bodies/solar_system.json5"
        ));
        SolarSystem::from_def(&SystemDef { bodies }).unwrap()
    }

    #[test]
    fn loads_real_solar_system() {
        let system = rss();
        let earth = system.body(system.find("Earth").unwrap());
        // Earth's SOI is ~924 000 km.
        assert!((earth.soi - 9.24e8).abs() < 0.01e8, "{}", earth.soi);
        let moon = system.find("Moon").unwrap();
        let dist = system.local_state(moon, SimTime::J2000).position.length();
        assert!((3.5e8..4.1e8).contains(&dist));
        // Sidereal day.
        assert!((earth.rotation.sidereal_period() - 86_164.09).abs() < 0.01);
    }

    #[test]
    fn earth_orientation_matches_iau() {
        let system = rss();
        let earth = system.body(system.find("Earth").unwrap());
        // Celestial north pole in ecliptic coordinates is (0, sin ε, cos ε).
        let eps = crate::constants::OBLIQUITY_J2000;
        assert!((earth.pole() - DVec3::new(0.0, eps.sin(), eps.cos())).length() < 1e-9);
        // At J2000 the Greenwich meridian is ~280.46° east of the equinox (GMST).
        let ecliptic = earth.orientation(SimTime::J2000) * DVec3::X;
        let equatorial = glam::DQuat::from_rotation_x(eps) * ecliptic;
        let ra = equatorial
            .y
            .atan2(equatorial.x)
            .to_degrees()
            .rem_euclid(360.0);
        assert!((ra - 280.46).abs() < 0.5, "{ra}");
    }

    #[test]
    fn relative_states_compose() {
        let system = rss();
        let t = SimTime::from_seconds(1.234e8);
        let (sun, earth, moon, mars) = (
            system.find("Sun").unwrap(),
            system.find("Earth").unwrap(),
            system.find("Moon").unwrap(),
            system.find("Mars").unwrap(),
        );
        let moon_sun = system.relative_state(moon, sun, t);
        let chained = system.relative_state(moon, earth, t) + system.relative_state(earth, sun, t);
        assert!((moon_sun.position - chained.position).length() < 1e-3);
        let moon_mars = system.relative_state(moon, mars, t);
        let mars_moon = system.relative_state(mars, moon, t);
        assert!((moon_mars.position + mars_moon.position).length() < 1e-3);
        assert_eq!(system.relative_state(moon, moon, t), StateVector::default());
    }

    #[test]
    fn moon_frame_math_stays_precise() {
        // A point 1 m above the Moon's surface, converted to the Earth frame and back, must
        // come back to well under a millimetre: the heliocentric 1.5e11 m never enters.
        let system = rss();
        let (earth, moon) = (system.find("Earth").unwrap(), system.find("Moon").unwrap());
        let t = SimTime::from_seconds(9.87e8);
        let local = StateVector::new(
            DVec3::new(1_737_401.0, 0.25, -0.5),
            DVec3::new(1.0, 2.0, 3.0),
        );
        let in_earth = system.transform(local, moon, earth, t);
        let back = system.transform(in_earth, earth, moon, t);
        assert!((back.position - local.position).length() < 1e-6);
    }

    #[test]
    fn locate_finds_the_right_soi() {
        let system = rss();
        let (sun, earth, moon) = (
            system.find("Sun").unwrap(),
            system.find("Earth").unwrap(),
            system.find("Moon").unwrap(),
        );
        let t = SimTime::J2000;
        let moon_pos = system.relative_state(moon, sun, t);
        let near_moon = moon_pos + StateVector::new(DVec3::new(2.0e6, 0.0, 0.0), DVec3::ZERO);
        let (frame, local) = system.locate(sun, near_moon, t);
        assert_eq!(frame, moon);
        assert!((local.position - DVec3::new(2.0e6, 0.0, 0.0)).length() < 1e-3);

        let leo = StateVector::new(DVec3::new(7e6, 0.0, 0.0), DVec3::ZERO);
        assert_eq!(system.locate(earth, leo, t).0, earth);
        let far = StateVector::new(DVec3::new(2e9, 0.0, 0.0), DVec3::ZERO);
        assert_eq!(system.locate(earth, far, t).0, sun);
    }
}
