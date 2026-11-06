# T209: Tank contents follow the engines on the craft, at their ratio

Asked for on 2026-11-06: choosing what a tank holds "should only give you options for the fuels
that an engine in the rocket can use, with the correct fuel ratios". The part window offers a fixed
list of mixes (`MIXES` in `osp_sim::properties`) with ratios of their own, whatever engines the
craft has.

## Scope

- The choices are the propellant sets of the engines on the craft (each engine's `propellants`,
  at its own mass ratio), named once each, plus Nothing (structure). Mixes no engine burns are not
  offered; with no engine on the craft yet, the usual list is.
- A tank's current contents stay shown if they match none (as now, "custom").
- Single resources an engine burns alone (monopropellant) are offered the same way.

## Done when

- On a craft with one kerolox engine the tank offers kerolox at that engine's ratio and nothing
  else but structure; adding a hydrolox engine adds hydrolox; tests cover it.
