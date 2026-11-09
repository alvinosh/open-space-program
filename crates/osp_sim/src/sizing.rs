//! Procedural parts: what a part must do, and the design that does it.
//!
//! A craft file can give a part a requirement (`procedural`) in place of a shape and what it is
//! made of. A sizer turns that and a technology into dimensions, mass and performance whenever
//! the craft is built, so a better record improves every craft that uses it. A requirement that
//! cannot be met is a build error with its reason, never a part that quietly falls short. Each
//! sizer lists its fields for the editor's palette, and has an oracle: a real part it must
//! reproduce within a stated tolerance, as a test here.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use crate::{
    component::{
        AntennaDef, AvionicsUnitDef, BatteryPackDef, ComponentDef, DecouplerDef, DockingPortDef,
        FuelCellDef, PortKind, RtgDef, SolarArrayDef, SolidMotorDef, StorageDef, StructureDef,
    },
    constants::G0,
    data::{BatteryDef, PartCatalog},
    environment::SOLAR_CONSTANT,
    geometry::{
        Fill, ShapeDef,
        preset::{BoxDef, CylinderTankDef, RingDef},
    },
    skip::is_default,
    structure::DOME_RATIO,
};

/// One thing a requirement asks, as the palette shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    /// What a slider runs over and starts at.
    pub range: [f64; 2],
    pub default: f64,
}

/// A part's requirement, by what it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ProceduralDef {
    Battery(BatteryRequirement),
    FuelCell(FuelCellRequirement),
    SolarArray(SolarArrayRequirement),
    Rtg(RtgRequirement),
    Avionics(AvionicsRequirement),
    Antenna(AntennaRequirement),
    Decoupler(DecouplerRequirement),
    DockingPort(DockingPortRequirement),
    SeparationMotor(SeparationMotorRequirement),
}

/// What a sizer makes of a requirement: a part's shape, what it is made of and what it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    pub shape: ShapeDef,
    pub structure: StructureDef,
    pub components: Vec<ComponentDef>,
}

impl ProceduralDef {
    /// Size it from `catalog`'s records, or say why it cannot be done.
    pub fn size(&self, catalog: &PartCatalog) -> Result<Design, String> {
        match self {
            Self::Battery(requirement) => requirement.size(catalog),
            Self::FuelCell(requirement) => requirement.size(),
            Self::SolarArray(requirement) => requirement.size(),
            Self::Rtg(requirement) => requirement.size(),
            Self::Avionics(requirement) => requirement.size(catalog),
            Self::Antenna(requirement) => requirement.size(),
            Self::Decoupler(requirement) => requirement.size(),
            Self::DockingPort(requirement) => requirement.size(),
            Self::SeparationMotor(requirement) => requirement.size(catalog),
        }
    }

    /// What it asks for.
    pub fn fields(&self) -> &'static [Field] {
        match self {
            Self::Battery(_) => BatteryRequirement::FIELDS,
            Self::FuelCell(_) => FuelCellRequirement::FIELDS,
            Self::SolarArray(_) => SolarArrayRequirement::FIELDS,
            Self::Rtg(_) => RtgRequirement::FIELDS,
            Self::Avionics(_) => AvionicsRequirement::FIELDS,
            Self::Antenna(_) => AntennaRequirement::FIELDS,
            Self::Decoupler(_) => DecouplerRequirement::FIELDS,
            Self::DockingPort(_) => DockingPortRequirement::FIELDS,
            Self::SeparationMotor(_) => SeparationMotorRequirement::FIELDS,
        }
    }
}

/// A battery: so much energy, from a technology.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryRequirement {
    /// What it holds full, kJ.
    pub energy: f64,
    /// Id of a `batteries` record.
    pub technology: String,
    /// It has to recharge in flight.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rechargeable: bool,
}

/// kJ in a watt-hour.
const KJ_PER_WH: f64 = 3.6;
/// Litres in a cubic metre.
const LITRES: f64 = 1000.0;

