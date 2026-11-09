//! What a part does besides propulsion, with state of its own: a component's runtime half.
//!
//! A component is a definition in the craft file ([`crate::component::ComponentDef`]), a built
//! form ([`crate::component::Component`]), and, if it holds state or does something over time,
//! a [`System`] here. [`KINDS`] is the list of them: a new system is a type that implements
//! [`System`] and a line in it, and the physics, the part window, the smoke report and saved
//! games pick it up with no code of their own for it. A component no one knows is an error when the craft loads,
//! which names it.
//!
//! [`Systems`] holds the state part by part, next to the tanks and engines of the
//! [`crate::propulsion::Propulsion`], and goes with a part when it is let go.

mod battery;
mod crew;
mod electric;
mod leg;
mod parachute;
mod swing;
mod thermal;

use std::{any::Any, collections::BTreeMap, fmt::Debug};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, de::DeserializeOwned, ser};

pub use self::{
    battery::Battery,
    crew::{BOARD, Crew},
    electric::{Antenna, Avionics, FuelCell, Load, Rtg, SolarArray},
    leg::{DEPLOY_LEG, Leg},
    parachute::{ARM, CUT, ChuteState, DEPLOY, Parachute},
    swing::{Swing, UNFOLD},
    thermal::{HeatShield, Thermal},
};
use crate::{
    component::Component,
    environment::Environment,
    parts::{Part, PartId, PartTree},
    staging::Action,
};

/// One figure of a system, for whoever shows it: an editor's property panel, a part window, a
/// chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Stable, for data bindings and charts.
    pub id: &'static str,
    pub label: &'static str,
    pub value: f64,
    pub unit: &'static str,
}

/// A system's state as bytes, for a saved game; [`Kind::load`] reads it back.
pub trait SavedState {
    fn saved_state(&self) -> Result<Vec<u8>, String>;
}

impl<T: Serialize> SavedState for T {
    fn saved_state(&self) -> Result<Vec<u8>, String> {
        rmp_serde::to_vec_named(self).map_err(|e| e.to_string())
    }
}

/// How a vessel is flying, for the systems that care.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlightNow {
    /// Over the sea, m.
    pub altitude: f64,
    /// kg/m³.
    pub density: f64,
    /// Pa.
    pub dynamic_pressure: f64,
    /// What it feels, m/s².
    pub acceleration: f64,
    /// W/m² at a blunt nose of the vessel's size.
    pub stagnation_heating: f64,
    pub on_the_ground: bool,
}

/// The heat a part is taking on this tick, W, and how much of it the part's shield could take.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PartHeat {
    pub bare: f64,
    pub shielded: f64,
    /// What of the shield faces the flow, m².
    pub shield_area: f64,
}

/// How a vessel's power stands, for the player.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PowerState {
    /// What its stores hold, and can hold, kJ.
    pub charge: f64,
    pub capacity: f64,
    /// Seconds until the first of its power networks runs out, if one is running down. Each network
    /// has its own stores, so this can come well before the total above runs out.
    pub runs_out_in: Option<f64>,
    /// Some load has been left without power, the lowest priority first.
    pub shedding: bool,
    /// A load it cannot do without has no power, its avionics among them.
    pub browned_out: bool,
    /// The pad powers it (see [`Systems::step`]): nothing runs down.
    pub supplied: bool,
}

impl PowerState {
    /// It has stores, and there is next to nothing in them: what it runs on now is only what it makes.
    pub fn is_flat(&self) -> bool {
        self.capacity > 0.0 && self.charge <= 1e-3 * self.capacity
    }
}

/// A component at run time.
pub trait System: SavedState + Debug + Send + Sync {
    /// Its component's name in a craft file.
    fn kind(&self) -> &'static str;

    /// What it was built as: what does not change in flight.
    fn parameters(&self) -> Vec<Reading>;

    /// What it is doing now.
    fn state(&self) -> Vec<Reading>;

    /// Move on `dt` seconds. It has to be exact for any `dt` up to [`Self::next_event`], since
    /// time warp takes long steps.
    fn step(&mut self, _dt: f64) {}

    /// How it takes part in the power network it is on, under `environment`, if it does.
    fn power(&self, _environment: &Environment) -> Option<Power> {
        None
    }

    /// Take `kj` into its store (a negative amount out of it). Only a [`Power::Store`] is asked.
    fn charge(&mut self, _kj: f64) {}

    /// Tell a [`Power::Load`] whether it has power. It switches off what it must without it.
    fn powered(&mut self, _on: bool) {}

    /// Whether it needs to know the sunlight, so that time warp stops at a shadow's edge.
    fn uses_light(&self) -> bool {
        false
    }

    /// Tell a source or a [`Power::Dispatchable`] what it gave over the last `dt` seconds, kJ.
    fn deliver(&mut self, _kj: f64, _dt: f64) {}

    /// Whether it burns what is in the tanks its part can reach.
    fn consumes(&self) -> bool {
        false
    }

    /// What it has burned since this was last asked: resources and kg, to come out of the tanks.
    fn take_consumption(&mut self) -> Vec<(&'static str, f64)> {
        Vec::new()
    }

    /// What it is burning now: resources and kg/s, so time warp stops where the tanks run dry.
    fn consuming(&self) -> Vec<(&'static str, f64)> {
        Vec::new()
    }

    /// Tell it whether the tanks gave what it burned.
    fn fed(&mut self, _fed: bool) {}

    /// Whether going short of this resource is not its concern for now: it does not count against
    /// [`Self::fed`], and time warp need not stop for it.
    fn can_go_without(&self, _resource: &str) -> bool {
        false
    }

    /// The same, with how long that was for, s.
    fn fed_for(&mut self, fed: bool, _dt: f64) {
        self.fed(fed);
    }

    /// One tick of flight, with the heat falling on its part. A system takes the heat it deals with
    /// and leaves the rest for the next on the part.
    fn fly(&mut self, _flight: &FlightNow, _heat: &mut PartHeat, _dt: f64) {}

    /// What to tell the pilot, once, if anything has happened.
    fn announce(&mut self) -> Option<String> {
        None
    }

    /// Have someone leave its seat. False if there was nobody.
    fn take_person(&mut self) -> bool {
        false
    }

    /// Seat someone. False if there is no room.
    fn seat_person(&mut self) -> bool {
        false
    }

    /// Whether a load the vessel cannot do without has no power.
    fn browned_out(&self) -> bool {
        false
    }

    /// Whether it is a load that has been left without power.
    fn shed(&self) -> bool {
        false
    }

    /// Seconds until it needs attention (a battery runs out, the Sun sets), so that time warp
    /// stops there as it does for a trajectory's events.
    fn next_event(&self) -> Option<f64> {
        None
    }

