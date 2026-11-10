<!--
Pasted by the user on 2026-11-10 as the brief for T260. Kept close to how it was given: long
one-item-a-line lists in sections 4, 5 and 11 to 15 are joined into sentences, nothing is added. It is a design
reference for the KSP mod Kerbalism, not a source of numbers: the figures in it are illustrative or
unchecked (the "default-profile" daily rates in 3.1 are far below what people need), and where it
says something is "core Kerbalism" that was not checked against the mod. No Kerbalism code or data
is copied into the game. The tasks under T260 cite its sections as "spec §n.n".
-->

# Kerbalism: technical feature specification for implementation in a new game

The following is a software-oriented breakdown of Kerbalism, treating it as an independent spacecraft simulation system.

The objective is to describe what each subsystem does, what state it maintains, how it interacts with other subsystems, and what an implementation needs to reproduce its behavior.

This is intended to be usable as input for coding agents implementing the systems in a completely different game engine.

The central design principle is:
A spacecraft is a continuously operating system composed of interconnected components. Every component has resource requirements, operating conditions, state, and consequences when it cannot function.

Kerbalism can be understood as a collection of simulation subsystems sharing a common spacecraft model, resource network, time system, and environmental model.

## 1. Core architecture

Before implementing individual features, establish the shared simulation architecture. This is more important than any individual mechanic.

I recommend six foundational systems.

- Simulation Clock: universal elapsed time, time acceleration, simulation intervals
- Vessel Simulation Manager: schedules and updates every active and inactive spacecraft
- Resource Network
- Environment Model
- Crew & Habitat
- Component System
- Science System
- Communications
- Persistent Simulation State: resources, crew health, experiment progress, failures, data and automation state

### 1.1 Entity model

Everything should be represented as entities with properties and simulation behavior.

| Entity | Responsibilities |
| --- | --- |
| Vessel | Contains spacecraft components and crew |
| Component | Generates, consumes or transforms resources |
| Resource | Represents a consumable or producible material |
| Crew member | Has biological and psychological state |
| Habitat | Defines the internal living environment |
| Experiment | Collects scientific data |
| Data storage | Stores scientific datasets |
| Antenna | Provides communication capabilities |
| Network node | Represents a communication endpoint |
| Environment | Supplies radiation, temperature, pressure and illumination |
| Process | Converts resource inputs into outputs |
| Automation script | Executes actions based on conditions |
| Failure state | Tracks component malfunction and repair |

### 1.2 Simulation update model

Do not implement each feature as an independent real-time loop.

Instead, implement a unified simulation scheduler.

For each simulation interval:

1. Determine environmental conditions for each vessel.
2. Calculate available power.
3. Calculate resource production and consumption.
4. Update internal habitat conditions.
5. Update crew biological and psychological state.
6. Advance scientific experiments.
7. Advance communication and transmission.
8. Update component reliability.
9. Evaluate automation conditions.
10. Persist state and generate notifications.

Some operations have dependencies. For example, a scrubber requires electricity and affects cabin CO₂, which affects crew health.

You should therefore model resource dependencies explicitly rather than relying on arbitrary update order.

### 1.3 Background simulation

This is a fundamental architectural requirement.

Every vessel must continue simulating regardless of whether it is currently being rendered or controlled.

The simulation should use elapsed time rather than frame time.

For example:

- A spacecraft runs for 30 minutes.
- The player switches to another spacecraft.
- The original spacecraft is not loaded.
- Thirty minutes later, all its systems have advanced by the appropriate amount.

Implement an event-driven or fixed-step simulation with catch-up support.

For performance, you can aggregate elapsed time and analytically integrate simple processes instead of simulating every second.

For example, if oxygen consumption is constant:

\[ O_2(t)=O_2(0)-r_{O_2}t \]

If the tank becomes empty before the interval ends, calculate the exact depletion time and apply the consequences for the remaining interval.

Kerbalism explicitly supports background simulation of resource processing, science, communications and other systems.

## 2. Resource management system

Foundational subsystem. This should be implemented before life support, greenhouses, laboratories or ISRU.

The resource system represents materials and energy that spacecraft components consume, produce, store and transform.

### 2.1 Resource properties

Every resource should have:

