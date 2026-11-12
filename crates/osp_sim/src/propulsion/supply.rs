//! The vessel's one account of what its systems take from its tanks and give back, every step.
//!
//! A step goes in two passes. Before the power networks run, each system that uses the tanks says
//! the most it would take and give ([`System::wants`]), and the tanks as they stand decide what
//! share of that each can have ([`System::plan`]), so a process asks its network for the power of
//! that share only and a fuel cell offers no more than its tanks can feed. After the networks have
//! run, each takes and gives what it used ([`System::uses`]) the same way, and is told the share it
//! got ([`System::exchanged`]).
//!
//! Who gets what: everything in a step runs together, as it would through the step, so a scrubber
//! takes out of a cabin's air what the crew breathe into it in the same step, and a recycler's water
//! is there for them to drink. Where what there is and what others make falls short of what is
//! taken, the takers are cut back, the highest priority keeping the most and each priority sharing
//! in proportion to what it wants; where the room and what others take falls short of what is made,
//! the makers are, unless the output is one they may dump. The shares are worked out from the bins
//! at the start of the step and hold for all of it, so the step is exact up to
//! [`Propulsion::next_system_event`], which is where a tank someone draws on runs dry or one someone
//! fills is full.
//!
//! What a system keeps can also leak away, a share of what there is each second (a cabin's air
//! through a hole, [`System::decay`]). With what goes in and out besides steady over the step, what
//! is left follows e^-kt towards where the two balance, which the step works out exactly; a maker
//! filling such a store can make what leaks out of it besides the room it has.
//!
//! [`System::wants`]: crate::systems::System::wants
//! [`System::plan`]: crate::systems::System::plan
//! [`System::uses`]: crate::systems::System::uses
//! [`System::exchanged`]: crate::systems::System::exchanged

use std::collections::{BTreeMap, BTreeSet};

use super::Propulsion;
use crate::{
    environment::Environment,
    parts::PartId,
    systems::{Exchange, System},
};

/// Less than this is nothing, kg.
const TRACE: f64 = 1e-9;

/// The lots of one system.
struct Lots {
    part: PartId,
    /// Which of its part's systems.
    index: usize,
    lots: Vec<Exchange>,
}

/// One lot, and the tanks it reaches: `tiers`, those furthest up the fuel lines first, and all of
/// them in `feeds`.
struct Lot<'a> {
    tiers: Vec<Vec<usize>>,
    feeds: Vec<usize>,
    exchange: &'a Exchange,
}

/// What the tanks hold as the lots run, apart from them, so that a plan touches nothing: the
/// vessel's tanks first, by their index, and then what systems keep for their part (a cabin's air
/// and lockers), so many bins each.
struct Store {
    /// Each bin's resources and how much of each it holds when full, kg.
    contents: Vec<Vec<(String, f64)>>,
    amounts: Vec<Vec<f64>>,
    /// How many of the bins are tanks.
    tanks: usize,
    /// The part and system each bin after the tanks is.
    systems: Vec<(PartId, usize)>,
    /// The share of what each slot holds that it loses a second, 1/s ([`System::decay`]).
    decay: Vec<Vec<f64>>,
    /// What the bins held after the lots ran and before anything leaked, as of the last
    /// [`allocate`].
    linear: Vec<Vec<f64>>,
}