    /// What it can be told to do: the part's window, the sequences and the staging panel offer
    /// them with no code of their own for any of them.
    fn actions(&self) -> Vec<Action> {
        Vec::new()
    }

    /// Do `action`; what to tell the player, if anything.
    fn act(&mut self, _action: Action) -> Option<String> {
        None
    }

    fn boxed(&self) -> Box<dyn System>;
    fn any(&self) -> &dyn Any;
}

impl Clone for Box<dyn System> {
    fn clone(&self) -> Self {
        self.boxed()
    }
}

/// How a system takes part in a power network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Power {
    /// Makes this much now, W.
    Source(f64),
    /// Makes what the network asks, up to this much, W: a fuel cell, which burns no more than
    /// that.
    Dispatchable(f64),
    /// Holds this much, kJ, of this much when full.
    Store { charge: f64, capacity: f64 },
    /// Wants this much, W. Of the loads on a network the lowest `priority` is shed first.
    Load { watts: f64, priority: u8 },
}

/// A kind of system: which built components become one, and how, the actions it can offer, and
/// what a saved game knows it by.
pub struct Kind {
    pub of: fn(&Component, &Part) -> Option<Box<dyn System>>,
    pub actions: &'static [Action],
    /// Its [`System::kind`].
    pub name: &'static str,
    pub load: fn(&[u8]) -> Result<Box<dyn System>, String>,
}

/// Read a system saved by [`SavedState`].
fn load<T: System + DeserializeOwned + 'static>(state: &[u8]) -> Result<Box<dyn System>, String> {
    let system: T = rmp_serde::from_slice(state).map_err(|e| e.to_string())?;
    Ok(Box::new(system))
}

/// A system in a saved game: its kind, and its state as that kind writes it.
#[derive(Serialize, Deserialize)]
struct SavedSystem {
    kind: String,
    state: Vec<u8>,
}

impl Serialize for Box<dyn System> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedSystem {
            kind: self.kind().to_owned(),
            state: (**self).saved_state().map_err(ser::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Box<dyn System> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedSystem::deserialize(deserializer)?;
        let kind = (KINDS.iter().find(|k| k.name == saved.kind))
            .ok_or_else(|| de::Error::custom(format!("no system is called `{}`", saved.kind)))?;
        (kind.load)(&saved.state).map_err(de::Error::custom)
    }
}

/// Every kind of system there is.
pub const KINDS: &[Kind] = &[
    Kind {
        of: battery::of,
        actions: &[],
        name: "battery",
        load: load::<Battery>,
    },
    Kind {
        of: crew::of,
        actions: &[],
        name: "crew",
        load: load::<Crew>,
    },
    Kind {
        of: electric::fuel_cell,
        actions: &[],
        name: "fuel_cell",
        load: load::<FuelCell>,
    },
    Kind {
        of: electric::solar_array,
        actions: &[],
        name: "solar_array",
        load: load::<SolarArray>,
    },
    Kind {
        of: electric::rtg,
        actions: &[],
        name: "rtg",
        load: load::<Rtg>,
    },
    Kind {
        of: electric::load,
        actions: &[],
        name: "load",
        load: load::<Load>,
    },
    Kind {
        of: electric::avionics,
        actions: &[],
        name: "avionics",
        load: load::<Avionics>,
    },
    Kind {
        of: electric::antenna,
        actions: &[],
        name: "antenna",
        load: load::<Antenna>,
    },
    Kind {
        of: parachute::of,
        actions: &[parachute::ARM, parachute::DEPLOY, parachute::CUT],
        name: "parachute",
        load: load::<Parachute>,
    },
    Kind {
        of: leg::of,
        actions: &[leg::DEPLOY_LEG],
        name: "leg",
        load: load::<Leg>,
    },
    Kind {
        of: swing::of,
        actions: &[swing::UNFOLD],
        name: "swing",
        load: load::<Swing>,
    },
    Kind {
        of: thermal::heat_shield,
        actions: &[],
        name: "heat_shield",
        load: load::<HeatShield>,
    },
    Kind {
        of: |_, _| None,
        actions: &[],
        name: "thermal",
        load: load::<Thermal>,
    },
];

/// The systems of a vessel, by the part each is on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Systems {
    parts: BTreeMap<PartId, Vec<Box<dyn System>>>,
    /// The parts with systems on them that power and data pass between.
    networks: Vec<Vec<PartId>>,
}

/// One system's part in one power network for one step.
struct Flow {
    part: PartId,
    /// Where it is among its part's systems.
    index: usize,
    /// W.
    source: f64,
    /// It is a source, whatever it makes now.
    is_source: bool,
    /// What it could make if asked, W.
    dispatch: f64,
    /// kJ.
    charge: f64,
    capacity: f64,
    /// W, and its priority.
    load: Option<(f64, u8)>,
}

/// A reading with where it comes from, for a report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartReading {
    pub part: PartId,
    pub kind: &'static str,
    pub reading: Reading,
}

impl Systems {
    /// The systems of `tree`'s parts, as they were built.
    pub fn of(tree: &PartTree) -> Self {
        let mut parts: BTreeMap<PartId, Vec<Box<dyn System>>> = BTreeMap::new();
        for part in tree.parts() {
            let Some(built) = &part.built else {
                continue;
            };
            for component in &built.components {
                for kind in KINDS {
                    if let Some(system) = (kind.of)(component, part) {
                        parts.entry(part.id).or_default().push(system);
                    }
                }
            }
            // Last on the part: what its heat shield leaves, it takes.
            if let Some(thermal) = Thermal::of_part(part) {
                parts.entry(part.id).or_default().push(Box::new(thermal));
            }
        }
        let mut systems = Self {
            parts,
            networks: Vec::new(),
        };
        systems.wire(tree);
        systems
    }

    /// Work the networks out again over `tree`'s joints, after parts came off.
    pub fn wire(&mut self, tree: &PartTree) {
        self.networks = (tree.power_groups().into_iter())
            .map(|group| {
                let on = |part: &PartId| self.parts.contains_key(part);
                group.into_iter().filter(on).collect::<Vec<_>>()
            })
            .filter(|group| !group.is_empty())
            .collect();
    }

    /// The systems on `part`.
    pub fn on(&self, part: PartId) -> &[Box<dyn System>] {
        self.parts.get(&part).map_or(&[], Vec::as_slice)
    }