| Property | Description |
| --- | --- |
| ID | Unique identifier |
| Name | Display name |
| Unit | kg, liters, kWh, etc. |
| Density | Mass per unit volume |
| Capacity | Maximum amount a container can hold |
| Amount | Current quantity |
| Flow rules | Which components can access it |
| Resource type | Consumable, producible, energy, etc. |

Example resource definitions:

```json
{
  "id": "oxygen",
  "name": "Oxygen",
  "unit": "kg",
  "density": 1.141,
  "type": "consumable",
  "flowMode": "connected_tanks"
}
```

### 2.2 Resource containers

A vessel contains resource containers.

Each container has:

- Maximum capacity.
- Current quantity.
- Resource type.
- Connection to a resource network.
- Optional priority.

A component requesting oxygen should draw it from accessible containers.

### 2.3 Resource processes

The most important abstraction is the process.

A process is a transformation:

\[ \text{Inputs} \rightarrow \text{Outputs} \]

For example:

\[ CO_2 + 2H_2 \rightarrow C + 2H_2O \]

A process definition should contain:

```json
{
  "id": "water_recovery",
  "inputs": {
    "waste_water": 0.8,
    "electricity": 0.15
  },
  "outputs": {
    "water": 0.65
  },
  "powerRequired": 0.15,
  "duration": 1.0,
  "conditions": {
    "minimumPressure": 0.5
  }
}
```

The numerical values here are illustrative, not Kerbalism's actual rates.

### 2.4 Resource solver

The resource solver must:

1. Determine available inputs.
2. Determine process priorities.
3. Calculate the maximum possible production.
4. Ensure inputs are not consumed twice.
5. Respect container capacities.
6. Calculate unused outputs and bottlenecks.
7. Update all resource quantities.

A useful feature is to support competing processes.

For example, a spacecraft has 100 units of oxygen:

- Crew consumes 2 units/hour.
- An experiment consumes 1 unit/hour.
- A manufacturing process consumes 3 units/hour.

The solver must distribute resources according to priority and availability.

Important: Never let each component independently subtract resources without a central accounting mechanism. Otherwise, simultaneous processes can consume the same resource twice.

## 3. Life-support system

Foundational gameplay subsystem. Life support simulates the biological requirements of crew members and the equipment necessary to maintain survivable conditions.

It consists of four major systems:

1. Biological consumption.
2. Atmospheric management.
3. Waste management.
4. Food production.

### 3.1 Biological consumption

Every crew member has a consumption profile.

Kerbalism's default life-support model includes three essential resources:

| Resource | Consumed by | Produced by |
| --- | --- | --- |
| Food | Eating | Greenhouse |
| Water | Drinking | Water recycling |
| Oxygen | Breathing | Greenhouse / electrolysis |

Biological waste products include:

| Output | Source |
| --- | --- |
| Waste | Eating |
| WasteWater | Drinking |
| WasteAtmosphere | Breathing |

The simulation calculates consumption per crew member per unit of elapsed time.

\[ C_{\text{total}}=N_{\text{crew}}\times C_{\text{individual}}\times t \]

Where \(N_{\text{crew}}\) is the crew count, \(C_{\text{individual}}\) is the consumption rate, and \(t\) is elapsed time.

Kerbalism's default-profile values are documented as approximately 0.07375 kg food, 0.14 kg water and 0.05287 kg oxygen per person per day. These should be configurable rather than hardcoded.

### 3.2 Depletion consequences

Resource depletion should have separate consequences.

For example:

- Food shortage: starvation.
- Water shortage: dehydration.
- Oxygen shortage: suffocation.

Each should have its own exposure duration, health degradation and eventual fatality threshold.

Do not instantly kill crew when a tank reaches zero.

Implement a health state machine:

```
NORMAL
  |
  v
RESOURCE_SHORTAGE
  |
  v
PHYSIOLOGICAL_STRESS
  |
  v
CRITICAL_CONDITION
  |
  v
DEATH
```

The exact thresholds should be configurable.

### 3.3 Atmospheric management

A habitat needs an atmosphere with appropriate properties.

Track:

- Total pressure.
- Oxygen partial pressure.
- CO₂ concentration.
- Nitrogen concentration.
- Humidity.
- Temperature.

Use partial pressures rather than simply tracking percentages.

For a gas mixture:

\[ P_i=x_iP_{\text{total}} \]

Where \(x_i\) is the gas's mole fraction.

Components can modify the atmosphere:

| Component | Function |
| --- | --- |
| Oxygen supply | Adds oxygen |
| Nitrogen supply | Maintains pressure |
| CO₂ scrubber | Removes carbon dioxide |
| Humidity control | Regulates water vapor |
| Ventilation | Circulates cabin atmosphere |
| Pressure regulator | Maintains target pressure |

### 3.4 Waste management

Waste is not necessarily useless.

Implement conversion chains that recover useful resources.

Examples:

- Wastewater → water.
- Organic waste → ammonia.
- CO₂ → oxygen through biological or chemical processes.
- Hydrogen + CO₂ → water and other by-products.

These processes should have efficiency losses.

A closed-loop life-support system should never produce unlimited resources without inputs or energy.

### 3.5 Greenhouse

A greenhouse is a biological production component.

It consumes water, CO₂, nutrients, and electricity when artificial lighting is required.

It produces food and oxygen.

Its productivity depends on light intensity, radiation, pressure, temperature, available inputs, and available output storage.

Natural sunlight should reduce artificial lighting consumption.

This is an important implementation detail: the greenhouse should not simply run at a constant output rate.

## 4. Habitat system

The habitat system models the environment in which crew members live.

It is distinct from life support: life support maintains the environment; the habitat system describes its properties and how suitable it is for human habitation.

### 4.1 Habitat volume

Each habitat has an internal volume.

Calculate volume per person:

\[ V_{\text{per capita}}=\frac{V_{\text{habitat}}}{N_{\text{crew}}} \]

Low volume per person increases psychological stress.

### 4.2 Pressure

A habitat can be pressurized, partially pressurized, or depressurized.

Pressure affects crew survivability, greenhouse operation, gas behavior, and comfort.

### 4.3 Comfort

Comfort is a combination of environmental and equipment-based modifiers.

| Factor | Effect |
| --- | --- |
| Living space | More space improves comfort |
| Gravity | Suitable gravity improves comfort |
| Exercise equipment | Improves physical wellbeing |
| Entertainment | Improves psychological wellbeing |
| Environmental conditions | Poor conditions reduce comfort |

### 4.4 Gravity

Gravity can be supplied by natural gravitational fields, artificial rotating habitats, or other gravity-generating systems.

A rotating habitat needs to calculate effective acceleration:

\[ a=\omega^2r \]

Where \(\omega\) is angular velocity and \(r\) is distance from the rotation axis.

### 4.5 Psychological condition

Crew members accumulate stress over time.

Stress should depend on habitat volume, comfort, gravity, environmental conditions, and duration of exposure.

Kerbalism uses psychological breakdowns as a consequence of inadequate living conditions. The precise model can be simplified for a new implementation while retaining the important relationship between habitat design and crew wellbeing.

## 5. Radiation and environmental system

Independent simulation subsystem. The environment system provides external conditions to every spacecraft.

It should be a world-level service, not something each spacecraft calculates independently.

### 5.1 Environmental properties

| Property | Description |
| --- | --- |
| Temperature | Ambient environmental temperature |
| Radiation | Background radiation intensity |
| Pressure | Ambient atmospheric pressure |
| Light intensity | Available solar illumination |
| Atmospheric density | Density of surrounding atmosphere |
| Magnetic field | Radiation protection modifier |
| Gravity | Local gravitational acceleration |

### 5.2 Radiation sources

- Galactic cosmic radiation: persistent background radiation originating outside the local star system.
- Stellar radiation: radiation produced by the star.
- Radiation belts: localized regions of increased radiation around certain celestial bodies.
- Solar particle events: temporary radiation increases caused by stellar activity.
- Surface radiation: radiation emitted by planetary surfaces and local radioactive materials.

### 5.3 Radiation exposure

Crew members accumulate radiation over time.

\[ D=\int_0^t \dot D(\tau)\,d\tau \]

Where \(D\) is accumulated dose and \(\dot D\) is dose rate.

Shielding reduces incoming radiation. A simple model:

\[ \dot D_{\text{effective}}=\dot D_{\text{external}}\times S \]

Where \(S\) is a shielding transmission factor between 0 and 1.