impl Store {
    fn new(p: &Propulsion) -> Self {
        let mut contents: Vec<Vec<(String, f64)>> = (p.tanks.iter())
            .map(|t| {
                (t.contents.iter())
                    .map(|c| (c.resource.clone(), c.capacity))
                    .collect()
            })
            .collect();
        let mut amounts: Vec<Vec<f64>> = p.tanks.iter().map(|t| t.amounts.clone()).collect();
        let mut decay: Vec<Vec<f64>> = amounts.iter().map(|a| vec![0.0; a.len()]).collect();
        let mut systems = Vec::new();
        for (part, on) in p.systems.by_part() {
            for (index, system) in on.iter().enumerate() {
                let stored = system.stored();
                if stored.is_empty() {
                    continue;
                }
                contents.push(
                    stored
                        .iter()
                        .map(|(r, _, full)| (r.clone(), *full))
                        .collect(),
                );
                amounts.push(stored.iter().map(|(_, held, _)| *held).collect());
                let mut lost = system.decay();
                lost.resize(stored.len(), 0.0);
                decay.push(lost);
                systems.push((part, index));
            }
        }
        Self {
            contents,
            amounts,
            tanks: p.tanks.len(),
            systems,
            decay,
            linear: Vec::new(),
        }
    }

    /// What leaks out of `bins`' `resource` a second as they stand, kg/s.
    fn leaking(&self, bins: &[usize], resource: &str) -> f64 {
        (self.slots(bins, resource))
            .map(|(k, c, _)| self.decay[k][c] * self.amounts[k][c])
            .sum()
    }

    /// Let each slot that leaks lose what it does over `dt`, from what it held at `start`, with
    /// what the lots put in and took out since spread evenly over the step: from `m0` with `s`
    /// going in a second, `m0 e^-kdt + s (1 - e^-kdt) / k`.
    fn leak(&mut self, start: &[Vec<f64>], dt: f64) {
        for ((decay, amounts), start) in self.decay.iter().zip(&mut self.amounts).zip(start) {
            for ((&k, amount), &m0) in decay.iter().zip(amounts).zip(start) {
                let x = k * dt;
                if x <= 0.0 {
                    continue;
                }
                // (1 - e^-x) / x, which is 1 as x goes to nothing.
                let spread = -(-x).exp_m1() / x;
                *amount = m0 * (-x).exp() + (*amount - m0) * spread;
            }
        }
    }

    /// The bins systems keep on `part`.
    fn own(&self, part: PartId) -> impl Iterator<Item = usize> + '_ {
        (self.systems.iter().enumerate())
            .filter(move |(_, (on, _))| *on == part)
            .map(|(k, _)| self.tanks + k)
    }

    /// Each place `resource` is kept in `bins`: the bin, which of its contents, and how much it
    /// holds when full.
    fn slots<'b>(
        &'b self,
        bins: &'b [usize],
        resource: &'b str,
    ) -> impl Iterator<Item = (usize, usize, f64)> + 'b {
        bins.iter().flat_map(move |&k| {
            (self.contents[k].iter().enumerate())
                .filter(move |(_, (r, _))| r == resource)
                .map(move |(c, (_, full))| (k, c, *full))
        })
    }

    fn held(&self, tanks: &[usize], resource: &str) -> f64 {
        (self.slots(tanks, resource))
            .map(|(k, c, _)| self.amounts[k][c])
            .sum()
    }

    fn room(&self, tanks: &[usize], resource: &str) -> f64 {
        (self.slots(tanks, resource))
            .map(|(k, c, full)| (full - self.amounts[k][c]).max(0.0))
            .sum()
    }

    /// Take `kg` of `resource`, the tiers furthest up the fuel lines first, each tier's tanks
    /// giving in proportion to what they hold, as [`Propulsion::draw`] does.
    fn take(&mut self, tiers: &[Vec<usize>], resource: &str, kg: f64) {
        let mut left = kg;
        for tier in tiers {
            let total = self.held(tier, resource);
            if total <= 0.0 {
                continue;
            }
            let take = left.min(total);
            let slots: Vec<_> = self.slots(tier, resource).collect();
            for (k, c, _) in slots {
                let amount = &mut self.amounts[k][c];
                *amount = (*amount - take * *amount / total).max(0.0);
            }
            left -= take;
            if left <= 0.0 {
                break;
            }
        }
    }

    /// Put `kg` of `resource` in, the part's own bins and tanks first, each tier's taking it in
    /// proportion to their room. What does not fit is let go of, unless it may `overflow`: then the
    /// nearest bins that keep it take it over their capacity, for a lot after it in the same step to
    /// take out again ([`Self::settle`] lets go of what is left over).
    fn put(&mut self, tiers: &[Vec<usize>], resource: &str, kg: f64, overflow: bool) {
        let mut left = kg;
        for tier in tiers.iter().rev() {
            let room = self.room(tier, resource);
            if room <= 0.0 {
                continue;
            }
            let put = left.min(room);
            let slots: Vec<_> = self.slots(tier, resource).collect();
            for (k, c, full) in slots {
                let amount = &mut self.amounts[k][c];
                *amount = (*amount + put * (full - *amount).max(0.0) / room).min(full);
            }
            left -= put;
            if left <= 0.0 {
                return;
            }
        }
        if !overflow {
            return;
        }
        for tier in tiers.iter().rev() {
            let capacity: f64 = self.slots(tier, resource).map(|(_, _, full)| full).sum();
            if capacity <= 0.0 {
                continue;
            }
            let slots: Vec<_> = self.slots(tier, resource).collect();
            for (k, c, full) in slots {
                self.amounts[k][c] += left * full / capacity;
            }
            return;
        }
    }

    /// Let go of what is over any bin's capacity, and call a rounding below nothing nothing.
    fn settle(&mut self) {
        for (contents, amounts) in self.contents.iter().zip(&mut self.amounts) {
            for ((_, full), amount) in contents.iter().zip(amounts) {
                *amount = amount.clamp(0.0, *full);
            }
        }
    }
}

