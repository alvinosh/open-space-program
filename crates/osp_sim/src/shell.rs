//! Procedural shells: bodies of revolution stacked on a stage, from one radius to another
//! along a profile ([`ShellProfile`]). A nose cone is a shell that closes to a point, an
//! adapter one between two diameters, and one with the same diameter at both ends a plain
//! cylinder. The air, the mass and the renderer all go by the same outline
//! ([`Shell::profile`]).

use std::f64::consts::FRAC_PI_2;

use crate::{
    data::{ShellDef, ShellProfile},
    geometry::curve::remaining,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub name: String,
    /// Height of its base above the base of its stage, m.
    pub base: f64,
    pub length: f64,
    /// Radii at its base and its top, m.
    pub bottom_radius: f64,
    pub top_radius: f64,
    pub profile: ShellProfile,
    /// kg, spread over its wall.
    pub mass: f64,
}

impl Shell {
    /// `def` standing `base` m up its stage on something `below` m in radius.
    pub fn new(def: &ShellDef, base: f64, below: f64) -> Result<Self, String> {
        let bottom = def.bottom_diameter.map_or(below, |d| d / 2.0);
        let top = def.top_diameter.map_or(bottom, |d| d / 2.0);
        let name = def.name.clone().unwrap_or_else(|| {
            if top == 0.0 {
                "Nose cone"
            } else if top != bottom {
                "Adapter"
            } else {
                "Shell"
            }
            .into()
        });
        let bad = |problem: &str| Err(format!("shell `{name}`: {problem}"));
        if def.length <= 0.0 {
            return bad("its length must be positive");
        }
        if !(bottom >= 0.0 && top >= 0.0) || bottom + top == 0.0 {
            return bad("its diameters cannot be negative, nor both 0");
        }
        if def.mass < 0.0 {
            return bad("its mass cannot be negative");
        }
        if def.profile == ShellProfile::Ogive && def.length < (bottom - top).abs() {
            return bad(
                "a tangent ogive is at least as long as its radius changes (an elliptical \
                 profile is blunter)",
            );
        }
        Ok(Self {
            name,
            base,
            length: def.length,
            bottom_radius: bottom,
            top_radius: top,
            profile: def.profile,
            mass: def.mass,
        })
    }

    /// Height of its top above the base of its stage, m.
    pub fn top(&self) -> f64 {
        self.base + self.length
    }

    /// Its radius `t` of the way up it (0 at its base, 1 at its top), m.
    pub fn radius_at(&self, t: f64) -> f64 {
        let change = self.bottom_radius - self.top_radius;
        let left = remaining(self.profile, t, self.length, change.abs());
        self.top_radius + change * left
    }