A more realistic implementation should calculate shielding based on material, thickness, and radiation type.

### 5.4 Radiation effects

Radiation can cause increased crew radiation dose, acute radiation sickness, long-term health degradation, increased component failure rates, temporary communication interruptions, and reduced greenhouse productivity.

### 5.5 Temperature

Components should have operating temperature ranges. A component may operate normally, lose efficiency, consume additional power for thermal control, or shut down when temperature exceeds limits.

Temperature should be affected by solar exposure, planetary infrared radiation, internal heat generation, radiators, insulation, and thermal mass.

### 5.6 Solar storms

Implement solar storms as time-dependent environmental events.

Each event should have start time, duration, intensity, spatial distribution, radiation increase, and communication effects.

This allows spacecraft to encounter different hazards depending on their location and the timing of the mission.

## 6. Scientific data acquisition system

Major subsystem. This is not simply a research-points generator. It is a system for collecting, storing, analyzing, transporting and eventually delivering scientific information.

There are three separate concepts:

1. An experiment generates measurements or samples.
2. A storage device holds the resulting data.
3. A communications system transports eligible data to a destination.

These must be independent systems.

### 6.1 Experiment definitions

| Property | Description |
| --- | --- |
| ID | Unique experiment identifier |
| Duration | Required collection time |
| Data rate | Amount of data generated per second |
| Power consumption | Electricity required |
| Required conditions | Environment in which it can operate |
| Required equipment | Instrument or component requirement |
| Data type | Measurement or physical sample |
| Maximum output | Maximum data or sample quantity |
| Repeatability | Whether it can collect additional datasets |
| Analysis requirements | Whether laboratory processing is necessary |

Example:

```json
{
  "id": "orbital_magnetometer",
  "name": "Orbital Magnetic Survey",
  "type": "measurement",
  "duration": 86400,
  "dataRate": 0.5,
  "powerRate": 0.02,
  "conditions": {
    "minimumAltitude": 100000,
    "maximumAltitude": 1000000,
    "requiresOperation": true
  },
  "output": {
    "dataType": "magnetic_field_measurements",
    "requiresAnalysis": false
  }
}
```

Values are illustrative.

### 6.2 Experiment lifecycle

```
AVAILABLE -> READY -> RUNNING -> DATA_GENERATED
DATA_GENERATED -> TRANSMISSIBLE -> TRANSMITTING -> RECEIVED
DATA_GENERATED -> REQUIRES_ANALYSIS -> IN_LABORATORY -> ANALYSIS_COMPLETE
  -> TRANSMISSIBLE -> TRANSMITTING -> RECEIVED
```

The experiment must continuously evaluate its environmental requirements. For example, an atmospheric sensor may only operate when atmospheric pressure is above a certain threshold. If conditions become invalid, the experiment pauses rather than generating invalid data.

### 6.3 Two types of scientific output

A. Measurement data: digital information (temperature, radiation, magnetic field, atmospheric composition, pressure). It can be stored digitally and transmitted through a communication network.

B. Physical samples: actual collected material (rock, soil, atmospheric, biological). They have physical mass and occupy storage capacity. They cannot be transmitted as ordinary digital information. They must be physically transported, recovered, or processed through a suitable laboratory.

### 6.4 Data storage

A spacecraft should have one or more storage devices. Each has capacity in bytes, current occupancy, supported data types, read/write availability, reliability, and optional environmental restrictions.

A dataset should contain:

```json
{
  "id": "dataset_001",
  "experimentId": "orbital_magnetometer",
  "createdAt": 123456789,
  "sizeBytes": 524288,
  "quality": 1.0,
  "analysisRequired": false,
  "transmissionStatus": "pending",
  "destination": "mission_control"
}
```

Important behaviors:

- Experiments stop or pause when storage is full.
- Datasets can be deleted.
- Data can be transferred between connected storage devices.
- Damaged storage may result in data loss.
- Data transmission should not automatically delete the original dataset unless configured to do so.

### 6.5 Laboratories

A laboratory is a processing facility for scientific samples or datasets requiring analysis. It has processing capacity, power consumption, crew requirements if applicable, input resource requirements, processing duration, and output data type.

```
Physical sample -> Laboratory (+ electricity, processing time, optional crew) -> Analyzed digital dataset
```