impl BatteryRequirement {
    pub const FIELDS: &'static [Field] = &[Field {
        id: "energy",
        label: "Energy",
        unit: "kJ",
        range: [10.0, 1e6],
        default: 3600.0,
    }];

    fn record<'a>(&self, catalog: &'a PartCatalog) -> Result<&'a BatteryDef, String> {
        let record = catalog
            .batteries
            .iter()
            .find(|b| b.id == self.technology)
            .ok_or_else(|| format!("unknown battery technology `{}`", self.technology))?;
        if self.rechargeable && !record.rechargeable {
            return Err(format!(
                "{} cannot recharge, and this battery has to",
                record.id
            ));
        }
        Ok(record)
    }

    /// A solid drum as tall as it is wide, as big as the energy takes at the technology's
    /// density; it weighs what such a battery does ([`crate::component`]).
    fn size(&self, catalog: &PartCatalog) -> Result<Design, String> {
        if !(self.energy > 0.0 && self.energy.is_finite()) {
            return Err(format!("a battery holds some energy, not {}", self.energy));
        }
        let record = self.record(catalog)?;
        if record.energy_density <= 0.0 {
            return Err(format!("{} gives no energy density", record.id));
        }
        let volume = self.energy / KJ_PER_WH / record.energy_density / LITRES;
        let diameter = (4.0 * volume / PI).cbrt();
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter,
                height: diameter,
                thickness: diameter / 2.0,
            }),
            structure: StructureDef::default(),
            components: vec![ComponentDef::Battery(BatteryPackDef {
                charge: None,
                technology: Some(record.id.clone()),
                solid: None,
            })],
        })
    }
}

/// Kilograms a watt of fuel cell takes: the Apollo CSM's, 111 kg for 1.42 kW.
const FUEL_CELL_KG_PER_W: f64 = 111.0 / 1420.0;
/// Its size at 1.42 kW across, m, as a box twice and a bit as tall as it is wide.
const FUEL_CELL_WIDTH: f64 = 0.5;
const FUEL_CELL_ASPECT: f64 = 2.2;

/// A fuel cell: so much power.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelCellRequirement {
    /// The most it makes, W.
    pub power: f64,
}

impl FuelCellRequirement {
    pub const FIELDS: &'static [Field] = &[Field {
        id: "power",
        label: "Power",
        unit: "W",
        range: [10.0, 50_000.0],
        default: 1420.0,
    }];

    fn size(&self) -> Result<Design, String> {
        if !(self.power > 0.0 && self.power.is_finite()) {
            return Err(format!("a fuel cell makes some power, not {}", self.power));
        }
        let width = FUEL_CELL_WIDTH * (self.power / 1420.0).cbrt();
        Ok(Design {
            shape: ShapeDef::Cuboid(BoxDef {
                size: [width, width, FUEL_CELL_ASPECT * width],
                fill: Fill::Solid,
            }),
            structure: StructureDef {
                mass: Some(self.power * FUEL_CELL_KG_PER_W),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::FuelCell(FuelCellDef { power: self.power })],
        })
    }
}

/// Kilograms a square metre of panel takes, with its frame.
const SOLAR_KG_PER_M2: f64 = 3.0;
/// How thick it is drawn, m.
const SOLAR_THICKNESS: f64 = 0.03;

/// A solar array: so much power at the Earth's distance from the Sun.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolarArrayRequirement {
    /// What it makes, new, facing the Sun at 1 AU, W.
    pub power: f64,
    /// The share of the sunlight it turns into electricity; a 1970s silicon cell's if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<f64>,
    /// It turns to face the Sun.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tracking: bool,
}

