//! What entering an atmosphere looks like, as far as the sim can say: how hot the air gets, and the
//! shape of the vessel as the airflow meets it.
//!
//! The heating is Sutton and Graves's formula for a blunt nose. The shape is a small map looking
//! along the flow: at each point across it, how far upstream and downstream the vessel's hull
//! reaches, and whether it is there at all. The game draws the shock and the wake from it, so a
//! capsule and a long rocket at an angle each get their own without any of it being said per part.

use glam::{DQuat, DVec3};

/// Texels along each side of the hull map (`HULL_MAP_SIZE` in the game).
pub const HULL_MAP_SIZE: usize = 48;
/// Sutton-Graves constant for air, kg^0.5/m, with a nose radius in metres and heating in W/m².
const SUTTON_GRAVES: f64 = 1.74153e-4;

/// Stagnation-point heating, W/m², of a nose of this radius in air of this density.
pub fn stagnation_heating(density: f64, speed: f64, nose_radius: f64) -> f64 {
    SUTTON_GRAVES * (density / nose_radius.max(0.05)).sqrt() * speed.powi(3)
}

/// How strong the show is, 0 to 1, at this heating: a dim glow from about 30 kW/m², and all there
/// is around 3 MW/m², which is about the peak coming back from the Moon. On a log scale, as the
/// light of a gas that hot climbs very steeply.
pub fn glow_level(heating: f64) -> f64 {
    let t = ((heating.max(1.0).log10() - 4.5) / 2.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The vessel along the flow.
pub struct HullMap {
    /// Half the width of the map, m: well past the hull, to leave room for what is blurred from it.
    pub half_width: f64,
    /// How far the hull reaches from the flow's axis through the map's centre, m.
    pub reach: f64,
    /// Row-major. Front and back along the flow relative to the map's centre (m), each times the
    /// coverage so they average properly into coarser levels, and the coverage.
    pub texels: Vec<[f32; 3]>,
    /// How far the hull reaches, upstream and downstream of the map's centre, m.
    pub front: f64,
    pub back: f64,
}

/// A rotation that takes `flow` to +Z, so the map's x and y are across it.
pub fn flow_frame(flow: DVec3) -> DQuat {
    DQuat::from_rotation_arc(flow.normalize(), DVec3::Z)
}

/// Build the map from the hull as rings: their middles relative to the map's centre and how far
/// they reach, in the vessel's axes, bottom up; `axis` is the rings' axis.
pub fn hull_map(rings: &[(DVec3, f64)], axis: DVec3, flow: DVec3) -> HullMap {
    let n = HULL_MAP_SIZE;
    let frame = flow_frame(flow);
    let axis = frame * axis.normalize_or(DVec3::Z);
    // Every ring as a circle in the flow's frame.
    let rings: Vec<(DVec3, f64)> = rings.iter().map(|&(c, r)| (frame * c, r)).collect();
    let (mut reach, mut front, mut back) = (1.0f64, f64::MAX, f64::MIN);
    let edge = (1.0 - axis.z * axis.z).max(0.0).sqrt();
    for &(c, r) in &rings {
        reach = reach.max(c.x.hypot(c.y) + r);
        front = front.min(c.z - r * edge);
        back = back.max(c.z + r * edge);
    }
    if rings.is_empty() {
        (front, back) = (0.0, 0.0);
    }
    let half_width = reach * 2.4;
    let texel = 2.0 * half_width / n as f64;
    let mut texels = vec![[0.0f32; 3]; n * n];
    for (j, row) in texels.chunks_mut(n).enumerate() {
        let y = (j as f64 + 0.5) * texel - half_width;
        for (i, out) in row.iter_mut().enumerate() {
            let x = (i as f64 + 0.5) * texel - half_width;
            let (mut near, mut far) = (f64::MAX, f64::MIN);
            for pair in rings.windows(2) {
                let (a, ra) = pair[0];
                let (b, rb) = pair[1];
                // The closest point of this piece of the hull to the line along the flow here,
                // taken as a round cone across the map. Good enough for a glow.
                let (ax, ay) = (a.x - x, a.y - y);
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let length2 = dx * dx + dy * dy;
                let closest = if length2 > 1e-9 {
                    (-(ax * dx + ay * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                // Its ends count too: along the flow they are all that sets how far it reaches.
                for s in [0.0, closest, 1.0] {
                    let (px, py) = (ax + dx * s, ay + dy * s);
                    let radius = ra + (rb - ra) * s;
                    let off2 = px * px + py * py;
                    if off2 >= radius * radius {
                        continue;
                    }
                    let middle = a.z + (b.z - a.z) * s;
                    // A round section seen at an angle: its depth along the flow at this offset.
                    let half = (radius * radius - off2).sqrt() * edge;
                    near = near.min(middle - half);
                    far = far.max(middle + half);
                }
            }
            if far >= near {
                *out = [near as f32, far as f32, 1.0];
            }
        }
    }
    HullMap {
        half_width,
        reach,
        texels,
        front,
        back,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cylinder along Z, 2 m in radius and 10 long, with the origin in the middle.
    fn cylinder() -> Vec<(DVec3, f64)> {
        (0..=10)
            .map(|k| (DVec3::new(0.0, 0.0, k as f64 - 5.0), 2.0))
            .collect()
    }

    fn at(map: &HullMap, x: f64, y: f64) -> [f32; 3] {
        let n = HULL_MAP_SIZE as f64;
        let texel = 2.0 * map.half_width / n;
        let i = ((x + map.half_width) / texel) as usize;
        let j = ((y + map.half_width) / texel) as usize;
        map.texels[j * HULL_MAP_SIZE + i]
    }

    #[test]
    fn apollo_comes_in_hot_and_a_low_orbit_decay_gently() {
        // Peak heating of a lunar return, at about 60 km and 10 km/s, on a 4.7 m nose: about 3 to 5
        // MW/m² in the Apollo records.
        let q = stagnation_heating(3e-4, 10_000.0, 4.7);
        assert!(q > 1e6 && q < 6e6, "{q}");
        assert!(glow_level(q) > 0.8, "{}", glow_level(q));
        // A plane at Mach 3 in thin air: nothing to see.
        assert_eq!(glow_level(stagnation_heating(0.02, 900.0, 1.0)), 0.0);
        // Faint high up, on the way in from orbit.
        let high = glow_level(stagnation_heating(1e-6, 7800.0, 2.0));
        assert!(high < 0.2, "{high}");
    }

    #[test]
    fn a_vessel_flying_nose_first_shows_a_disc() {
        // Flow along its axis, from above: the front is at the top and the back at the bottom.
        let map = hull_map(&cylinder(), DVec3::Z, -DVec3::Z);
        let centre = at(&map, 0.0, 0.0);
        assert_eq!(centre[2], 1.0);
        // Upstream is the smaller coordinate along the flow.
        assert!(centre[0] < centre[1], "{centre:?}");
        assert!((centre[1] - centre[0] - 10.0).abs() < 0.01, "{centre:?}");
        // Outside its radius, nothing.
        assert_eq!(at(&map, 2.5, 0.0)[2], 0.0);
        assert_eq!(at(&map, 1.5, 0.0)[2], 1.0);
    }

    #[test]
    fn a_vessel_flying_sideways_shows_its_length() {
        let map = hull_map(&cylinder(), DVec3::Z, DVec3::X);
        let (mut along, mut across) = (0, 0);
        let n = HULL_MAP_SIZE;
        for j in 0..n {
            if map.texels[j * n + n / 2][2] > 0.0 {
                along += 1;
            }
            if map.texels[n / 2 * n + j][2] > 0.0 {
                across += 1;
            }
        }
        // Long one way and narrow the other, in whichever axes the frame chose.
        assert!(
            along.max(across) > 2 * along.min(across),
            "{along} {across}"
        );
    }

    #[test]
    fn the_map_holds_all_of_the_vessel() {
        let map = hull_map(&cylinder(), DVec3::Z, DVec3::new(1.0, 0.0, 1.0));
        let n = HULL_MAP_SIZE;
        for k in 0..n {
            for texel in [
                map.texels[k],
                map.texels[(n - 1) * n + k],
                map.texels[k * n],
                map.texels[k * n + n - 1],
            ] {
                assert_eq!(texel[2], 0.0, "the hull reaches the map's edge");
            }
        }
        assert!(map.front < map.back);
    }
}