Laboratory analysis must not create arbitrary scientific value. It transforms an existing sample into a useful scientific result.

### 6.6 Experiment quality

Not all experiments produce equally useful data. Model quality using instrument quality, calibration, environmental stability, collection duration, signal-to-noise ratio, and experiment-specific conditions. For example, a magnetometer may generate better measurements when it is sufficiently far from magnetic interference. The experiment's output quality can influence its scientific value.

### 6.7 Scientific value

Keep scientific value separate from the data storage system. A dataset can have physical size, scientific importance, quality, processing status, and delivery status. This allows the game to support scientific progression without making every byte of data equivalent to the same amount of research.

Kerbalism's defining behavior is that experiments collect data over time, continue operating in the background, and distinguish between digital measurements and physical samples.

## 7. Communications and signal network

Major subsystem. This system models the ability of spacecraft to communicate with other spacecraft, stations and ground infrastructure. It has two distinct responsibilities:

1. Communication availability: can two nodes establish a connection?
2. Communication throughput: how much information can they transfer?

Do not combine these into a single boolean property.

### 7.1 Communication nodes

Every communication-capable entity is a network node: spacecraft, ground stations, relay satellites, space stations, planetary bases. Each node can have multiple antennas.

### 7.2 Antenna properties

| Property | Description |
| --- | --- |
| Maximum range | Maximum supported communication distance |
| Transmit power | Signal transmission power |
| Gain | Antenna directional gain |
| Frequency | Supported communication frequency |
| Data rate | Maximum transmission throughput |
| Directionality | Omnidirectional or directional |
| Power consumption | Energy used during operation |
| Deployment state | Whether the antenna is operational |

### 7.3 Signal strength

A simplified free-space signal model uses the Friis transmission equation:

\[ P_r=P_tG_tG_r\left(\frac{\lambda}{4\pi d}\right)^2 \]

The system should also account for atmospheric attenuation, obstacles, antenna orientation, interference, noise, and weather events.

### 7.4 Line of sight

A connection requires a valid path between communication nodes. If a planet blocks the path, communication is unavailable. A relay satellite can provide an alternative route.

### 7.5 Network routing

A communication network can be represented as a graph \(G=(V,E)\). A spacecraft can communicate with mission control through multiple relay nodes (remote probe -> relay satellite A -> orbital station -> ground station). Every connection has its own bandwidth. The effective throughput is limited by the slowest connection along the route, accounting for contention and scheduling.

### 7.6 Data transmission

Transmission is a continuous process: \(T=\frac{S}{R}\) for dataset size \(S\) and data rate \(R\). The actual transmission rate should change dynamically when distance changes, antennas move, a relay becomes unavailable, signal quality deteriorates, or other spacecraft consume bandwidth.

### 7.7 Communication failures

Possible states: connected, weak signal, intermittent connection, blocked, out of range, power unavailable, antenna malfunction. A transmission should pause when the connection disappears and resume when connectivity returns.

### 7.8 Communication blackouts

Environmental events such as solar storms can interrupt communication. These events should be able to increase signal noise, reduce throughput, and completely disable communication temporarily.

### 7.9 Telemetry versus scientific data

These are separate types of network traffic. Telemetry contains operational information (position, power, fuel, component health, temperature, crew condition). Scientific data contains collected measurements and experiment results. Telemetry should be allowed priority over bulk scientific data.

Kerbalism's communication model specifically emphasizes signal strength, relaying, data-rate attenuation and transmission that continues while spacecraft are inactive.

## 8. Component reliability and failure system

This system introduces uncertainty into spacecraft operation. Components are not guaranteed to work forever.

### 8.1 Reliability properties

| Property | Description |
| --- | --- |
| MTBF | Mean time between failures |
| Operational age | Total operating time |
| Failure probability | Probability of failure over time |
| Quality | Manufacturing quality |
| Radiation tolerance | Radiation level before accelerated degradation |
| Repairability | Whether it can be repaired |
| Repair requirements | Crew skill, tools and resources |
| Failure consequences | Which functions become unavailable |

### 8.2 Failure probability

A simple exponential reliability model is \(R(t)=e^{-\lambda t}\) with \(\lambda=\frac{1}{MTBF}\).