impl SolarArrayRequirement {
    pub const FIELDS: &'static [Field] = &[Field {
        id: "power",
        label: "Power at 1 AU",
        unit: "W",
        range: [1.0, 100_000.0],
        default: 1000.0,
    }];

    fn size(&self) -> Result<Design, String> {
        let efficiency = self.efficiency.unwrap_or(0.14);
        if !(self.power > 0.0 && self.power.is_finite()) {
            return Err(format!(
                "a solar array makes some power, not {}",
                self.power
            ));
        }
        if !(efficiency > 0.0 && efficiency <= 1.0) {
            return Err(format!(
                "no cell turns {efficiency} of the sunlight into power"
            ));
        }
        let area = self.power / (SOLAR_CONSTANT * efficiency);
        let side = area.sqrt();
        Ok(Design {
            shape: ShapeDef::Cuboid(BoxDef {
                size: [SOLAR_THICKNESS, side, side],
                fill: Fill::Solid,
            }),
            structure: StructureDef {
                mass: Some(area * SOLAR_KG_PER_M2),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::SolarArray(SolarArrayDef {
                area,
                efficiency,
                tracking: self.tracking,
                degradation: 0.01,
            })],
        })
    }
}

/// Kilograms a watt of RTG takes: Voyager's, 37.7 kg for 158 W.
const RTG_KG_PER_W: f64 = 37.7 / 158.0;

/// An RTG: so much power, new.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RtgRequirement {
    /// New, W.
    pub power: f64,
}

impl RtgRequirement {
    pub const FIELDS: &'static [Field] = &[Field {
        id: "power",
        label: "Power, new",
        unit: "W",
        range: [1.0, 5000.0],
        default: 158.0,
    }];

    fn size(&self) -> Result<Design, String> {
        if !(self.power > 0.0 && self.power.is_finite()) {
            return Err(format!("an RTG makes some power, not {}", self.power));
        }
        // A drum about as dense as Voyager's, 0.4 m across and 0.6 m long.
        let diameter = 0.4 * (self.power / 158.0).cbrt();
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter,
                height: 1.5 * diameter,
                thickness: diameter / 2.0,
            }),
            structure: StructureDef {
                mass: Some(self.power * RTG_KG_PER_W),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::Rtg(RtgDef {
                power: self.power,
                half_life: 87.7,
            })],
        })
    }
}

/// Avionics: a ring that controls so much mass, from a technology.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvionicsRequirement {
    /// The most it is to control, kg.
    pub controlled_mass: f64,
    /// Id of an `avionics` record.
    pub technology: String,
    /// The ring it is, m.
    pub diameter: f64,
    pub height: f64,
    /// What it runs on, kJ: a battery of its own on the ring; none if a network feeds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<f64>,
}

impl AvionicsRequirement {
    pub const FIELDS: &'static [Field] = &[Field {
        id: "controlled_mass",
        label: "Controls up to",
        unit: "kg",
        range: [100.0, 5e6],
        default: 10_000.0,
    }];

    /// RP-1's procedural avionics: mass and power are a factor times the tonnes it controls to
    /// a power, plus a constant.
    fn size(&self, catalog: &PartCatalog) -> Result<Design, String> {
        if !(self.controlled_mass > 0.0 && self.controlled_mass.is_finite()) {
            return Err(format!(
                "avionics control some mass, not {}",
                self.controlled_mass
            ));
        }
        if !(self.diameter > 0.0 && self.height > 0.0) {
            return Err("avionics are a ring with a diameter and a height".into());
        }
        let record = catalog
            .avionics
            .iter()
            .find(|a| a.id == self.technology)
            .ok_or_else(|| format!("unknown avionics technology `{}`", self.technology))?;
        let tonnes = self.controlled_mass / 1e3;
        let kg = record.mass_factor * tonnes.powf(record.mass_exponent) + record.mass_constant;
        let watts =
            record.power_factor * tonnes.powf(record.power_exponent) + record.power_constant;
        let mut components = vec![ComponentDef::Avionics(AvionicsUnitDef {
            controlled_mass: self.controlled_mass,
            power: watts,
            interplanetary: record.interplanetary,
        })];
        if let Some(charge) = self.battery {
            components.push(ComponentDef::Battery(BatteryPackDef {
                charge: Some(charge),
                technology: None,
                solid: None,
            }));
        }
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter: self.diameter,
                height: self.height,
                thickness: 0.02 * self.diameter,
            }),
            structure: StructureDef {
                mass: Some(kg),
                ..StructureDef::default()
            },
            components,
        })
    }
}