fn trace_free(kg: f64) -> f64 {
    if kg < TRACE { 0.0 } else { kg }
}

/// One resource where some lots reach the same bins: who takes it, who makes it and must keep it,
/// and who makes it and may dump it, kg at full speed.
struct Pool<'a> {
    resource: &'a str,
    bins: Vec<usize>,
    takers: Vec<(usize, f64)>,
    makers: Vec<(usize, f64)>,
    dumpers: Vec<(usize, f64)>,
}

/// The pools of `lots`: for each resource, lots whose bins for it overlap go together.
fn pools<'a>(store: &Store, lots: &'a [Lot]) -> Vec<Pool<'a>> {
    let mut resources: BTreeSet<&str> = BTreeSet::new();
    for lot in lots {
        resources.extend(lot.exchange.inputs.iter().map(|(r, _)| r.as_str()));
        resources.extend(lot.exchange.outputs.iter().map(|(r, _)| r.as_str()));
    }
    let mut pools = Vec::new();
    for resource in resources {
        let keeps = |k: &usize| {
            store
                .slots(std::slice::from_ref(k), resource)
                .next()
                .is_some()
        };
        // Each lot that takes or holds back on it, with the bins it reaches that keep it.
        let mut groups: Vec<Pool> = Vec::new();
        for (i, lot) in lots.iter().enumerate() {
            let takes: f64 = (lot.exchange.inputs.iter())
                .filter(|(r, _)| r == resource)
                .map(|(_, kg)| kg)
                .sum();
            let made = |dumped: bool| -> f64 {
                (lot.exchange.outputs.iter())
                    .filter(|(r, _)| r == resource && lot.exchange.dump.contains(r) == dumped)
                    .map(|(_, kg)| kg)
                    .sum()
            };
            let (makes, dumps) = (made(false), made(true));
            if takes <= 0.0 && makes <= 0.0 && dumps <= 0.0 {
                continue;
            }
            let bins: Vec<usize> = lot.feeds.iter().copied().filter(keeps).collect();
            // Every group it shares a bin with joins it.
            let mut pool = Pool {
                resource,
                bins,
                takers: Vec::new(),
                makers: Vec::new(),
                dumpers: Vec::new(),
            };
            if takes > 0.0 {
                pool.takers.push((i, takes));
            }
            if makes > 0.0 {
                pool.makers.push((i, makes));
            }
            if dumps > 0.0 {
                pool.dumpers.push((i, dumps));
            }
            groups.retain_mut(|other| {
                let shared = other.bins.iter().any(|b| pool.bins.contains(b));
                if shared {
                    pool.bins.append(&mut other.bins);
                    pool.takers.append(&mut other.takers);
                    pool.makers.append(&mut other.makers);
                    pool.dumpers.append(&mut other.dumpers);
                }
                !shared
            });
            pool.bins.sort_unstable();
            pool.bins.dedup();
            groups.push(pool);
        }
        pools.extend(groups);
    }
    pools
}