However, Kerbalism's current reliability model does not use a simple constant hazard across the entire lifetime: it guarantees roughly 50% of MTBF without failure and raises the failure probability afterward, approaching certainty around 150% of MTBF. For a new implementation, you can choose either model, but the latter better reproduces Kerbalism's characteristic behavior.

### 8.3 Component states

```
OPERATIONAL -> MALFUNCTION -> REPAIRABLE -> REPAIRING -> OPERATIONAL
                           -> CRITICAL FAILURE -> PERMANENTLY BROKEN
```

A malfunction should disable the component's functionality. A critical failure may permanently disable it.

### 8.4 Manufacturing quality

| Quality | Cost | Mass | MTBF |
| --- | --- | --- | --- |
| Low | Low | Low | Short |
| Standard | Standard | Standard | Normal |
| High | Higher | Higher | Longer |

The exact trade-offs should be configurable.

### 8.5 Redundancy

Suppose a spacecraft has two power generators. If either one fails, the other can continue generating electricity. Support redundancy groups:

```json
{
  "componentId": "generator_a",
  "redundancyGroup": "main_power",
  "requiredOperationalUnits": 1
}
```

### 8.6 Radiation-induced failures

A component should have a radiation tolerance. Above that tolerance, its remaining time to failure decreases faster.

### 8.7 Repair

Repair operations should require a repairable failure, appropriate tools, available crew with the necessary skill, time, and optionally replacement parts. A repair can have a success probability or take a deterministic amount of time.

### 8.8 Propulsion-specific reliability

Optional extension: maximum ignition count, maximum cumulative burn time, ignition failure probability, shutdown reliability, restart restrictions.

## 9. ISRU and industrial resource production

ISRU means In-Situ Resource Utilization: extracting and processing resources directly from the environment. It should be implemented as an extension of the resource network.

### 9.1 Resource deposits

```json
{
  "resource": "water_ice",
  "concentration": 0.12,
  "remainingQuantity": 100000,
  "extractionDifficulty": 0.7,
  "depth": 2.5
}
```

Properties: resource type, concentration, total available quantity, extraction difficulty, required equipment, environmental restrictions.

### 9.2 Harvesters

A harvester consumes electricity, time and potentially consumable equipment; it produces extracted resources, waste materials and heat. Production depends on resource concentration and equipment efficiency.

### 9.3 Processing facilities

```
Water ice -> Extraction -> Water -> Electrolysis -> Hydrogen + Oxygen
Atmospheric CO2 -> Processing -> Oxygen
```

These chains should be data-driven. You should be able to define new chemical processes without writing new simulation code.

### 9.4 Resource distribution

Resources should vary by location: water ice in polar regions, atmospheric gases on planets, minerals in geological deposits, volatile compounds in certain environments. The harvester queries the distribution based on its position.

### 9.5 Industrial scaling

Support multiple harvesters operating simultaneously. Production is constrained by deposit availability, power, processing capacity, storage, and environmental conditions.

## 10. Automation and scripting system

Automation allows components to react to conditions without requiring player interaction. It is essentially a small event-driven scripting language.

### 10.1 Basic structure

An automation script consists of a trigger, conditions, actions, an optional cooldown, and execution priority.

```json
{
  "id": "solar_panel_protection",
  "trigger": { "event": "enter_atmosphere" },
  "conditions": [],
  "actions": [ { "component": "solar_panels", "action": "retract" } ]
}
```

### 10.2 Event types

| Event | Meaning |
| --- | --- |
| Landed | Vessel becomes landed |
| Atmosphere | Vessel enters atmosphere |
| Space | Vessel enters space |
| Sunlight | Star becomes visible |
| Shadow | Star becomes occluded |
| Power high | Battery exceeds a threshold |
| Power low | Battery drops below a threshold |
| Radiation high | Radiation exceeds threshold |
| Radiation low | Radiation drops below threshold |
| Linked | Communication is restored |
| Unlinked | Communication is lost |
| Storage full | Data storage approaches capacity |
| Storage empty | Data storage is nearly empty |

### 10.3 Actions

Activate or deactivate a component, extend or retract an antenna, start or stop an experiment, start or stop a resource processor, activate a greenhouse, disable unnecessary systems, change operating mode.

### 10.4 Conditional scripts

```
IF battery < 20%
AND solar_panels == deployed
AND sunlight == true
THEN disable_nonessential_systems
```