/// Kilograms a square metre of dish takes, kilograms of transmitter and transponder.
const DISH_KG_PER_M2: f64 = 6.0;
const TRANSPONDER_KG: f64 = 15.0;

/// An antenna: a dish of so much diameter (none for an omni), so much power, so much data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntennaRequirement {
    /// m; 0 for an omni.
    #[serde(default)]
    pub diameter: f64,
    /// What it sends, W.
    pub power: f64,
    /// bit/s.
    pub data_rate: f64,
    /// It passes on other vessels' signals to the ground.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relay: bool,
}

impl AntennaRequirement {
    pub const FIELDS: &'static [Field] = &[
        Field {
            id: "diameter",
            label: "Dish",
            unit: "m",
            range: [0.0, 10.0],
            default: 0.8,
        },
        Field {
            id: "power",
            label: "Sends",
            unit: "W",
            range: [0.1, 500.0],
            default: 20.0,
        },
    ];

    fn size(&self) -> Result<Design, String> {
        if !(self.power > 0.0 && self.data_rate > 0.0 && self.diameter >= 0.0) {
            return Err(format!(
                "an antenna sends some power at some rate, not {} W at {} bit/s",
                self.power, self.data_rate
            ));
        }
        let drawn = self.diameter.max(0.1);
        let area = PI * (self.diameter / 2.0).powi(2);
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter: drawn,
                height: 0.1 * drawn,
                thickness: drawn / 2.0,
            }),
            structure: StructureDef {
                mass: Some(area * DISH_KG_PER_M2 + TRANSPONDER_KG),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::Antenna(AntennaDef {
                diameter: self.diameter,
                power: self.power,
                data_rate: self.data_rate,
                relay: self.relay,
            })],
        })
    }
}

/// What an aluminium ring is good for, Pa, and weighs, kg/m³; how much over the load it is built
/// for; the thinnest wall worth making, m; and how tall it is against its diameter.
const RING_STRESS: f64 = 300e6;
const RING_DENSITY: f64 = 2700.0;
const RING_SAFETY: f64 = 1.25;
const RING_MIN_WALL: f64 = 0.002;
const RING_HEIGHT: f64 = 0.04;

/// A decoupler: a ring of so much diameter carrying so much load, sized from its wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecouplerRequirement {
    /// m.
    pub diameter: f64,
    /// What it has to carry, N: what stands on it at the most the stack pulls.
    pub load: f64,
    /// What it pushes the two sides apart with, N·s.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub impulse: f64,
    /// Propellant flows across until it lets go.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crossfeed: bool,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

impl DecouplerRequirement {
    pub const FIELDS: &'static [Field] = &[
        Field {
            id: "diameter",
            label: "Diameter",
            unit: "m",
            range: [0.1, 15.0],
            default: 2.0,
        },
        Field {
            id: "load",
            label: "Carries",
            unit: "N",
            range: [1e3, 1e8],
            default: 1e6,
        },
    ];

    fn size(&self) -> Result<Design, String> {
        if !(self.diameter > 0.0 && self.load > 0.0) {
            return Err(format!(
                "a decoupler is a ring of some size carrying some load, not {} m and {} N",
                self.diameter, self.load
            ));
        }
        let circumference = PI * self.diameter;
        let wall = (RING_SAFETY * self.load / (circumference * RING_STRESS)).max(RING_MIN_WALL);
        if wall > self.diameter / 4.0 {
            return Err(format!(
                "a {} m ring cannot carry {} N: its wall would be {wall:.3} m",
                self.diameter, self.load
            ));
        }
        let height = RING_HEIGHT * self.diameter;
        let mass = circumference * height * wall * RING_DENSITY;
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter: self.diameter,
                height,
                thickness: wall,
            }),
            structure: StructureDef {
                mass: Some(mass),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::Decoupler(DecouplerDef {
                node: "top".into(),
                impulse: self.impulse,
                crossfeed: self.crossfeed,
                // What its wall carries at the stress it yields at.
                strength: circumference * wall * RING_STRESS,
            })],
        })
    }
}

