use serde::{Deserialize, Serialize};

use crate::constants::R_GAS;

/// Ratio of specific heats for air-like gases; good enough for CO2-heavy atmospheres too.
const GAMMA: f64 = 1.4;

/// Static state of the air at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtmosphereSample {
    pub pressure: f64,
    pub density: f64,
    pub temperature: f64,
    pub speed_of_sound: f64,
}

impl AtmosphereSample {
    pub const VACUUM: Self = Self {
        pressure: 0.0,
        density: 0.0,
        temperature: 0.0,
        speed_of_sound: 0.0,
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AtmosphereModel {
    /// US Standard Atmosphere 1976 below 86 km, exponential tail above.
    Us1976,
    /// Isothermal exponential atmosphere.
    Exponential {
        surface_pressure: f64,
        scale_height: f64,
        temperature: f64,
        /// kg/mol
        molar_mass: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Atmosphere {
    pub model: AtmosphereModel,
    /// Altitude above which the atmosphere is treated as vacuum (enables on-rails warp).
    pub height: f64,
}

impl Atmosphere {
    pub fn sample(&self, altitude: f64) -> AtmosphereSample {
        if altitude >= self.height {
            return AtmosphereSample::VACUUM;
        }
        let altitude = altitude.max(0.0);
        let (pressure, temperature, molar_mass) = match &self.model {
            AtmosphereModel::Us1976 => {
                let (p, t) = us1976(altitude);
                (p, t, US1976_MOLAR_MASS)
            }
            AtmosphereModel::Exponential {
                surface_pressure,
                scale_height,
                temperature,
                molar_mass,
            } => (
                surface_pressure * (-altitude / scale_height).exp(),
                *temperature,
                *molar_mass,
            ),
        };
        let specific_gas_constant = R_GAS / molar_mass;
        AtmosphereSample {
            pressure,
            density: pressure / (specific_gas_constant * temperature),
            temperature,
            speed_of_sound: (GAMMA * specific_gas_constant * temperature).sqrt(),
        }
    }
}

const US1976_MOLAR_MASS: f64 = 0.028_964_4;
const US1976_G0: f64 = 9.806_65;
const EARTH_RADIUS_FOR_GEOPOTENTIAL: f64 = 6_356_766.0;

/// (base geopotential altitude m, base temperature K, lapse rate K/m, base pressure Pa)
const US1976_LAYERS: [(f64, f64, f64, f64); 7] = [
    (0.0, 288.15, -0.0065, 101_325.0),
    (11_000.0, 216.65, 0.0, 22_632.06),
    (20_000.0, 216.65, 0.001, 5_474.889),
    (32_000.0, 228.65, 0.0028, 868.0187),
    (47_000.0, 270.65, 0.0, 110.9063),
    (51_000.0, 270.65, -0.0028, 66.938_87),
    (71_000.0, 214.65, -0.002, 3.956_42),
];

/// Returns (pressure Pa, temperature K) for geometric altitude in metres.
fn us1976(geometric_altitude: f64) -> (f64, f64) {
    let h = EARTH_RADIUS_FOR_GEOPOTENTIAL * geometric_altitude
        / (EARTH_RADIUS_FOR_GEOPOTENTIAL + geometric_altitude);

    // Above 84.852 km geopotential (86 km geometric) the 1976 model's composition changes;
    // a 7 km scale-height exponential tail is plenty accurate for drag purposes.
    const TOP: f64 = 84_852.0;
    if h > TOP {
        let (p_top, t_top) = us1976_layered(TOP);
        return (p_top * (-(h - TOP) / 7_000.0).exp(), t_top);
    }
    us1976_layered(h)
}

fn us1976_layered(h: f64) -> (f64, f64) {
    let &(h_b, t_b, lapse, p_b) = US1976_LAYERS
        .iter()
        .rev()
        .find(|layer| h >= layer.0)
        .unwrap_or(&US1976_LAYERS[0]);
    let gmr = US1976_G0 * US1976_MOLAR_MASS / R_GAS;
    let dh = h - h_b;
    if lapse == 0.0 {
        (p_b * (-gmr * dh / t_b).exp(), t_b)
    } else {
        let t = t_b + lapse * dh;
        (p_b * (t_b / t).powf(gmr / lapse), t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth() -> Atmosphere {
        Atmosphere {
            model: AtmosphereModel::Us1976,
            height: 140_000.0,
        }
    }

    #[test]
    fn sea_level_matches_standard() {
        let s = earth().sample(0.0);
        assert!((s.pressure - 101_325.0).abs() < 1.0);
        assert!((s.density - 1.225).abs() < 0.001);
        assert!((s.speed_of_sound - 340.29).abs() < 0.1);
    }

    #[test]
    fn tropopause_and_stratosphere() {
        let s = earth().sample(11_019.0); // 11 km geopotential
        assert!((s.temperature - 216.65).abs() < 0.1);
        let s = earth().sample(30_000.0);
        assert!(
            (s.density - 0.018_41).abs() / 0.018_41 < 0.01,
            "{}",
            s.density
        );
    }

    #[test]
    fn vacuum_above_height() {
        assert_eq!(earth().sample(150_000.0).pressure, 0.0);
        assert!(earth().sample(100_000.0).pressure > 0.0);
    }
}