### 10.5 Script execution

Scripts must work when the spacecraft is not actively controlled. The automation engine should evaluate events in the background simulation. Avoid allowing arbitrary user scripts to execute unrestricted game code: a constrained action/condition language is safer, easier to serialize and easier to debug.

## 11. Vessel monitoring and management interface

This is the centralized control and monitoring layer. It should not own simulation logic.

### 11.1 Vessel overview

Every known spacecraft and its operational state: name, location, power, oxygen, food, data storage, communication, crew, component health.

### 11.2 Vessel detail panels

- Resources: quantities, capacity, consumption and production rates, time until depletion, bottlenecks.
- Crew: individual health, food, water and oxygen status, radiation dose, stress, habitat conditions.
- Science: active experiments, progress, datasets, storage, pending analysis, transmission queue.
- Communications: connection, signal strength, bandwidth, relay route, transmission progress.
- Reliability: operational and malfunctioning components, reliability estimates, repair options, redundancy.
- Automation: running scripts, triggers, last execution, component states.

### 11.3 Remote control

The player should be able to control supported components remotely (stop an experiment, enable a greenhouse, start a processor, retract an antenna, disable a malfunctioning system). A spacecraft with no available communication connection should not accept arbitrary remote commands.

### 11.4 Alerts

Notifications when important conditions occur: oxygen expected to run out in 12 hours, battery below 15%, experiment storage full, solar panel malfunction, radiation exposure over safe limits, communication link lost, greenhouse production interrupted. Alerts should be generated by simulation events, not by polling the UI.

## 12. Mission and spacecraft design planner

A predictive simulation tool to evaluate a spacecraft design before it is deployed. It should reuse the same simulation systems as the actual game.

- 12.1 Resource planning: food, water, oxygen, waste, electricity, storage, recycling efficiency for a crew and duration (\(R_{\text{required}}=N \times r \times t\)).
- 12.2 Habitat planning: living volume, volume per crew member, pressure, comfort, shielding, thermal control.
- 12.3 Reliability planning: failure probabilities, redundancy, single points of failure, expected lifetime.
- 12.4 Communications planning: coverage, availability, relay connectivity, transmission time, storage.
- 12.5 Radiation planning: expected exposure across a planned trajectory.
- 12.6 Mission simulation: simulate a proposed mission over time, for example a 180-day mission of 4 crew, with predicted problems ("water recycling efficiency is insufficient", "one power generator is a single point of failure", "storage capacity is insufficient for the planned experiments").

## 13. Data-driven configuration system

Kerbalism is designed around configurable profiles, component definitions, resource rules and feature toggles. Reproduce that rather than hardcoding every component and process.

### 13.1 Separate engine from configuration

The engine understands generic concepts (consumption, production, environmental conditions, reliability, experiments, storage, communication, crew health). The configuration defines how they behave. A new scrubber should not need a new subsystem:

```json
{
  "id": "co2_scrubber",
  "type": "resource_processor",
  "inputs": { "electricity": 0.05, "waste_atmosphere": 0.02 },
  "outputs": { "captured_co2": 0.018 },
  "conditions": { "minimumPressure": 0.1 }
}
```

### 13.2 Suggested configuration structure

resources/, components/ (life_support, science, communication, reliability), processes/, experiments/, environments/ (radiation zones, temperature, atmosphere), automation/ (events, actions), profiles/.

### 13.3 Feature toggles

Every major system should be independently configurable:

```json
{
  "features": {
    "lifeSupport": true,
    "psychology": true,
    "radiation": true,
    "reliability": true,
    "science": true,
    "communications": true,
    "isru": true,
    "automation": true,
    "backgroundSimulation": true
  }
}
```

A configuration profile should also be able to change numerical values without modifying engine code.

### 13.4 Component interface

```ts
interface SimulatedComponent {
  id: string;
  initialize(): void;
  simulate(deltaTime: number, context: SimulationContext): void;
  getState(): ComponentState;
  save(): SerializedComponentState;
  load(state: SerializedComponentState): void;
}
```

Components communicate through services, not by modifying unrelated components: a greenhouse requests water and electricity through the resource network.

## 14. Additional technical considerations