/// Give `amount` to `wanting` (lots and what each wants), the highest priority first and each
/// priority in proportion to what it wants; what each gets, by lot.
fn give(lots: &[Lot], wanting: &[(usize, f64)], amount: f64) -> Vec<(usize, f64)> {
    let mut left = amount.max(0.0);
    let levels: BTreeSet<u8> = wanting
        .iter()
        .map(|&(i, _)| lots[i].exchange.priority)
        .collect();
    let mut given = Vec::new();
    for &level in levels.iter().rev() {
        let at: Vec<(usize, f64)> = (wanting.iter().copied())
            .filter(|&(i, _)| lots[i].exchange.priority == level)
            .collect();
        let wanted: f64 = at.iter().map(|(_, kg)| kg).sum();
        let scale = if wanted <= left { 1.0 } else { left / wanted };
        left -= wanted * scale;
        given.extend(at.into_iter().map(|(i, kg)| (i, kg * scale)));
    }
    given
}

/// A first guess at each lot's share, which [`allocate`] then cuts back until nothing is out of
/// bounds: each pool shares what it has between the lots that want it, each asking for what the
/// other pools let it have, over again until that settles. Asking for what it can use, a lot that
/// wants far more than it can put anywhere (a regulator topping up a full cabin) takes nothing from
/// those that share its inputs with it.
fn settle_on(lots: &[Lot], pools: &[Pool], start: &[(f64, f64, f64)]) -> Vec<f64> {
    let n = lots.len();
    // What each pool lets each lot have, as a share: 1 where it does not hold it back.
    let mut caps = vec![vec![1.0_f64; n]; pools.len()];
    let least = |caps: &[Vec<f64>], i: usize, but: Option<usize>| {
        (caps.iter().enumerate())
            .filter(|(p, _)| Some(*p) != but)
            .map(|(_, c)| c[i])
            .fold(1.0_f64, f64::min)
    };
    for _ in 0..100 {
        let shares: Vec<f64> = (0..n).map(|i| least(&caps, i, None)).collect();
        let mut next = vec![vec![1.0_f64; n]; pools.len()];
        for (p, (pool, &(held, room, lost))) in pools.iter().zip(start).enumerate() {
            // Those on one side ask for what the other pools let them have; those on the other
            // side give and take at their shares.
            let asking = |list: &[(usize, f64)]| -> Vec<(usize, f64)> {
                (list.iter())
                    .map(|&(i, kg)| (i, kg * least(&caps, i, Some(p))))
                    .collect()
            };
            let at =
                |list: &[(usize, f64)]| -> f64 { list.iter().map(|&(i, kg)| kg * shares[i]).sum() };
            let kept = !pool.bins.is_empty();
            let mut hold_back = |wanting: Vec<(usize, f64)>, full: &[(usize, f64)], there: f64| {
                let asked: f64 = wanting.iter().map(|(_, kg)| kg).sum();
                if asked <= there * (1.0 + 1e-12) + TRACE {
                    return;
                }
                for (i, got) in give(lots, &wanting, there) {
                    let ask = wanting.iter().find(|(w, _)| *w == i).expect("asked").1;
                    let whole = full.iter().find(|(w, _)| *w == i).expect("wanted").1;
                    // Given all it asked for, this pool is not what holds it back.
                    if got < ask * (1.0 - 1e-12) {
                        next[p][i] = next[p][i].min(got / whole);
                    }
                }
            };
            let there = held
                + if kept {
                    at(&pool.makers) + at(&pool.dumpers)
                } else {
                    0.0
                };
            hold_back(asking(&pool.takers), &pool.takers, there);
            let outlet = if kept {
                room + at(&pool.takers) + lost
            } else {
                0.0
            };
            hold_back(asking(&pool.makers), &pool.makers, outlet);
        }
        if next == caps {
            break;
        }
        caps = next;
    }
    (0..n).map(|i| least(&caps, i, None)).collect()
}

