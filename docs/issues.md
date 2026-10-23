# Issues

Open problems found while playing, oldest first. Delete an entry when it is fixed.

## 3. Staging: what is still different from KSA

Reported 2026-10-20; the main request (KSA-style sequences, icons and numbers, fairings and
RCS stageable, reorder and regroup, decoupling apart from ignition) landed 2026-10-22, see
"Staging" in `docs/modding.md`. Left over:

- A stage's engines are one activation, so a cluster cannot be split (light the centre
  engine first, say). KSA moves single parts and groups symmetric ones under one icon.
- No right-click "add a sequence above / below"; + adds one at the end and numbers are
  dragged to reorder.
- One fuel bar per engine group, not per engine.
- Drag and drop in the panel is covered by tests of each piece and the scripted capture
  (`OSP_SCENARIO=leo OSP_CAPTURE_STAGING=1`), but not tried by hand yet.

KSA's model, from the community wiki's build notes: sequences are the activation groups
Space fires; resource groups (stages until 2026.6.9.4750) share propellant; the Sequence
window (F8) shows part icons with live delta-v, TWR and fuel; parts and sequences are
dragged around in flight (2026.5.12.4510). Sources:
https://kittenspaceagency.wiki.gg/wiki/Version_2026.5.12.4510,
https://kittenspaceagency.wiki.gg/wiki/Version_2026.6.9.4750,
https://kittenspaceagency.wiki.gg/wiki/Version_2026.7.4.4860,
https://kittenspaceagency.wiki.gg/wiki/Controls, https://areweksayet.com/ (unofficial).