- 14.1 Component emulation: existing components (a solar panel) take part through adapters, with one implementation for loaded and background simulation.
- 14.2 Dynamic habitat configuration: enabling or disabling a habitat compartment changes volume, surface, crew capacity, pressure needs, shielding and atmosphere quantities.
- 14.3 Time acceleration: no system may behave differently with warp (consumption, missed failures, early experiments, lost data, skipped triggers). Process threshold crossings explicitly.
- 14.4 Persistence: resources, crew condition, dose, operating age, failure state, experiment progress, datasets, transmission progress, automation state, environmental event state. Do not reconstruct them from the configuration.
- 14.5 Deterministic simulation: seeded randomness for failures, solar storms and random events.
- 14.6 Performance: central scheduling, update frequency groups, analytical resource calculations, event-driven discrete events, cached environment, cheaper inactive vessels with the same outcomes.

## 15. Recommended implementation order

1. Simulation foundation: clock, entity model, persistence, background scheduler, save/load.
2. Resource network: definitions, containers, flow, production and consumption, process solver, power.
3. Crew and life support: consumption, waste, atmosphere, CO₂ removal, water recycling, health.
4. Habitat and environment: volume, pressure and humidity, temperature, comfort and stress, radiation, shielding.
5. Science: experiments, lifecycle, storage, samples, laboratories, quality.
6. Communications: antennas, propagation, line of sight, relays, bandwidth, transmission.
7. Reliability: MTBF, malfunctions, critical failures, redundancy, repairs, radiation degradation.
8. Industrial systems: deposits, harvesters, chemical processing, greenhouses, recycling.
9. Automation: events, conditions, actions, persistence, background execution.
10. Management and planning UI: vessel monitor, crew, science, comms, resource planning, mission simulation.

## 16. Acceptance tests for coding agents

| Test | Expected behavior |
| --- | --- |
| Background consumption | Crew consumes resources while vessel is inactive |
| Resource depletion | Crew health deteriorates when oxygen runs out |
| Resource competition | Two processes cannot consume the same available resource |
| Recycling | Wastewater processing produces water according to configured efficiency |
| Greenhouse | Production stops when required resources are unavailable |
| Atmospheric poisoning | CO₂ increases when crew breathe without sufficient scrubbing |
| Habitat pressure | Pressure decreases when atmosphere leaks |
| Radiation | Crew dose accumulates over time |
| Shielding | Greater shielding reduces radiation exposure |
| Solar storm | Radiation and communication conditions change during an event |
| Experiment | Data accumulates over the configured duration |
| Full storage | Experiment pauses when storage capacity is exhausted |
| Physical sample | Sample cannot be transmitted as digital data |
| Laboratory | Analysis converts an eligible sample into a dataset |
| Communication | Transmission pauses when the signal is lost |
| Relay | A relay can provide an alternative communication route |
| Reliability | Components can malfunction after sufficient operating time |
| Redundancy | A backup component maintains system availability after primary failure |
| Repair | A repair operation restores an eligible component |
| Automation | A threshold crossing triggers the configured action |
| Save/load | Simulation state is preserved exactly across reload |
| Time acceleration | Simulated outcomes remain consistent across different time-step sizes |

## 17. What is actually Kerbalism versus proposed extensions

| System | Classification |
| --- | --- |
| Background simulation | Core Kerbalism |
| Resource consumption/production | Core Kerbalism |
| Life support | Core Kerbalism |
| Habitat and pressure | Core Kerbalism |
| Comfort and stress | Core Kerbalism |
| Radiation and shielding | Core Kerbalism |
| Solar weather | Core Kerbalism |
| Science over time | Core Kerbalism |
| Digital data and physical samples | Core Kerbalism |
| Laboratories | Core Kerbalism |
| Communications and relays | Core Kerbalism |
| Component failures | Core Kerbalism |
| Manufacturing quality | Core Kerbalism |
| Automation | Core Kerbalism |
| ISRU | Core Kerbalism |
| Vessel monitor and planner | Core Kerbalism |
| General-purpose deterministic simulation | Implementation recommendation |
| Generic JSON component architecture | Implementation recommendation |
| Advanced network routing | Implementation extension |
| Detailed radiation damage model | Implementation extension |
| Advanced mission prediction | Implementation extension |

The feature classification is based on the project's documentation and source repository.