/// A docking port's ring against its diameter, and what its latches, collar and seals weigh
/// against its wall: a rough allowance, which makes a 0.8 m port for 100 kN about 30 kg.
const PORT_HEIGHT: f64 = 0.25;
const PORT_HARDWARE: f64 = 10.0;

/// A docking port: a ring of so much diameter carrying so much load, sized from its wall, with
/// the port on its top node. It stands on what carries it by its bottom one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockingPortRequirement {
    /// m.
    pub diameter: f64,
    /// What it has to carry, N.
    pub load: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: PortKind,
    /// What its springs push two vessels apart with, N·s.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub impulse: f64,
    /// Propellant flows across while docked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crossfeed: bool,
}

impl DockingPortRequirement {
    pub const FIELDS: &'static [Field] = &[
        Field {
            id: "diameter",
            label: "Diameter",
            unit: "m",
            range: [0.3, 5.0],
            default: 0.8,
        },
        Field {
            id: "load",
            label: "Carries",
            unit: "N",
            range: [1e3, 1e7],
            default: 1e5,
        },
    ];

    fn size(&self) -> Result<Design, String> {
        if !(self.diameter > 0.0 && self.load > 0.0) {
            return Err(format!(
                "a docking port is a ring of some size carrying some load, not {} m and {} N",
                self.diameter, self.load
            ));
        }
        let circumference = PI * self.diameter;
        let wall = (RING_SAFETY * self.load / (circumference * RING_STRESS)).max(RING_MIN_WALL);
        if wall > self.diameter / 4.0 {
            return Err(format!(
                "a {} m ring cannot carry {} N: its wall would be {wall:.3} m",
                self.diameter, self.load
            ));
        }
        let height = PORT_HEIGHT * self.diameter;
        Ok(Design {
            shape: ShapeDef::Ring(RingDef {
                diameter: self.diameter,
                height,
                thickness: wall,
            }),
            structure: StructureDef {
                mass: Some(PORT_HARDWARE * circumference * height * wall * RING_DENSITY),
                ..StructureDef::default()
            },
            components: vec![ComponentDef::DockingPort(DockingPortDef {
                node: "top".into(),
                kind: self.kind,
                impulse: self.impulse,
                crossfeed: self.crossfeed,
            })],
        })
    }
}

/// A motor's specific impulse in a vacuum and at sea level, s, from the S-II's TE-M-424.
const MOTOR_ISP: [f64; 2] = [256.0, 216.0];
/// Its case against its grain: the TE-M-424's, 104 kg for 153 kg.
const MOTOR_CASE: f64 = 0.68;
/// How long it is against its diameter.
const MOTOR_ASPECT: f64 = 3.0;

/// A small solid motor that pulls a spent stage away or settles propellant: so much impulse
/// over so long.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeparationMotorRequirement {
    /// N·s.
    pub impulse: f64,
    /// s.
    pub burn_time: f64,
}