    /// Every system of type `T`, with its part.
    pub fn all<T: 'static>(&self) -> impl Iterator<Item = (PartId, &T)> {
        self.parts.iter().flat_map(|(&part, systems)| {
            systems
                .iter()
                .filter_map(move |s| Some((part, s.any().downcast_ref::<T>()?)))
        })
    }

    /// Take the systems on parts `gone` says are gone: they leave with their parts.
    pub fn split_off(&mut self, gone: impl Fn(PartId) -> bool) -> Systems {
        let (leaving, staying) = std::mem::take(&mut self.parts)
            .into_iter()
            .partition(|(part, _)| gone(*part));
        self.parts = staying;
        Systems {
            parts: leaving,
            networks: Vec::new(),
        }
    }

    /// Put `other`'s systems here too, each on the part `id` calls its part now. [`Self::wire`]
    /// is for the caller, once the parts are joined.
    pub(crate) fn absorb(&mut self, other: Systems, id: impl Fn(PartId) -> PartId) {
        self.parts
            .extend(other.parts.into_iter().map(|(part, on)| (id(part), on)));
    }

    /// Call every part what `id` says.
    pub(crate) fn renumber(&mut self, id: impl Fn(PartId) -> PartId) {
        self.parts = std::mem::take(&mut self.parts)
            .into_iter()
            .map(|(part, on)| (id(part), on))
            .collect();
    }

    /// Drop the systems on parts no longer on `tree` (torn off, not let go), and wire up what
    /// is left.
    pub fn retain_on(&mut self, tree: &PartTree) {
        self.parts.retain(|&part, _| tree.get(part).is_some());
        self.wire(tree);
    }

    /// Put a system on `part` of `tree`.
    pub fn add(&mut self, tree: &PartTree, part: PartId, system: Box<dyn System>) {
        self.parts.entry(part).or_default().push(system);
        self.wire(tree);
    }

    /// Have the system on `part` that offers `action` do it.
    pub fn act(&mut self, part: PartId, action: Action) -> Option<String> {
        let systems = self.parts.get_mut(&part)?;
        let system = systems
            .iter_mut()
            .find(|s| s.actions().iter().any(|a| a.kind == action.kind))?;
        system.act(action)
    }

    /// One tick of flight for every system. `heat` is what falls on each part; what is left of a
    /// part's heat when its systems have had it is gone.
    pub fn fly(
        &mut self,
        flight: &FlightNow,
        heat: &BTreeMap<PartId, PartHeat>,
        dt: f64,
    ) -> Vec<String> {
        let mut said = Vec::new();
        for (part, systems) in &mut self.parts {
            let mut here = heat.get(part).copied().unwrap_or_default();
            for system in systems {
                system.fly(flight, &mut here, dt);
                said.extend(system.announce());
            }
        }
        said
    }

    /// Whether a part has got hotter than the vessel can stand.
    pub fn overheated(&self) -> bool {
        self.all::<Thermal>().any(|(_, thermal)| thermal.failed())
    }

    /// Everyone aboard, alive.
    pub fn crew_aboard(&self) -> u32 {
        self.all::<Crew>().map(|(_, crew)| crew.aboard).sum()
    }

    /// Everyone lost.
    pub fn crew_lost(&self) -> u32 {
        self.all::<Crew>().map(|(_, crew)| crew.lost).sum()
    }

    /// Move one person to the seats on `to` from the others on this vessel. Whether it could.
    pub fn board(&mut self, to: PartId) -> bool {
        let room = (self.parts.get(&to).into_iter().flatten())
            .filter_map(|s| s.any().downcast_ref::<Crew>())
            .any(|crew| crew.aboard < crew.seats);
        if !room {
            return false;
        }
        let from = self.parts.iter_mut().filter(|(part, _)| **part != to);
        let mut moved = false;
        for (_, systems) in from {
            if systems.iter_mut().any(|s| s.take_person()) {
                moved = true;
                break;
            }
        }
        if moved && let Some(systems) = self.parts.get_mut(&to) {
            for system in systems {
                if system.seat_person() {
                    break;
                }
            }
        }
        moved
    }

    /// Move every system on `dt` seconds: first the power networks, under `environment`.
    ///
    /// `supplied`: the vessel stands at a launch site on its ground support, which powers every
    /// load and keeps the stores full, so its fuel cells stand idle.
    pub fn step(&mut self, environment: &Environment, dt: f64, supplied: bool) {
        for k in 0..self.networks.len() {
            let network = std::mem::take(&mut self.networks[k]);
            if supplied {
                self.supply_power(&network, environment, dt);
            } else {
                self.solve_power(&network, environment, dt);
            }
            self.networks[k] = network;
        }
        for system in self.parts.values_mut().flatten() {
            system.step(dt);
        }
    }

    /// Run one power network for `dt` seconds. What it makes goes to the loads, the highest
    /// priority first, then what its fuel cells can give and its stores hold; a load the energy
    /// does not reach is shed. The fuel cells then make what the loads still want and the stores
    /// have room for, in proportion to what they are rated for, and no more; what is over from
    /// the rest charges the stores by the room they have.
    fn solve_power(&mut self, network: &[PartId], environment: &Environment, dt: f64) {
        let flows = self.flows(network, environment);
        let made: f64 = flows.iter().map(|f| f.source).sum();
        let can_make: f64 = flows.iter().map(|f| f.dispatch).sum();
        let stored: f64 = flows.iter().map(|f| f.charge).sum();
        let room: f64 = flows.iter().map(|f| f.capacity - f.charge).sum();
        let (renewed, can_burn) = (made * dt / 1e3, can_make * dt / 1e3);
        // Energy there is for the step, kJ.
        let mut left = renewed + can_burn + stored;
        let mut loads: Vec<(usize, f64, u8)> = (flows.iter().enumerate())
            .filter_map(|(k, f)| f.load.map(|(watts, priority)| (k, watts, priority)))
            .collect();
        loads.sort_by(|a, b| b.2.cmp(&a.2));
        let mut served = vec![true; flows.len()];
        let mut drawn = 0.0;
        for &(k, watts, _) in &loads {
            let wanted = watts * dt / 1e3;
            // A store runs out at a known time, which time warp stops at to the tick: a share
            // of a joule more or less there is not a shed load, nor a sliver left of the store.
            if wanted <= left * (1.0 + 1e-6) + 1e-9 {
                left = (left - wanted).max(0.0);
                drawn += wanted;
            } else {
                served[k] = false;
            }
        }
        let burned = (drawn + room - renewed).clamp(0.0, can_burn);
        let net = renewed + burned - drawn;
        // Into the stores by the room they have, out by what they hold.
        for (k, flow) in flows.iter().enumerate() {
            let system = &mut self.parts.get_mut(&flow.part).expect("its part")[flow.index];
            if flow.load.is_some() {
                system.powered(served[k]);
            }
            if can_make > 0.0 && flow.dispatch > 0.0 {
                system.deliver(burned * flow.dispatch / can_make, dt);
            }
            if flow.source > 0.0 || flow.is_source {
                system.deliver(flow.source * dt / 1e3, dt);
            }
            if flow.capacity > 0.0 {
                let share = if net >= 0.0 {
                    if room > 0.0 {
                        ((flow.capacity - flow.charge) / room * net)
                            .min(flow.capacity - flow.charge)
                    } else {
                        0.0
                    }
                } else if stored > 0.0 {
                    flow.charge / stored * net
                } else {
                    0.0
                };
                system.charge(share);
            }
        }
    }

    /// Run one power network for `dt` seconds on ground power: every load has it, the stores are
    /// topped up, and what the vessel makes itself goes nowhere.
    fn supply_power(&mut self, network: &[PartId], environment: &Environment, dt: f64) {
        for flow in self.flows(network, environment) {
            let system = &mut self.parts.get_mut(&flow.part).expect("its part")[flow.index];
            if flow.load.is_some() {
                system.powered(true);
            }
            if flow.source > 0.0 || flow.is_source {
                system.deliver(flow.source * dt / 1e3, dt);
            }
            if flow.capacity > flow.charge {
                system.charge(flow.capacity - flow.charge);
            }
        }
    }

    fn flows(&self, network: &[PartId], environment: &Environment) -> Vec<Flow> {
        let mut flows = Vec::new();
        for &part in network {
            for (index, system) in self.on(part).iter().enumerate() {
                let Some(power) = system.power(environment) else {
                    continue;
                };
                let mut flow = Flow {
                    part,
                    index,
                    source: 0.0,
                    is_source: false,
                    dispatch: 0.0,
                    charge: 0.0,
                    capacity: 0.0,
                    load: None,
                };
                match power {
                    Power::Source(watts) => (flow.source, flow.is_source) = (watts, true),
                    Power::Dispatchable(watts) => flow.dispatch = watts,
                    Power::Store { charge, capacity } => {
                        (flow.charge, flow.capacity) = (charge, capacity);
                    }
                    Power::Load { watts, priority } => flow.load = Some((watts, priority)),
                }
                flows.push(flow);
            }
        }
        flows
    }

    /// Seconds until the first system needs attention, or a power network's stores run out with
    /// what it draws now.
    pub fn next_event(&self, environment: &Environment, supplied: bool) -> Option<f64> {
        let own = (self.parts.values().flatten()).filter_map(|s| s.next_event());
        let run_out = (!supplied)
            .then(|| self.first_run_out(environment))
            .flatten();
        own.chain(run_out).reduce(f64::min)
    }

    /// Seconds until the first of the power networks runs out, with what each draws now.
    fn first_run_out(&self, environment: &Environment) -> Option<f64> {
        let empty = self.networks.iter().filter_map(|network| {
            let flows = self.flows(network, environment);
            let made: f64 = flows.iter().map(|f| f.source + f.dispatch).sum();
            let stored: f64 = flows.iter().map(|f| f.charge).sum();
            let wanted: f64 = (flows.iter()).filter_map(|f| f.load.map(|(w, _)| w)).sum();
            (stored > 1e-6 && wanted > made).then(|| stored * 1e3 / (wanted - made))
        });
        empty.reduce(f64::min)
    }

    /// Each store of power on the vessel: its part, what it holds and what it can hold, kJ.
    pub fn stores(&self) -> impl Iterator<Item = (PartId, f64, f64)> + '_ {
        self.all::<Battery>()
            .map(|(part, battery)| (part, battery.charge, battery.capacity))
    }

    /// How the vessel's power stands under `environment`, `supplied` from the ground or not.
    pub fn power_state(&self, environment: &Environment, supplied: bool) -> PowerState {
        let (charge, capacity) = self
            .stores()
            .fold((0.0, 0.0), |(charge, capacity), (_, held, most)| {
                (charge + held, capacity + most)
            });
        let shedding = self.parts.values().flatten().any(|s| s.shed());
        PowerState {
            charge,
            capacity,
            // Already gone: the time to the end of a store that loads are being kept from says
            // nothing, and sits still.
            runs_out_in: (self.first_run_out(environment)).filter(|_| !shedding && !supplied),
            shedding,
            browned_out: self.brownout() || self.all::<Avionics>().any(|(_, unit)| !unit.on),
            supplied,
        }
    }

    /// The parts with a system that burns what is in the tanks.
    pub fn consumers(&self) -> Vec<PartId> {
        (self.parts.iter())
            .filter(|(_, systems)| systems.iter().any(|s| s.consumes()))
            .map(|(&part, _)| part)
            .collect()
    }

    /// The systems on `part`, to change.
    pub(crate) fn on_mut(&mut self, part: PartId) -> &mut [Box<dyn System>] {
        self.parts.get_mut(&part).map_or(&mut [], Vec::as_mut_slice)
    }

    /// Whether a load the vessel cannot do without has no power.
    pub fn brownout(&self) -> bool {
        self.parts.values().flatten().any(|s| s.browned_out())
    }

    /// Whether any system needs to know the sunlight.
    pub fn uses_light(&self) -> bool {
        self.parts.values().flatten().any(|s| s.uses_light())
    }

    /// Every system's state, for a report.
    pub fn readings(&self) -> Vec<PartReading> {
        self.parts
            .iter()
            .flat_map(|(&part, systems)| {
                systems.iter().flat_map(move |s| {
                    let kind = s.kind();
                    s.state().into_iter().map(move |reading| PartReading {
                        part,
                        kind,
                        reading,
                    })
                })
            })
            .collect()
    }

    /// The sum of each reading over the vessel, by `<kind>_<reading>`: what a report charts.
    pub fn totals(&self) -> BTreeMap<String, f64> {
        let mut totals = BTreeMap::new();
        for r in self.readings() {
            *totals
                .entry(format!("{}_{}", r.kind, r.reading.id))
                .or_insert(0.0) += r.reading.value;
        }
        totals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::CraftDef,
        propulsion::Propulsion,
        staging::{Action, Activation, Sequences},
        test_data::records,
        vessel::tests::catalog,
    };

    /// A pad of two stages with a battery on the lower one and seats on the upper.
    fn craft() -> CraftDef {
        records(
            r#"{ id: "two", name: "", parts: [
                { id: "cabin", shape: { shell: { bottom_diameter: 1, sections: [{ length: 2 }],
                    cap_bottom: true, cap_top: true } },
                  structure: { mass: 200 }, components: [{ container: {} }, { crew: { seats: 2 } }] },
                { id: "ring", name: "Decoupler", on: "cabin.bottom", by: "top",
                  shape: { ring: { diameter: 1, height: 0.04, thickness: 0.02 } },
                  structure: { mass: 3 }, components: [{ decoupler: { node: "top" } }] },
                { id: "lower", on: "ring.bottom", by: "top",
                  shape: { shell: { bottom_diameter: 1, sections: [{ length: 2 }],
                    cap_bottom: true, cap_top: true } },
                  structure: { mass: 100 },
                  components: [{ container: {} }, { battery: { charge: 3600 } }] },
            ] }"#,
        )
        .remove(0)
    }

    fn propulsion() -> Propulsion {
        Propulsion::from_craft(&craft(), &catalog()).unwrap()
    }

    #[test]
    fn every_part_keeps_its_systems() {
        let p = propulsion();
        let charge: f64 = p.systems.all::<Battery>().map(|(_, b)| b.charge).sum();
        let seats: u32 = p.systems.all::<Crew>().map(|(_, c)| c.seats).sum();
        assert_eq!((charge, seats), (3600.0, 2));
        let totals = p.systems.totals();
        assert_eq!(totals["battery_charge"], 3600.0);
    }

    #[test]
    fn a_batterys_charge_stays_with_its_part_through_a_separation() {
        let mut p = propulsion();
        // Use some, then let the lower stage go with its battery.
        let (part, _) = p.systems.all::<Battery>().next().unwrap();
        let staged = p.activate_next_sequence().expect("a sequence");
        let [released] = staged.released.as_slice() else {
            panic!("{}", staged.released.len());
        };
        assert_eq!(p.systems.all::<Battery>().count(), 0);
        assert_eq!(p.systems.all::<Crew>().count(), 1);
        let debris = Propulsion::from_released(released.clone());
        let (on, battery) = debris.systems.all::<Battery>().next().unwrap();
        assert_eq!((on, battery.charge), (part, 3600.0));
    }

    #[test]
    fn parameters_are_read_without_knowing_the_system() {
        let p = propulsion();
        // What an editor's property panel does: it knows no battery.
        let shown: Vec<(&str, f64, &str)> = p
            .parts()
            .parts()
            .iter()
            .flat_map(|part| p.systems.on(part.id))
            .flat_map(|s| s.parameters())
            .map(|r| (r.label, r.value, r.unit))
            .collect();
        assert!(shown.contains(&("Capacity", 3600.0, "kJ")), "{shown:?}");
        assert!(shown.contains(&("Seats", 2.0, "")), "{shown:?}");
    }

    /// A source that makes `watts` while the sun shines on it, for the network's tests.
    #[derive(Debug, Clone, Serialize)]
    struct Source {
        watts: f64,
    }

    /// A load that remembers whether it has power.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Load {
        watts: f64,
        priority: u8,
        on: bool,
    }

    impl System for Source {
        fn kind(&self) -> &'static str {
            "source"
        }

        fn parameters(&self) -> Vec<Reading> {
            Vec::new()
        }

        fn state(&self) -> Vec<Reading> {
            Vec::new()
        }

        fn power(&self, environment: &Environment) -> Option<Power> {
            Some(Power::Source(self.watts * environment.sunlit))
        }

        fn boxed(&self) -> Box<dyn System> {
            Box::new(self.clone())
        }

        fn any(&self) -> &dyn Any {
            self
        }
    }

    impl System for Load {
        fn kind(&self) -> &'static str {
            "load"
        }

        fn parameters(&self) -> Vec<Reading> {
            Vec::new()
        }

        fn state(&self) -> Vec<Reading> {
            Vec::new()
        }

        fn power(&self, _environment: &Environment) -> Option<Power> {
            Some(Power::Load {
                watts: self.watts,
                priority: self.priority,
            })
        }

        fn powered(&mut self, on: bool) {
            self.on = on;
        }

        fn shed(&self) -> bool {
            !self.on
        }

        fn boxed(&self) -> Box<dyn System> {
            Box::new(self.clone())
        }

        fn any(&self) -> &dyn Any {
            self
        }
    }

    /// Full sun, or none.
    fn light(sunlit: f64) -> Environment {
        Environment {
            sun_direction: glam::DVec3::X,
            sun_distance: 1.5e11,
            sun_in_vessel: glam::DVec3::X,
            sunlit,
            solar_flux: 1361.0 * sunlit,
            albedo_flux: 0.0,
            infrared_flux: 0.0,
        }
    }

    /// The lower stage of the test craft with a source and two loads beside its battery of
    /// 3600 kJ: 1 kW high priority, 500 W low.
    fn powered_stage() -> (Propulsion, PartId) {
        let mut p = propulsion();
        let (part, _) = p.systems.all::<Battery>().next().unwrap();
        add(&mut p, part, Box::new(Source { watts: 200.0 }));
        for (watts, priority) in [(1000.0, 9), (500.0, 1)] {
            let load = Load {
                watts,
                priority,
                on: true,
            };
            add(&mut p, part, Box::new(load));
        }
        (p, part)
    }

    fn add(p: &mut Propulsion, part: PartId, system: Box<dyn System>) {
        let tree = p.parts().clone();
        p.systems.add(&tree, part, system);
    }

    fn step(p: &mut Propulsion, environment: &Environment, dt: f64) {
        p.systems.step(environment, dt, false);
    }

    fn charge(p: &Propulsion) -> f64 {
        p.systems.all::<Battery>().map(|(_, b)| b.charge).sum()
    }

    fn loads_on(p: &Propulsion) -> Vec<(u8, bool)> {
        p.systems
            .all::<Load>()
            .map(|(_, l)| (l.priority, l.on))
            .collect()
    }

    #[test]
    fn a_network_runs_on_its_source_and_its_battery() {
        let (mut p, _) = powered_stage();
        // 1500 W wanted, 200 W made: 1300 W out of the battery, 1.3 kJ a second.
        step(&mut p, &light(1.0), 100.0);
        assert!(
            (charge(&p) - (3600.0 - 130.0)).abs() < 1e-9,
            "{}",
            charge(&p)
        );
        assert_eq!(loads_on(&p), [(9, true), (1, true)]);
    }

    #[test]
    fn the_lowest_priority_is_shed_first() {
        let (mut p, _) = powered_stage();
        // All of it until the battery runs out, which the network can say beforehand.
        let empty = p.systems.next_event(&light(1.0), false).unwrap();
        assert!((empty - 3600.0 / 1.3).abs() < 1e-6, "{empty}");
        step(&mut p, &light(1.0), empty);
        assert!(charge(&p).abs() < 1e-6, "{}", charge(&p));
        assert_eq!(loads_on(&p), [(9, true), (1, true)]);
        // With nothing stored the 200 W reach neither: both go, the low one's turn would have
        // come first.
        step(&mut p, &light(1.0), 1.0);
        assert_eq!(loads_on(&p), [(9, false), (1, false)]);
        // A bigger source runs the high priority load and not the low.
        let (mut p, part) = powered_stage();
        add(&mut p, part, Box::new(Source { watts: 900.0 }));
        step(&mut p, &light(1.0), 3600.0 / 0.4);
        assert!(charge(&p).abs() < 1e-6, "{}", charge(&p));
        step(&mut p, &light(1.0), 1.0);
        assert_eq!(loads_on(&p), [(9, true), (1, false)]);
    }

    /// A jump rounded up to the next tick still runs the store out, not leaving a sliver that
    /// would bring time warp back every microsecond.
    #[test]
    fn a_jump_a_little_long_runs_the_store_out() {
        let (mut p, _) = powered_stage();
        let empty = p.systems.next_event(&light(1.0), false).unwrap();
        step(&mut p, &light(1.0), empty * (1.0 + 1e-7));
        assert_eq!(loads_on(&p), [(9, true), (1, true)]);
        assert!(charge(&p) < 1e-3, "{}", charge(&p));
    }

    #[test]
    fn the_power_state_says_when_it_runs_out_and_is_right() {
        let (mut p, part) = powered_stage();
        // 3600 kJ, with 1500 W drawn and 200 W made in the light.
        let state = p.systems.power_state(&light(1.0), false);
        assert_eq!((state.charge, state.capacity), (3600.0, 3600.0));
        assert!(!state.browned_out);
        let left = state.runs_out_in.unwrap();
        assert!((left - 3_600_000.0 / 1300.0).abs() < 1e-6, "{left}");
        // Half way there it says half as long.
        step(&mut p, &light(1.0), left / 2.0);
        let later = p.systems.power_state(&light(1.0), false);
        assert!((later.charge - 1800.0).abs() < 1e-6, "{later:?}");
        assert!((later.runs_out_in.unwrap() - left / 2.0).abs() < 1e-6);
        assert!(!later.shedding);
        // At the time it gave, it is out, and what it told of is what happens.
        step(&mut p, &light(1.0), left / 2.0);
        let empty = p.systems.power_state(&light(1.0), false);
        assert!(empty.charge < 1e-6, "{empty:?}");
        assert!(empty.is_flat());
        assert!(!later.is_flat());
        assert_eq!(empty.runs_out_in, None);
        // Then the loads go, and what the source still makes does not start a countdown again.
        step(&mut p, &light(1.0), 1.0);
        let out = p.systems.power_state(&light(1.0), false);
        assert!(out.shedding, "{out:?}");
        assert!(out.charge > 0.0, "{out:?}");
        assert_eq!(out.runs_out_in, None);
        // Making more than it draws, it is not running down.
        add(&mut p, part, Box::new(Source { watts: 2500.0 }));
        assert_eq!(p.systems.power_state(&light(1.0), false).runs_out_in, None);
    }

    #[test]
    fn avionics_left_without_power_are_a_brownout_and_end_the_countdown() {
        let (mut p, part) = powered_stage();
        let unit = Avionics {
            controlled_mass: 1e6,
            watts: 300.0,
            interplanetary: false,
            on: true,
        };
        add(&mut p, part, Box::new(unit));
        assert!(!p.systems.power_state(&light(1.0), false).browned_out);
        // A step longer than the batteries last serves no load, and leaves the store as it was with
        // the avionics off: what the countdown stuck at under time warp.
        step(&mut p, &light(1.0), 1e5);
        let out = p.systems.power_state(&light(1.0), false);
        assert!(out.browned_out && out.shedding, "{out:?}");
        assert_eq!(out.runs_out_in, None);
    }

    #[test]
    fn the_stores_are_listed_by_part() {
        let (p, part) = powered_stage();
        let stores: Vec<_> = p.systems.stores().collect();
        assert_eq!(stores, [(part, 3600.0, 3600.0)]);
    }

    #[test]
    fn a_source_charges_its_battery_and_stops_at_full() {
        let (mut p, part) = powered_stage();
        add(&mut p, part, Box::new(Source { watts: 2500.0 }));
        step(&mut p, &light(1.0), 100.0);
        // Full already: what is over goes nowhere.
        assert_eq!(charge(&p), 3600.0);
        // Half empty, it takes it in: 2700 W made, 1500 W drawn.
        step(&mut p, &light(0.0), 1200.0);
        assert!(
            (charge(&p) - (3600.0 - 1800.0)).abs() < 1e-9,
            "{}",
            charge(&p)
        );
        step(&mut p, &light(1.0), 100.0);
        assert!(
            (charge(&p) - (1800.0 + 120.0)).abs() < 1e-9,
            "{}",
            charge(&p)
        );
    }

    #[test]
    fn a_stage_let_go_takes_its_network_with_it() {
        let (mut p, _) = powered_stage();
        let staged = p.activate_next_sequence().unwrap();
        let debris = Propulsion::from_released(staged.released[0].clone());
        // What is left has no power of its own; what left runs on.
        assert_eq!(p.systems.all::<Load>().count(), 0);
        let mut debris = debris;
        step(&mut debris, &light(1.0), 100.0);
        assert!((charge(&debris) - (3600.0 - 130.0)).abs() < 1e-9);
    }

    /// A craft of plain parts: `body` on a shell, with hydrogen and oxygen tanks beside it.
    fn powered(body: &str) -> Propulsion {
        let text = format!(
            r#"{{ id: "power", name: "", parts: [
                {{ id: "host", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 4 }}],
                    cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 500 }},
                  components: [{{ container: {{}} }}, {body}],
                  nodes: [{{ name: "h2", position: [0, 0, 1], direction: [0, 0, 1] }},
                          {{ name: "o2", position: [0, 0, 3], direction: [0, 0, 1] }}] }},
                {{ id: "hydrogen", on: "host.h2", shape: {{ sphere: {{ diameter: 0.8 }} }},
                  components: [{{ storage: {{ type: "Insulated Al-Li",
                    contents: {{ LqdHydrogen: 1 }} }} }}] }},
                {{ id: "oxygen", on: "host.o2", shape: {{ sphere: {{ diameter: 0.7 }} }},
                  components: [{{ storage: {{ type: "Insulated Al-Li",
                    contents: {{ LqdOxygen: 1 }} }} }}] }}
            ] }}"#
        );
        let craft: CraftDef = records(&text).remove(0);
        Propulsion::from_craft(&craft, &catalog()).unwrap()
    }

    fn held(p: &Propulsion, resource: &str) -> f64 {
        p.tanks.iter().map(|t| t.held(resource)).sum()
    }

    /// A kWh burns 42 g of hydrogen and eight times that of oxygen, and a fuel cell makes only
    /// what is asked of it.
    #[test]
    fn a_fuel_cell_burns_what_the_load_asks() {
        let mut p =
            powered(r#"{ fuel_cell: { power: 1500 } }, { electric_load: { watts: 1000 } }"#);
        let (h2, o2) = (held(&p, "LqdHydrogen"), held(&p, "LqdOxygen"));
        p.step_systems(&light(1.0), 3600.0, false);
        let (used_h2, used_o2) = (h2 - held(&p, "LqdHydrogen"), o2 - held(&p, "LqdOxygen"));
        assert!((used_h2 - 0.042).abs() < 1e-6, "{used_h2}");
        assert!((used_o2 - 8.0 * 0.042).abs() < 1e-5, "{used_o2}");
        let (_, cell) = p.systems.all::<FuelCell>().next().unwrap();
        assert!((cell.output - 1000.0).abs() < 1e-6, "{}", cell.output);
    }

    /// With its tanks dry a fuel cell stops, and with no other power the vessel browns out; time
    /// warp stops where they run dry.
    #[test]
    fn a_dry_fuel_cell_stops_and_the_load_it_fed_goes_dark() {
        let mut p = powered(
            r#"{ fuel_cell: { power: 1500 } },
               { electric_load: { watts: 1000, essential: true } }"#,
        );
        assert!(!p.systems.brownout());
        p.step_systems(&light(1.0), 1.0, false);
        let to_dry = p.next_system_event(&light(1.0), false).unwrap();
        let hours = held(&p, "LqdOxygen").min(8.0 * held(&p, "LqdHydrogen")) / (8.0 * 0.042);
        assert!((to_dry - hours * 3600.0).abs() < 1.0, "{to_dry} {hours}");
        for tank in &mut p.tanks {
            tank.amounts.iter_mut().for_each(|a| *a = 0.0);
        }
        p.step_systems(&light(1.0), 1.0, false);
        p.step_systems(&light(1.0), 1.0, false);
        assert!(p.systems.brownout());
        let (_, load) = p.systems.all::<electric::Load>().next().unwrap();
        assert!(!load.on);
    }

    /// A fuel cell keeps the battery on its network full, and carries on with what is over.
    #[test]
    fn a_fuel_cell_tops_up_a_battery() {
        let mut p = powered(
            r#"{ fuel_cell: { power: 1500 } }, { electric_load: { watts: 200 } },
               { battery: { charge: 3600 } }"#,
        );
        let part = p.systems.all::<Battery>().next().unwrap().0;
        // Run it down a little by hand, as a spell without the cell would.
        p.systems.on_mut(part)[2].charge(-1000.0);
        assert!((charge(&p) - 2600.0).abs() < 1e-9);
        p.step_systems(&light(1.0), 100.0, false);
        // 1300 W over for 100 s: 130 kJ.
        assert!((charge(&p) - 2730.0).abs() < 1e-6, "{}", charge(&p));
    }

    #[test]
    fn a_solar_array_makes_what_the_sun_gives_it() {
        let mut p = powered(
            r#"{ solar_array: { area: 10, efficiency: 0.2, tracking: true, degradation: 0 } },
               { electric_load: { watts: 2000 } }, { battery: { charge: 1000 } }"#,
        );
        // 1361 W/m² × 10 m² × 0.2 = 2722 W: all the load wants and more.
        let (_, array) = p.systems.all::<SolarArray>().next().unwrap();
        assert_eq!(array.area, 10.0);
        p.step_systems(&light(1.0), 10.0, false);
        assert_eq!(charge(&p), 1000.0);
        // In the shadow it makes nothing, and the battery takes over.
        p.step_systems(&light(0.0), 10.0, false);
        assert!((charge(&p) - 980.0).abs() < 1e-6, "{}", charge(&p));
        // A panel that does not track makes what it faces: edge on, nothing.
        let mut fixed = powered(
            r#"{ solar_array: { area: 10, efficiency: 0.2, degradation: 0 } },
               { electric_load: { watts: 100 } }, { battery: { charge: 1000 } }"#,
        );
        let mut edge = light(1.0);
        edge.sun_in_vessel = glam::DVec3::Y;
        fixed.step_systems(&edge, 10.0, false);
        assert!((charge(&fixed) - 999.0).abs() < 1e-6, "{}", charge(&fixed));
        assert!(fixed.systems.uses_light());
    }

    #[test]
    fn an_rtg_falls_with_its_half_life() {
        let mut p = powered(r#"{ rtg: { power: 158 } }"#);
        const YEAR: f64 = 365.25 * 86400.0;
        p.step_systems(&light(0.0), 87.7 * YEAR, false);
        let (_, rtg) = p.systems.all::<Rtg>().next().unwrap();
        assert!((rtg.output() - 79.0).abs() < 1e-6, "{}", rtg.output());
    }

    /// With no avionics and no crew, a vessel in a game has no control; a crew flies a small one
    /// by hand and a unit what it is built for.
    #[test]
    fn a_vessel_in_a_game_needs_something_to_fly_it() {
        let craft = |components: &str, mass: f64| -> Propulsion {
            let text = format!(
                r#"{{ id: "x", name: "", parts: [{{ id: "bus",
                    shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 2 }}],
                        cap_bottom: true, cap_top: true }} }},
                    structure: {{ mass: {mass} }},
                    components: [{{ container: {{}} }} {components}] }}] }}"#
            );
            let mut p =
                Propulsion::from_craft(&records::<CraftDef>(&text).remove(0), &catalog()).unwrap();
            p.require_control();
            p
        };
        assert!(!craft("", 1000.0).controlled(true));
        assert!(craft(", { crew: { seats: 1 } }", 1000.0).controlled(true));
        assert!(!craft(", { crew: { seats: 1 } }", 40_000.0).controlled(true));
        let unit = ", { avionics: { controlled_mass: 2000 } }";
        assert!(craft(unit, 1000.0).controlled(true));
        assert!(!craft(unit, 3000.0).controlled(true));
        // A near-Earth level is no use beyond the Earth's sphere of influence.
        assert!(!craft(unit, 1000.0).controlled(false));
        // And what it leaves behind on its own has none.
        let free = Propulsion::from_craft(
            &records::<CraftDef>(
                r#"{ id: "x", name: "", parts: [{ id: "bus",
                    shape: { shell: { bottom_diameter: 1, sections: [{ length: 2 }],
                        cap_bottom: true, cap_top: true } },
                    structure: { mass: 100 }, components: [{ container: {} }] }] }"#,
            )
            .remove(0),
            &catalog(),
        )
        .unwrap();
        assert!(
            free.controlled(true),
            "a craft outside a game flies as it did"
        );
    }

    /// Without avionics rules any pod or unit flies whatever it is on, powered or not, and with
    /// neither the vessel takes no commands at all.
    #[test]
    fn without_avionics_rules_any_pod_flies_and_no_pod_takes_no_commands() {
        let rules = crate::data::RulesDef {
            avionics: false,
            ..Default::default()
        };
        let craft = |components: &str, mass: f64| -> Propulsion {
            let text = format!(
                r#"{{ id: "x", name: "", parts: [{{ id: "bus",
                    shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 2 }}],
                        cap_bottom: true, cap_top: true }} }},
                    structure: {{ mass: {mass} }},
                    components: [{{ container: {{}} }} {components}] }}] }}"#
            );
            let mut p =
                Propulsion::from_craft(&records::<CraftDef>(&text).remove(0), &catalog()).unwrap();
            p.require_control_under(&rules);
            p
        };
        let pod = craft(", { crew: { seats: 1 } }", 500_000.0);
        assert!(pod.controlled(true) && !pod.no_pod());
        assert!(pod.controlled(false), "and anywhere");
        let unit = craft(", { avionics: { controlled_mass: 1 } }", 500_000.0);
        assert!(unit.controlled(true) && !unit.no_pod());
        let nothing = craft("", 1000.0);
        assert!(!nothing.controlled(true) && nothing.no_pod());
        // A game with the rules is as it was.
        let mut limited = craft(", { crew: { seats: 1 } }", 500_000.0);
        limited.require_control_under(&crate::data::RulesDef::default());
        assert!(!limited.controlled(true) && !limited.no_pod());
    }

    /// A part with avionics or a crew offers to be flown from, and the vessel's control frame is
    /// then that part's.
    #[test]
    fn a_vessel_can_be_flown_from_a_part_turned_on_it() {
        let craft = records::<CraftDef>(
            r#"{ id: "x", name: "", parts: [
                { id: "bus", shape: { shell: { bottom_diameter: 1, sections: [{ length: 2 }],
                    cap_bottom: true, cap_top: true } },
                  structure: { mass: 100 },
                  components: [{ container: {} }, { avionics: { controlled_mass: 5000 } }],
                  nodes: [{ name: "side", position: [0.5, 0, 1], direction: [1, 0, 0] }] },
                { id: "cupola", on: "bus.side", by: "bottom",
                  shape: { ring: { diameter: 0.5, height: 0.5, thickness: 0.25 } },
                  structure: { mass: 10 }, components: [{ crew: { seats: 1 } }] }
            ] }"#,
        )
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        assert_eq!(p.control_rotation(), glam::DQuat::IDENTITY);
        let cupola = p.parts().parts()[1].id;
        let fly_from = Activation::new(cupola, Action::CONTROL);
        assert!(p.part_actions(cupola, false).contains(&fly_from));
        assert!(p.perform(fly_from, false).is_some());
        // Its +Z, the way it stands out from the bus, is the nose now.
        let nose = p.control_rotation() * glam::DVec3::Z;
        assert!((nose - glam::DVec3::X).length() < 1e-9, "{nose}");
        // It is not offered twice.
        assert!(!p.part_actions(cupola, false).contains(&fly_from));
    }

    /// A system nobody has heard of, with an action nobody has heard of.
    #[derive(Debug, Clone, Serialize)]
    struct Wiggler {
        wiggles: u32,
    }

    const WIGGLE: Action = Action::new("wiggle", "Wiggle");

    impl System for Wiggler {
        fn kind(&self) -> &'static str {
            "wiggler"
        }

        fn parameters(&self) -> Vec<Reading> {
            Vec::new()
        }

        fn state(&self) -> Vec<Reading> {
            vec![Reading {
                id: "wiggles",
                label: "Wiggles",
                value: f64::from(self.wiggles),
                unit: "",
            }]
        }

        fn actions(&self) -> Vec<Action> {
            vec![WIGGLE]
        }

        fn act(&mut self, _action: Action) -> Option<String> {
            self.wiggles += 1;
            Some("It wiggled".into())
        }

        fn boxed(&self) -> Box<dyn System> {
            Box::new(self.clone())
        }

        fn any(&self) -> &dyn Any {
            self
        }
    }

    /// A part's window, a sequence and the staging panel's icons all carry an action they have
    /// never heard of, by what the system says of it.
    #[test]
    fn an_action_nobody_has_heard_of_goes_through_like_any_other() {
        let mut p = propulsion();
        let part = p.parts().root().unwrap().id;
        add(&mut p, part, Box::new(Wiggler { wiggles: 0 }));
        let wiggle = Activation::new(part, WIGGLE);
        // On offer in the window, with its name.
        assert!(p.part_actions(part, false).contains(&wiggle));
        assert_eq!(wiggle.verb(), "Wiggle");
        // In a sequence, it is an icon of its own kind.
        p.sequences = Sequences::new(vec![vec![wiggle]]);
        let seq = p.sequences.list()[0].clone();
        let groups = p.groups(&seq);
        assert_eq!(groups[0].kind, "wiggle");
        // Fired, the system does it and says so.
        let staged = p.activate_next_sequence().unwrap();
        assert_eq!(staged.messages, ["It wiggled"]);
        assert_eq!(p.systems.totals()["wiggler_wiggles"], 1.0);
        // Done by hand as well.
        assert!(p.perform(wiggle, false).is_some());
        assert_eq!(p.systems.totals()["wiggler_wiggles"], 2.0);
    }

    #[test]
    fn a_component_nobody_knows_is_an_error_that_names_it() {
        let text = r#"{ id: "odd", name: "", parts: [
            { id: "x", shape: { ring: { diameter: 1, height: 1 } },
              components: [{ flux_capacitor: {} }] } ] }"#;
        let error = json5::from_str::<CraftDef>(text).unwrap_err().to_string();
        assert!(error.contains("flux_capacitor"), "{error}");
    }

    #[test]
    fn a_system_is_saved_under_its_kind_and_reads_back_as_itself() {
        let battery: Box<dyn System> = Box::new(Battery {
            capacity: 2_000.0,
            charge: 1_500.0,
        });
        let saved = rmp_serde::to_vec_named(&battery).unwrap();
        let read: Box<dyn System> = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(read.kind(), "battery");
        assert_eq!(
            read.any().downcast_ref(),
            battery.any().downcast_ref::<Battery>()
        );

        // One nobody has heard of cannot be read: better that than a vessel missing a system.
        let wiggler: Box<dyn System> = Box::new(Wiggler { wiggles: 3 });
        let saved = rmp_serde::to_vec_named(&wiggler).unwrap();
        let read = rmp_serde::from_slice::<Box<dyn System>>(&saved);
        assert!(read.unwrap_err().to_string().contains("wiggler"));
    }

    #[test]
    fn every_kind_of_system_can_be_found_by_the_name_it_goes_by() {
        let mut names: Vec<_> = KINDS.iter().map(|kind| kind.name).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two kinds share a name");
    }
}
