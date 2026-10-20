/// Standard gravity, used to convert specific impulse (s) into exhaust velocity (m/s).
pub const G0: f64 = 9.806_65;

/// Universal gas constant, J/(mol·K).
pub const R_GAS: f64 = 8.314_462_618;

/// Earth sea-level standard pressure, Pa. Engine Isp curves are referenced against it.
pub const ATM: f64 = 101_325.0;

pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Mean obliquity of the ecliptic at J2000 (IAU 2006), radians. Rotates ICRF (equatorial)
/// axes into the J2000 ecliptic axes used by the simulation.
pub const OBLIQUITY_J2000: f64 = 84_381.406 / 3_600.0 * std::f64::consts::PI / 180.0;