impl SeparationMotorRequirement {
    pub const FIELDS: &'static [Field] = &[
        Field {
            id: "impulse",
            label: "Impulse",
            unit: "N·s",
            range: [1e3, 1e7],
            default: 1e5,
        },
        Field {
            id: "burn_time",
            label: "Burns for",
            unit: "s",
            range: [0.1, 120.0],
            default: 4.0,
        },
    ];

    fn size(&self, catalog: &PartCatalog) -> Result<Design, String> {
        if !(self.impulse > 0.0 && self.burn_time > 0.0) {
            return Err(format!(
                "a motor has some impulse over some time, not {} N·s in {} s",
                self.impulse, self.burn_time
            ));
        }
        const GRAIN: &str = "HTPB";
        const TANK: &str = "Solid Grain";
        let density = (catalog.resources.iter())
            .find(|r| r.id == GRAIN)
            .map(|r| r.density)
            .ok_or_else(|| format!("no resource {GRAIN} to cast a motor of"))?;
        let utilization = (catalog.tank_types.iter())
            .find(|t| t.id == TANK)
            .map_or(1.0, |t| t.utilization);
        let [isp_vac, isp_sl] = MOTOR_ISP;
        let grain = self.impulse / (isp_vac * G0);
        let volume = grain / (density * utilization);
        // A cylinder with √2:1 domes, MOTOR_ASPECT diameters long: π r² L - 0.471 π r³.
        let radius = (volume / (PI * (2.0 * MOTOR_ASPECT - 0.471))).cbrt();
        let diameter = 2.0 * radius;
        Ok(Design {
            shape: ShapeDef::CylinderTank(CylinderTankDef {
                diameter,
                length: MOTOR_ASPECT * diameter,
                dome: DOME_RATIO,
                common_bulkhead: None,
                aft_skirt: 0.0,
                forward_skirt: 0.0,
                thrust_plate: false,
            }),
            structure: StructureDef::default(),
            components: vec![
                ComponentDef::Storage(StorageDef {
                    cell: None,
                    tank_type: TANK.into(),
                    contents: [(GRAIN.to_string(), 1.0)].into_iter().collect(),
                    design_load: None,
                    utilization: None,
                }),
                ComponentDef::SolidMotor(SolidMotorDef {
                    thrust: self.impulse / self.burn_time,
                    isp_vac,
                    isp_sl,
                    mass: MOTOR_CASE * grain,
                    propellant: GRAIN.into(),
                }),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        craft::Craft, data::CraftDef, propulsion::Propulsion, systems::Battery, test_data::records,
        vessel::tests::catalog,
    };

    const SILVER_ZINC: &str = "Silver-zinc (Mercury, Vostok)";

    fn craft(requirement: &str) -> CraftDef {
        records(&format!(
            r#"{{ id: "sized", name: "", parts: [
                {{ id: "battery", procedural: {{ battery: {{ {requirement} }} }} }}
            ] }}"#
        ))
        .remove(0)
    }

    fn sized(energy: f64) -> Propulsion {
        let text = format!(r#"energy: {energy}, technology: "{SILVER_ZINC}""#);
        Propulsion::from_craft(&craft(&text), &catalog()).unwrap()
    }

    #[test]
    fn a_craft_file_holds_the_requirement_and_the_part_is_the_design() {
        let p = sized(21390.0);
        let (_, battery) = p.systems.all::<Battery>().next().unwrap();
        assert!(
            (battery.charge - 21390.0).abs() < 1e-6,
            "{}",
            battery.charge
        );
        // E over the technology's Wh/kg: 5942 Wh at 96.1 Wh/kg.
        let mass = p.parts().parts()[0].built.as_ref().unwrap().mass.mass;
        assert!((mass - 21390.0 / 3.6 / 96.1).abs() < 0.5, "{mass}");
        // And nothing else was written down than what was asked.
        let def = craft(&format!(r#"energy: 21390, technology: "{SILVER_ZINC}""#));
        let text = serde_json::to_string(&def).unwrap();
        assert!(
            text.contains("procedural") && !text.contains("\"shape\""),
            "{text}"
        );
        let back: CraftDef = serde_json::from_str(&text).unwrap();
        assert_eq!(back.parts, def.parts);
    }

    /// The oracle: the CM's three entry batteries, 40 Ah at 29 V each, 12 528 kJ. Their mass is
    /// PROVISIONAL: 28 lb each is what a NASA description of the subsystem seems to say, from a
    /// fragment of it (NASA TN D-6976 is the report to check it against).
    #[test]
    fn the_battery_sizer_reproduces_the_command_modules_batteries() {
        let want = 3.0 * 28.0 * 0.453_592;
        let energy = 3.0 * 40.0 * 29.0 * 3.6;
        let p = sized(energy);
        let mass = p.parts().parts()[0].built.as_ref().unwrap().mass.mass;
        assert!((mass - want).abs() < 0.1 * want, "{mass} kg of {want} kg");
    }

    /// More energy takes more mass, and a better technology less.
    #[test]
    fn more_is_heavier_and_better_is_lighter() {
        let mass = |energy: f64, technology: &str| {
            let text = format!(r#"energy: {energy}, technology: "{technology}""#);
            let craft = Craft::build(&self::craft(&text), &catalog()).unwrap();
            craft.parts[0].built.mass.mass
        };
        assert!(mass(2000.0, SILVER_ZINC) > mass(1000.0, SILVER_ZINC));
        assert!(mass(1000.0, "Lithium-Thionyl Chloride") < mass(1000.0, SILVER_ZINC));
    }

    #[test]
    fn a_requirement_that_cannot_be_met_is_an_error_with_its_reason() {
        let error = |requirement: &str| {
            Craft::build(&craft(requirement), &catalog())
                .err()
                .expect("it cannot be built")
                .to_string()
        };
        let cannot = format!(r#"energy: 1000, technology: "{SILVER_ZINC}", rechargeable: true"#);
        assert!(
            error(&cannot).contains("cannot recharge"),
            "{}",
            error(&cannot)
        );
        let unknown = r#"energy: 1000, technology: "Moonbeams""#;
        assert!(error(unknown).contains("Moonbeams"), "{}", error(unknown));
        let none = format!(r#"energy: -5, technology: "{SILVER_ZINC}""#);
        assert!(error(&none).contains("-5"), "{}", error(&none));
    }

    fn part_mass(procedural: &str) -> f64 {
        let craft: CraftDef = records(&format!(
            r#"{{ id: "sized", name: "", parts: [{{ id: "x", procedural: {{ {procedural} }} }}] }}"#
        ))
        .remove(0);
        Craft::build(&craft, &catalog()).unwrap().parts[0]
            .built
            .mass
            .mass
    }

    /// The oracles: the Apollo CSM's fuel cell, 111 kg for 1.42 kW, and Voyager's RTG, 37.7 kg
    /// for 158 W.
    #[test]
    fn power_sources_size_to_their_oracles() {
        let cell = part_mass("fuel_cell: { power: 1420 }");
        assert!((cell - 111.0).abs() < 1e-6, "{cell}");
        let rtg = part_mass("rtg: { power: 158 }");
        assert!((rtg - 37.7).abs() < 1e-6, "{rtg}");
        // Twice the power takes twice the mass; a better cell a smaller array.
        assert!(part_mass("fuel_cell: { power: 2840 }") > 1.9 * cell);
        let (plain, better) = (
            part_mass("solar_array: { power: 1000 }"),
            part_mass("solar_array: { power: 1000, efficiency: 0.28 }"),
        );
        assert!(better < 0.6 * plain, "{better} of {plain}");
    }

    /// The oracle: the Saturn V's instrument unit weighed about 2 t. The mass of what it
    /// controls and a 1959 level of RP-1's records make the same, within 10%.
    #[test]
    fn avionics_size_to_the_instrument_unit() {
        let mass = part_mass(
            r#"avionics: { controlled_mass: 2950000, technology: "Near-Earth: Basic Avionics",
                diameter: 6.6, height: 0.91 }"#,
        );
        assert!((mass - 2041.0).abs() < 0.1 * 2041.0, "{mass}");
        // Less to control takes less, and a later level less again.
        let small = part_mass(
            r#"avionics: { controlled_mass: 45000, technology: "Near-Earth: Basic Avionics",
                diameter: 1, height: 0.2 }"#,
        );
        let later = part_mass(
            r#"avionics: { controlled_mass: 2950000, technology: "Near-Earth: Large Scale Avionics",
                diameter: 6.6, height: 0.91 }"#,
        );
        assert!(small < 0.2 * mass && later < 0.6 * mass, "{small} {later}");
    }

    /// A ring carries what it is sized for and a little more, and more load takes more mass.
    #[test]
    fn a_decoupler_is_sized_from_what_it_carries() {
        let ring = |load: f64| -> (f64, f64) {
            let craft: CraftDef = records(&format!(
                r#"{{ id: "r", name: "", parts: [{{ id: "x",
                    procedural: {{ decoupler: {{ diameter: 10.1, load: {load} }} }} }}] }}"#
            ))
            .remove(0);
            let built = Craft::build(&craft, &catalog()).unwrap();
            let part = built.parts[0].built.clone();
            let strength = part.components.iter().find_map(|c| match c {
                crate::component::Component::Decoupler(d) => Some(d.strength),
                _ => None,
            });
            (part.mass.mass, strength.unwrap())
        };
        // The Saturn V's load on the S-IC's ring, about 26 MN at 4 g.
        let (mass, strength): (f64, f64) = ring(30e6);
        assert!((strength - 1.25 * 30e6).abs() < 1e3, "{strength}");
        assert!(mass > 50.0 && mass < 500.0, "{mass}");
        assert!(ring(60e6).0 > 1.9 * mass);
        // A light load takes the thinnest wall worth making.
        assert_eq!(ring(1e3).0, ring(2e3).0);
        // And one past what a ring of it can take says so.
        let craft: CraftDef = records(
            r#"{ id: "r", name: "", parts: [{ id: "x",
                procedural: { decoupler: { diameter: 0.1, load: 1e9 } } }] }"#,
        )
        .remove(0);
        let error = Craft::build(&craft, &catalog()).err().unwrap().to_string();
        assert!(error.contains("cannot carry"), "{error}");
    }

    /// The oracle: the S-II's TE-M-424, 102.3 kN for about 3.75 s, 153 kg of grain in a 104 kg
    /// case. It is one part, a tank and an engine that feeds on it.
    #[test]
    fn a_separation_motor_sizes_to_the_te_m_424() {
        let craft: CraftDef = records(
            r#"{ id: "m", name: "", parts: [{ id: "x",
                procedural: { separation_motor: { impulse: 383600, burn_time: 3.75 } } }] }"#,
        )
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        let engine = p.engines[0].part;
        assert_eq!(p.tanks[0].part, engine);
        let thrust = p.engines[0].def.thrust_vac;
        assert!((thrust - 102_300.0).abs() < 100.0, "{thrust}");
        let grain: f64 = p.tanks[0].amounts.iter().sum();
        assert!((grain - 153.0).abs() < 0.1 * 153.0, "{grain}");
        // The part weighs its case and the grain's thin liner, once.
        let dry = p.parts().parts()[0].built.as_ref().unwrap().mass.mass;
        assert!((dry - 104.0).abs() < 0.1 * 104.0, "{dry}");
        // Lit, it burns about as long as it was asked to, on its own grain.
        p.set_throttle(1.0);
        p.activate_next_sequence().unwrap();
        let mut time: f64 = 0.0;
        for _ in 0..1000 {
            p.burn(0.0, 0.02, true, &mut Vec::new());
            if time > 0.1 && !p.engines[0].running {
                break;
            }
            time += 0.02;
        }
        assert!((time - 3.75).abs() < 0.5, "{time}");
    }

    #[test]
    fn a_sizer_lists_what_it_asks_for() {
        let def = ProceduralDef::Battery(BatteryRequirement {
            energy: 1.0,
            technology: SILVER_ZINC.into(),
            rechargeable: false,
        });
        let field = &def.fields()[0];
        assert_eq!((field.id, field.unit), ("energy", "kJ"));
        assert!(field.range[0] < field.default && field.default < field.range[1]);
    }
}