    /// Points up its side, (height above its stage's base, radius) from its base to its
    /// top: the two ends of a straight side, or `steps` along a curve, closer together
    /// towards the top, where blunt curves turn fastest.
    pub fn profile(&self, steps: usize) -> Vec<(f64, f64)> {
        let straight = self.profile == ShellProfile::Conic
            || (self.bottom_radius - self.top_radius).abs() < 1e-9;
        let n = if straight { 1 } else { steps.max(1) };
        (0..=n)
            .map(|i| {
                let t = (FRAC_PI_2 * i as f64 / n as f64).sin();
                let r = match i {
                    0 => self.bottom_radius,
                    _ if i == n => self.top_radius,
                    _ => self.radius_at(t),
                };
                (self.base + t * self.length, r)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(profile: ShellProfile, length: f64, top: f64) -> Shell {
        let def = ShellDef {
            name: None,
            length,
            bottom_diameter: None,
            top_diameter: Some(top),
            profile,
            mass: 0.0,
        };
        Shell::new(&def, 10.0, 1.85).unwrap()
    }

    const CURVES: [ShellProfile; 4] = [
        ShellProfile::Ogive,
        ShellProfile::Elliptical,
        ShellProfile::Parabolic,
        ShellProfile::Haack,
    ];

    #[test]
    fn noses_meet_the_side_below_and_close_at_the_tip() {
        for profile in CURVES {
            let points = shell(profile, 4.8, 0.0).profile(24);
            assert_eq!(points[0], (10.0, 1.85), "{profile:?}");
            assert_eq!(*points.last().unwrap(), (14.8, 0.0), "{profile:?}");
            // Narrowing all the way, and leaving the base along the axis: the radius
            // barely changes over the first step.
            assert!(
                points
                    .windows(2)
                    .all(|w| w[1].0 > w[0].0 && w[1].1 <= w[0].1),
                "{profile:?}"
            );
            let (dz, dr) = (points[1].0 - points[0].0, points[0].1 - points[1].1);
            assert!(dr < 0.25 * dz, "{profile:?} {:?}", &points[..2]);
        }
        // A cone is straight.
        let cone = shell(ShellProfile::Conic, 4.8, 0.0);
        assert_eq!(cone.profile(24), vec![(10.0, 1.85), (14.8, 0.0)]);
        assert!((cone.radius_at(0.5) - 0.925).abs() < 1e-12);
    }

    #[test]
    fn the_profiles_have_their_textbook_shapes() {
        // Halfway up a 1.85 m nose 4.8 m long.
        let half = |profile| shell(profile, 4.8, 0.0).radius_at(0.5);
        // Ogive: √(ρ² − x²) + R − ρ with ρ = (R² + L²) / 2R.
        let rho: f64 = (1.85f64.powi(2) + 4.8f64.powi(2)) / 3.7;
        let ogive = (rho * rho - 2.4f64.powi(2)).sqrt() + 1.85 - rho;
        assert!((half(ShellProfile::Ogive) - ogive).abs() < 1e-12);
        assert!((half(ShellProfile::Elliptical) - 1.85 * 0.75f64.sqrt()).abs() < 1e-12);
        assert!((half(ShellProfile::Parabolic) - 1.85 * 0.75).abs() < 1e-12);
        // Haack at its middle: θ = π/2, R √(1/2).
        assert!((half(ShellProfile::Haack) - 1.85 * 0.5f64.sqrt()).abs() < 1e-12);
        // Slimmest to fullest at this fineness: cone, Haack, parabola, ogive, ellipse.
        let r: Vec<f64> = [
            ShellProfile::Conic,
            ShellProfile::Haack,
            ShellProfile::Parabolic,
            ShellProfile::Ogive,
            ShellProfile::Elliptical,
        ]
        .map(half)
        .into();
        assert!(r.windows(2).all(|w| w[0] < w[1]), "{r:?}");
    }

    #[test]
    fn a_shell_can_stop_short_of_a_point_or_widen() {
        // Flat-tipped: the curve ends at the top radius.
        let blunt = shell(ShellProfile::Elliptical, 1.0, 1.0);
        assert_eq!(*blunt.profile(12).last().unwrap(), (11.0, 0.5));
        // A flare to a wider stage above.
        let flare = shell(ShellProfile::Conic, 2.0, 5.0);
        assert_eq!(flare.profile(12), vec![(10.0, 1.85), (12.0, 2.5)]);
        assert_eq!(flare.name, "Adapter");
        // A cylinder by default.
        let def = ShellDef {
            name: None,
            length: 3.0,
            bottom_diameter: None,
            top_diameter: None,
            profile: ShellProfile::Ogive,
            mass: 0.0,
        };
        let cylinder = Shell::new(&def, 0.0, 1.0).unwrap();
        assert_eq!(cylinder.profile(12), vec![(0.0, 1.0), (3.0, 1.0)]);
        assert_eq!(cylinder.name, "Shell");
    }

    #[test]
    fn impossible_shells_are_refused() {
        let def = |length: f64, top: f64, profile| ShellDef {
            name: None,
            length,
            bottom_diameter: None,
            top_diameter: Some(top),
            profile,
            mass: 0.0,
        };
        let ok = |d: ShellDef| Shell::new(&d, 0.0, 1.0).is_ok();
        assert!(!ok(def(0.0, 0.0, ShellProfile::Conic)));
        assert!(!ok(def(1.0, -1.0, ShellProfile::Conic)));
        // An ogive shorter than its radius change has no arc tangent at the base.
        assert!(!ok(def(0.5, 0.0, ShellProfile::Ogive)));
        assert!(ok(def(0.5, 0.0, ShellProfile::Elliptical)));
        assert!(ok(def(1.0, 0.0, ShellProfile::Ogive)));
    }
}