/// Run `lots` on `store` together for `dt` seconds, as if all of them run through the step at once:
/// the share each gets. Each starts at full speed. Where what is there and what the others make
/// falls short of what is taken, the takers are cut back, the highest priority keeping the most;
/// where the room, what the others take and what leaks out falls short of what is made, the makers
/// are. Over again until nothing changes, which it soon does, as shares only ever fall. Then
/// everything made goes in, and everything taken comes out. With what each takes and makes steady
/// through the step, what the bins hold moves in a straight line from start to end, so it is never
/// out of bounds on the way; then what leaks goes, exactly.
fn allocate(store: &mut Store, lots: &[Lot], dt: f64) -> Vec<f64> {
    let pools = pools(store, lots);
    let begun = store.amounts.clone();
    let start: Vec<(f64, f64, f64)> = (pools.iter())
        .map(|p| {
            let held = trace_free(store.held(&p.bins, p.resource));
            let room = trace_free(store.room(&p.bins, p.resource));
            let lost = store.leaking(&p.bins, p.resource) * dt;
            (held, room, lost)
        })
        .collect();
    let mut shares = settle_on(lots, &pools, &start);
    for _ in 0..100 {
        let mut most = vec![1.0_f64; lots.len()];
        for (pool, &(held, room, lost)) in pools.iter().zip(&start) {
            let now = |list: &[(usize, f64)]| -> Vec<(usize, f64)> {
                list.iter().map(|&(i, kg)| (i, shares[i] * kg)).collect()
            };
            let (takers, makers) = (now(&pool.takers), now(&pool.makers));
            let taken: f64 = takers.iter().map(|(_, kg)| kg).sum();
            let made: f64 = makers.iter().map(|(_, kg)| kg).sum();
            let dumped: f64 = now(&pool.dumpers).iter().map(|(_, kg)| kg).sum();
            // With no bin to keep it in, nothing made can be taken. What may be dumped is there to
            // take as much as what must be kept.
            let kept = !pool.bins.is_empty();
            let there = held + if kept { made + dumped } else { 0.0 };
            if taken > there * (1.0 + 1e-12) + TRACE {
                for (i, got) in give(lots, &takers, there) {
                    let full = pool
                        .takers
                        .iter()
                        .find(|(t, _)| *t == i)
                        .expect("a taker")
                        .1;
                    most[i] = most[i].min(got / full);
                }
            }
            let outlet = if kept { room + taken + lost } else { 0.0 };
            if made > outlet * (1.0 + 1e-12) + TRACE {
                for (i, got) in give(lots, &makers, outlet) {
                    let full = pool
                        .makers
                        .iter()
                        .find(|(m, _)| *m == i)
                        .expect("a maker")
                        .1;
                    most[i] = most[i].min(got / full);
                }
            }
        }
        let mut changed = false;
        for (share, most) in shares.iter_mut().zip(most) {
            if most < *share - 1e-15 {
                *share = most.max(0.0);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // All of it goes in, over the brim if need be, for those after to take; what is still over when
    // all is taken is what was dumped.
    for (lot, &share) in lots.iter().zip(&shares) {
        for (resource, kg) in &lot.exchange.outputs {
            store.put(&lot.tiers, resource, kg * share, true);
        }
    }
    for (lot, &share) in lots.iter().zip(&shares) {
        for (resource, kg) in &lot.exchange.inputs {
            store.take(&lot.tiers, resource, kg * share);
        }
    }
    store.linear = store.amounts.clone();
    store.leak(&begun, dt);
    store.settle();
    shares
}

impl Propulsion {
    /// Move the systems on `dt` seconds under `environment`, with what they take from the tanks
    /// their parts reach and give back to them: see the module's notes. `supplied`, the vessel is
    /// on its ground support at a launch site, which powers it and feeds what its systems burn:
    /// nothing comes out of its stores or goes into its tanks.
    pub fn step_systems(&mut self, environment: &Environment, dt: f64, supplied: bool) {
        self.systems.share_air();
        let wanted = self.lots(|s| s.wants(dt));
        let shares = self.exchange(&wanted, supplied, false, dt);
        self.tell(&wanted, &shares, |s, shares| s.plan(shares));
        self.systems.step(environment, dt, supplied);
        let used = self.lots(|s| s.uses(dt));
        let shares = self.exchange(&used, supplied, true, dt);
        self.tell(&used, &shares, |s, shares| s.exchanged(shares, dt));
        self.systems.share_air();
    }

    /// Seconds until a system needs attention: it says so, or a tank one draws on runs dry or one
    /// it fills is full, at what they take and give now. Nothing runs dry while `supplied`.
    pub fn next_system_event(&self, environment: &Environment, supplied: bool) -> Option<f64> {
        let own = self.systems.next_event(environment, supplied);
        if supplied {
            return own;
        }
        // What each would reach next at the shares the tanks give now.
        let wanted = self.lots(|s| s.wants(1.0));
        let shares = self.shares(&wanted, 1.0).0;
        let staged: Vec<f64> = (wanted.iter().zip(&shares))
            .filter_map(|(s, shares)| self.systems.on(s.part)[s.index].next_event_given(shares))
            .collect();
        // What goes out of (or into, less than nothing) each lot of tanks a second.
        let store = Store::new(self);
        // Each lot of bins and resource: what goes out net, kg/s (less than nothing coming in), and
        // whether anything coming in must be kept, so that its filling up matters.
        let mut flows: BTreeMap<(Vec<usize>, String), (f64, bool)> = BTreeMap::new();
        for part in self.systems.consumers() {
            let feeds = self.feeds_of(&store, part);
            for system in self.systems.on(part).iter().filter(|s| s.consumes()) {
                for lot in system.exchanging() {
                    for (resource, kg) in lot.inputs {
                        flows.entry((feeds.clone(), resource)).or_default().0 += kg;
                    }
                    for (resource, kg) in lot.outputs {
                        let kept = !lot.dump.contains(&resource);
                        let flow = flows.entry((feeds.clone(), resource)).or_default();
                        flow.0 -= kg;
                        flow.1 |= kept;
                    }
                }
            }
        }
        // What leaks out besides runs a store dry sooner and fills it later: as it does, neither
        // comes before this.
        let ends = flows
            .into_iter()
            .filter_map(|((feeds, resource), (net, kept))| {
                let lost = store.leaking(&feeds, &resource);
                let (left, rate) = if net > 0.0 {
                    (store.held(&feeds, &resource), net + lost)
                } else if kept {
                    (store.room(&feeds, &resource), -net - lost)
                } else {
                    (0.0, 0.0)
                };
                (rate > 0.0 && left > TRACE).then(|| left / rate)
            });
        // Where what systems keep crosses a line of theirs, at what goes in and out of it now.
        let running = self.lots(|s| s.exchanging());
        let (_, after) = self.shares(&running, 1.0);
        let crossings = (store.systems.iter().enumerate()).filter_map(|(k, &(part, index))| {
            let bin = store.tanks + k;
            let inflows: Vec<f64> = (after.linear[bin].iter().zip(&store.amounts[bin]))
                .map(|(end, start)| end - start)
                .collect();
            self.systems.on(part)[index].next_event_at(&inflows)
        });
        (own.into_iter().chain(staged).chain(ends).chain(crossings)).reduce(f64::min)
    }

    /// How much of `resource` there is for what is on `part`: in the tanks it reaches and what its
    /// systems keep (a cabin's lockers and air), kg.
    pub fn held_for(&self, part: PartId, resource: &str) -> f64 {
        let store = Store::new(self);
        store.held(&self.feeds_of(&store, part), resource)
    }

    /// The lots `of` gives for each system that uses the tanks.
    fn lots(&self, of: impl Fn(&dyn System) -> Vec<Exchange>) -> Vec<Lots> {
        let mut all = Vec::new();
        for part in self.systems.consumers() {
            for (index, system) in self.systems.on(part).iter().enumerate() {
                if system.consumes() {
                    all.push(Lots {
                        part,
                        index,
                        lots: of(system.as_ref()),
                    });
                }
            }
        }
        all
    }

    /// The share of each lot over `dt` seconds, by system; taken out of the tanks and put in them
    /// if `commit`.
    fn exchange(&mut self, all: &[Lots], supplied: bool, commit: bool, dt: f64) -> Vec<Vec<f64>> {
        if supplied {
            return all.iter().map(|s| vec![1.0; s.lots.len()]).collect();
        }
        let (shares, store) = self.shares(all, dt);
        if commit {
            let mut amounts = store.amounts.into_iter();
            for (tank, amounts) in self.tanks.iter_mut().zip(amounts.by_ref()) {
                tank.amounts = amounts;
            }
            for ((part, index), amounts) in store.systems.into_iter().zip(amounts) {
                self.systems.on_mut(part)[index].set_stored(&amounts);
            }
        }
        shares
    }

    /// The share of each lot over `dt` seconds, by system, and what the tanks and bins would hold
    /// after.
    fn shares(&self, all: &[Lots], dt: f64) -> (Vec<Vec<f64>>, Store) {
        let mut store = Store::new(self);
        let lots: Vec<Lot> = (all.iter())
            .flat_map(|s| {
                let tiers = self.tiers_of(&store, s.part);
                let feeds = self.feeds_of(&store, s.part);
                (s.lots.iter()).map(move |exchange| Lot {
                    tiers: tiers.clone(),
                    feeds: feeds.clone(),
                    exchange,
                })
            })
            .collect();
        let flat = allocate(&mut store, &lots, dt);
        let mut flat = flat.into_iter();
        let shares = (all.iter())
            .map(|s| flat.by_ref().take(s.lots.len()).collect())
            .collect();
        (shares, store)
    }

    /// Tell each system its shares.
    fn tell(&mut self, all: &[Lots], shares: &[Vec<f64>], f: impl Fn(&mut dyn System, &[f64])) {
        for (s, shares) in all.iter().zip(shares) {
            f(self.systems.on_mut(s.part)[s.index].as_mut(), shares);
        }
    }

    /// The bins `part` draws on, in tiers, those furthest up its fuel lines first, its own last:
    /// its own tanks and what its systems keep. A save from before fuel lines has its feeds and no
    /// tiers.
    fn tiers_of(&self, store: &Store, part: PartId) -> Vec<Vec<usize>> {
        let mut tiers = (self.tiers.get(&part).cloned())
            .unwrap_or_else(|| vec![self.feeds.get(&part).cloned().unwrap_or_default()]);
        let own: Vec<usize> = store.own(part).collect();
        if !own.is_empty() {
            match tiers.last_mut() {
                Some(last) => last.extend(own),
                None => tiers.push(own),
            }
        }
        tiers
    }

    fn feeds_of(&self, store: &Store, part: PartId) -> Vec<usize> {
        let mut feeds = self.feeds.get(&part).cloned().unwrap_or_default();
        feeds.extend(store.own(part));
        feeds.sort_unstable();
        feeds
    }
}

#[cfg(test)]
mod tests;
