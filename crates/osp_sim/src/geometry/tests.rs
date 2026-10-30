//! Shapes measured against what is known about them: analytic volumes, areas and inertias,
//! the mass shapes in [`crate::mass`] and the tank sizing in [`crate::structure`].

use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_4, PI};

use glam::{DMat3, DVec3};

use super::*;
use crate::{
    data::{ShellDef as StageShellDef, ShellProfile},
    mass::Mass,
    shell::Shell,
    test_data::records,
};

fn shape(text: &str) -> Geometry {
    let def: ShapeDef = records(text).remove(0);
    def.build().unwrap_or_else(|e| panic!("{e}"))
}

fn refused(text: &str) -> String {
    let def: ShapeDef = records(text).remove(0);
    def.build().expect_err("should be refused")
}

fn moments(g: &Geometry, name: &str) -> Moments {
    g.element(name)
        .unwrap_or_else(|| panic!("no `{name}` in {:?}", names(g)))
        .moments
}

fn names(g: &Geometry) -> Vec<String> {
    g.elements().into_iter().map(|e| e.name).collect()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs().max(1.0)
}

#[track_caller]
fn assert_close(a: f64, b: f64, tolerance: f64) {
    assert!(close(a, b, tolerance), "{a} vs {b}");
}

#[track_caller]
fn assert_same_mass(a: Mass, b: Mass, tolerance: f64) {
    assert_close(a.mass, b.mass, tolerance);
    let scale = b.center.length().max(1.0);
    assert!(
        (a.center - b.center).length() <= tolerance * scale,
        "centre {} vs {}",
        a.center,
        b.center
    );
    let size = b
        .inertia
        .to_cols_array()
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()));
    let off = (a.inertia - b.inertia)
        .to_cols_array()
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()));
    assert!(
        off <= tolerance * size,
        "inertia {:?} vs {:?}",
        a.inertia,
        b.inertia
    );
}

/// A solid hemisphere of `mass` on a flat face at height `face`, bulging up or down.
fn hemisphere(mass: f64, radius: f64, face: f64, up: bool) -> Mass {
    let offset = 3.0 * radius / 8.0;
    let axial = 0.4 * mass * radius * radius;
    let across = axial - mass * offset * offset;
    Mass {
        mass,
        center: DVec3::Z * (face + if up { offset } else { -offset }),
        inertia: DMat3::from_diagonal(DVec3::new(across, across, axial)),
    }
}

/// Half the area of an oblate spheroid with equatorial radius `a` and polar `c`.
fn half_spheroid_area(a: f64, c: f64) -> f64 {
    let e = (1.0 - c * c / (a * a)).sqrt();
    PI * a * a + PI * c * c / (2.0 * e) * ((1.0 + e) / (1.0 - e)).ln()
}

#[test]
fn a_sphere_is_measured_exactly() {
    let g = shape("{ sphere: { diameter: 3 } }");
    let r: f64 = 1.5;
    let tank = moments(&g, "tank");
    assert_close(tank.measure, 4.0 / 3.0 * PI * r.powi(3), 1e-12);
    let expected = Mass::sphere(tank.measure * 2.0, r).placed(DVec3::Z * r, DQuat::IDENTITY);
    assert_same_mass(tank.mass(2.0), expected, 1e-12);
    let wall = moments(&g, "wall");
    assert_close(wall.measure, 4.0 * PI * r * r, 1e-12);
    let shell = Mass::spherical_shell(wall.measure, r).placed(DVec3::Z * r, DQuat::IDENTITY);
    assert_same_mass(wall.mass(1.0), shell, 1e-12);
    // Turned all the way round: nothing off the axis.
    assert_eq!((tank.first.x, tank.first.y), (0.0, 0.0));
}

#[test]
fn a_cylinder_tank_measures_as_the_stage_tanks_do() {
    // Today's tanks (`structure::Tank`): √2:1 domes, volume and wall area from their outer
    // diameter.
    let g = shape("{ cylinder_tank: { diameter: 3.7, length: 10 } }");
    let (r, dome) = (1.85, 1.85 * FRAC_1_SQRT_2);
    let barrel = 10.0 - 2.0 * dome;
    assert_eq!(
        names(&g),
        ["aft_dome", "barrel", "forward_dome", "tank"].map(String::from)
    );
    let tank = moments(&g, "tank");
    assert_close(
        tank.measure,
        PI * r * r * (barrel + 4.0 / 3.0 * dome),
        1e-12,
    );
    assert_close(tank.centroid().unwrap().z, 5.0, 1e-12);
    let walls = moments(&g, "aft_dome") + moments(&g, "barrel") + moments(&g, "forward_dome");
    let area = 2.0 * PI * r * barrel + 2.0 * half_spheroid_area(r, dome);
    assert_close(walls.measure, area, 1e-11);
    let (lo, hi) = match &g.subparts[0].kind {
        Kind::Revolved(rev) => rev.heights(0),
        _ => unreachable!(),
    };
    assert_close(lo, 0.0, 1e-12);
    assert_close(hi, 10.0, 1e-12);
}

#[test]
fn a_tank_with_hemispherical_domes_is_a_capsule() {
    let g = shape("{ cylinder_tank: { diameter: 2, length: 6, dome: 1 } }");
    let (r, barrel) = (1.0, 4.0);
    let density = 3.0;
    let per = |volume: f64| volume * density;
    let half = 2.0 / 3.0 * PI;
    let capsule = Mass::cylinder(per(PI * barrel), r, barrel).placed(DVec3::Z * r, DQuat::IDENTITY)
        + hemisphere(per(half), r, r, false)
        + hemisphere(per(half), r, r + barrel, true);
    assert_same_mass(moments(&g, "tank").mass(density), capsule, 1e-12);
}

#[test]
fn a_common_bulkhead_parts_the_tank_in_two() {
    let g = shape("{ cylinder_tank: { diameter: 4, length: 12, common_bulkhead: 3 } }");
    let whole = shape("{ cylinder_tank: { diameter: 4, length: 12 } }");
    let (lower, upper) = (moments(&g, "lower"), moments(&g, "upper"));
    // Between two domes of the same shape, 3 m apart: exactly a 3 m cylinder's worth.
    assert_close(lower.measure, PI * 4.0 * 3.0, 1e-12);
    let tank = moments(&whole, "tank");
    assert_close(lower.measure + upper.measure, tank.measure, 1e-12);
    assert_close((lower.first + upper.first).z, tank.first.z, 1e-12);
    // The bulkhead is a dome's worth of wall.
    assert_close(
        moments(&g, "bulkhead").measure,
        moments(&g, "aft_dome").measure,
        1e-12,
    );
    for (bulkhead, problem) in [(0.0, "zero"), (9.5, "past the barrel")] {
        let text = format!(
            "{{ cylinder_tank: {{ diameter: 4, length: 12, common_bulkhead: {bulkhead} }} }}"
        );
        assert!(refused(&text).contains("common bulkhead"), "{problem}");
    }
}

#[test]
fn skirts_carry_the_wall_past_the_domes() {
    let g =
        shape("{ cylinder_tank: { diameter: 2, length: 5, aft_skirt: 1.5, forward_skirt: 0.2 } }");
    let dome = FRAC_1_SQRT_2;
    assert_close(moments(&g, "aft_skirt").measure, 2.0 * PI * 1.5, 1e-12);
    assert_close(moments(&g, "forward_skirt").measure, 2.0 * PI * 0.2, 1e-12);
    // The aft skirt is longer than the dome, so the tank stands on it: the dome's tip is
    // above the base.
    let Kind::Revolved(rev) = &g.subparts[0].kind else {
        unreachable!()
    };
    let (lo, hi) = rev.heights(0);
    assert_close(lo, 1.5 - dome, 1e-12);
    assert_close(hi, 1.5 - dome + 5.0, 1e-12);
    // The forward skirt ends above the forward rim, not the dome's tip.
    let top = g.surface_point("forward_skirt", 0.0, 1.0).unwrap();
    assert_close(top.position.z, 1.5 + (5.0 - 2.0 * dome) + 0.2, 1e-9);
}

#[test]
fn a_torus_follows_pappus() {
    let g = shape("{ torus: { diameter: 3, tube_diameter: 1 } }");
    let (center, r) = (1.0, 0.5);
    let tank = moments(&g, "tank");
    let volume = 2.0 * PI * PI * center * r * r;
    assert_close(tank.measure, volume, 1e-12);
    assert_close(
        moments(&g, "wall").measure,
        4.0 * PI * PI * center * r,
        1e-12,
    );
    let m = tank.mass(1.0);
    let axial = volume * (center * center + 0.75 * r * r);
    let across = volume * (center * center / 2.0 + 0.625 * r * r);
    let expected = Mass {
        mass: volume,
        center: DVec3::Z * r,
        inertia: DMat3::from_diagonal(DVec3::new(across, across, axial)),
    };
    assert_same_mass(m, expected, 1e-12);
    assert!(refused("{ torus: { diameter: 1.5, tube_diameter: 1 } }").contains("clear the axis"));
}

#[test]
fn a_sector_tank_is_a_share_of_the_whole() {
    // A quarter of a flat-ended cylinder, 2 m in radius and 5 m long.
    let g = shape(
        "{ revolved: { span: [0, 90], regions: [{ name: 'tank', fill: 'cell', boundary: {
            start: [0, 0], segments: [{ line: [2, 0] }, { line: [2, 5] }, { line: [0, 5] }] } }] } }",
    );
    let tank = moments(&g, "tank");
    assert_close(tank.measure, PI * 4.0 * 5.0 / 4.0, 1e-12);
    // The centroid of a sector of angle α lies 4 R sin(α/2) / 3α from the axis.
    let out = 4.0 * 2.0 * FRAC_PI_4.sin() / (3.0 * PI / 2.0);
    let c = tank.centroid().unwrap();
    assert_close(c.x, out * FRAC_PI_4.cos(), 1e-12);
    assert_close(c.y, out * FRAC_PI_4.sin(), 1e-12);
    assert_close(c.z, 2.5, 1e-12);
    // Its two flat sides, 2 m by 5 m each.
    let sides = moments(&g, "tank/sides");
    assert_close(sides.measure, 20.0, 1e-12);
    // The mesh agrees.
    let Kind::Revolved(rev) = &g.subparts[0].kind else {
        unreachable!()
    };
    let mesh = rev.region_mesh(0);
    assert!(mesh.is_closed());
    assert_same_mass(mesh.volume_moments().mass(1.0), tank.mass(1.0), 1e-3);
}

#[test]
fn shells_measure_as_the_outlines_they_replace() {
    // A frustum and an ogive nose, against `Mass::revolved_shell` over their outlines.
    let frustum =
        shape("{ shell: { bottom_diameter: 3, sections: [{ length: 2, top_diameter: 2 }] } }");
    let wall = moments(&frustum, "wall").mass(1.0);
    let expected = Mass::revolved_shell(wall.mass, &[(0.0, 1.5), (2.0, 1.0)]);
    assert_same_mass(wall, expected, 1e-12);
    assert_close(wall.mass, PI * 2.5 * (0.25f64 + 4.0).sqrt(), 1e-12);

    let nose = shape(
        "{ shell: { bottom_diameter: 2, sections: [{ length: 3, top_diameter: 0, profile: 'ogive' }],
                    cap_bottom: true } }",
    );
    let def = StageShellDef {
        name: None,
        length: 3.0,
        bottom_diameter: Some(2.0),
        top_diameter: Some(0.0),
        profile: ShellProfile::Ogive,
        mass: 0.0,
    };
    let outline = Shell::new(&def, 0.0, 1.0).unwrap().profile(4000);
    let wall = moments(&nose, "wall").mass(1.0);
    assert_same_mass(wall, Mass::revolved_shell(wall.mass, &outline), 1e-5);
    assert_close(moments(&nose, "bottom").measure, PI, 1e-12);
    // Over part of a turn, a fairing panel.
    let panel = shape(
        "{ shell: { bottom_diameter: 3, sections: [{ length: 2, top_diameter: 2 }], span: [0, 180] } }",
    );
    assert_close(
        moments(&panel, "wall").measure,
        wall_area(&frustum) / 2.0,
        1e-12,
    );
    assert!(
        refused("{ shell: { bottom_diameter: 3, sections: [{ length: 1, top_diameter: 0, profile: 'ogive' }] } }")
            .contains("ogive")
    );
}

fn wall_area(g: &Geometry) -> f64 {
    moments(g, "wall").measure
}

#[test]
fn a_cone_tank_without_domes_is_a_frustum() {
    let g = shape("{ cone_tank: { length: 4, bottom_diameter: 3, top_diameter: 1, dome: 0 } }");
    let tank = moments(&g, "tank");
    let (a, b, h) = (1.5f64, 0.5f64, 4.0);
    assert_close(tank.measure, PI * h / 3.0 * (a * a + a * b + b * b), 1e-12);
    assert_same_mass(tank.mass(1.0), Mass::frustum(tank.measure, a, b, h), 1e-12);
    // Closing to a point at the top, like an R-7 strap-on.
    let pointed = shape("{ cone_tank: { length: 19, bottom_diameter: 2.7, top_diameter: 0 } }");
    assert!(pointed.element("forward_dome").is_none());
    assert!(moments(&pointed, "tank").measure > 0.0);
}

#[test]
fn a_ring_is_an_annulus() {
    let g = shape("{ ring: { diameter: 3, height: 0.1, thickness: 0.2 } }");
    let ring = g.element("ring").unwrap();
    assert_eq!(ring.fill, Some(Fill::Solid));
    assert_close(
        ring.moments.measure,
        PI * (1.5f64.powi(2) - 1.3f64.powi(2)) * 0.1,
        1e-12,
    );
    let disc = shape("{ ring: { diameter: 1, height: 0.1, thickness: 2 } }");
    assert_close(moments(&disc, "ring").measure, PI * 0.25 * 0.1, 1e-12);
}

#[test]
fn liquid_settles_level_along_the_axis() {
    let sphere = shape("{ sphere: { diameter: 2 } }");
    let total = moments(&sphere, "tank").measure;
    let half = sphere.settle("tank", DVec3::Z, total / 2.0).unwrap();
    assert_close(half.level, 1.0, 1e-12);
    assert_close(half.moments.measure, total / 2.0, 1e-12);
    // A hemisphere's centroid is 3/8 of its radius from its face.
    assert_close(half.moments.centroid().unwrap().z, 1.0 - 3.0 / 8.0, 1e-12);
    // Upside down, it fills from the top.
    let top = sphere.settle("tank", -DVec3::Z, total / 4.0).unwrap();
    assert_close(top.moments.measure, total / 4.0, 1e-12);
    assert!(top.moments.centroid().unwrap().z > 1.5);
    assert!(top.level < -1.0);
    // Empty and full.
    assert_close(
        sphere
            .settle("tank", DVec3::Z, 0.0)
            .unwrap()
            .moments
            .measure,
        0.0,
        1e-15,
    );
    let full = sphere.settle("tank", DVec3::Z, 2.0 * total).unwrap();
    assert_close(full.moments.measure, total, 1e-12);
    assert_close(full.level, 2.0, 1e-12);

    // In a tank with a common bulkhead, each cell fills on its own.
    let tank = shape("{ cylinder_tank: { diameter: 4, length: 12, common_bulkhead: 3 } }");
    let upper = moments(&tank, "upper").measure;
    let settled = tank.settle("upper", DVec3::Z, upper / 2.0).unwrap();
    assert_close(settled.moments.measure, upper / 2.0, 1e-12);
    let rim = 2.0 * FRAC_1_SQRT_2 + 3.0;
    assert!(settled.level > rim, "{}", settled.level);
}

#[test]
fn liquid_settles_level_in_any_direction() {
    // A sphere looks the same every way: half full, the liquid's centroid is 3/8 of the
    // radius from the centre, against `up`.
    let sphere = shape("{ sphere: { diameter: 2 } }");
    let total = moments(&sphere, "tank").measure;
    let up = DVec3::new(1.0, 0.5, 1.0).normalize();
    let half = sphere.settle("tank", up, total / 2.0).unwrap();
    assert_close(half.moments.measure, total / 2.0, 1e-12);
    let expected = DVec3::Z - up * 3.0 / 8.0;
    assert!((half.moments.centroid().unwrap() - expected).length() < 1e-3);
    assert!((half.level - up.dot(DVec3::Z)).abs() < 1e-3);

    // Just off the axis, the mesh agrees with the exact level.
    let tank = shape("{ cylinder_tank: { diameter: 3, length: 9 } }");
    let total = moments(&tank, "tank").measure;
    let exact = tank.settle("tank", DVec3::Z, 0.3 * total).unwrap();
    let tilted = DVec3::new(1e-6, 0.0, 1.0).normalize();
    let meshed = tank.settle("tank", tilted, 0.3 * total).unwrap();
    assert_close(meshed.moments.measure, exact.moments.measure, 1e-12);
    assert!(
        (meshed.level - exact.level).abs() < 2e-3,
        "{} vs {}",
        meshed.level,
        exact.level
    );
    let (a, b) = (
        exact.moments.centroid().unwrap(),
        meshed.moments.centroid().unwrap(),
    );
    assert!((a - b).length() < 2e-3, "{a} vs {b}");

    // Lying on its side, a box fills half way up its width.
    let cuboid = shape("{ box: { size: [2, 1, 4], fill: 'cell' } }");
    let half = cuboid.settle("box", DVec3::X, 4.0).unwrap();
    assert_close(half.level, 0.0, 1e-9);
    assert_close(half.moments.centroid().unwrap().x, -0.5, 1e-9);
}

#[test]
fn meshes_are_watertight_and_close_to_exact() {
    for text in [
        "{ sphere: { diameter: 2 } }",
        "{ torus: { diameter: 4, tube_diameter: 1 } }",
        "{ torus: { diameter: 2, tube_diameter: 1 } }",
        "{ cylinder_tank: { diameter: 4, length: 12, common_bulkhead: 3, aft_skirt: 2 } }",
        "{ cone_tank: { length: 19, bottom_diameter: 2.7, top_diameter: 0 } }",
        "{ ring: { diameter: 3, height: 0.1 } }",
    ] {
        let g = shape(text);
        let Kind::Revolved(rev) = &g.subparts[0].kind else {
            unreachable!()
        };
        for i in 0..rev.regions().len() {
            let mesh = rev.region_mesh(i);
            assert!(mesh.is_closed(), "{text}");
            let (exact, meshed) = (rev.region_moments(i), mesh.volume_moments());
            assert!(close(meshed.measure, exact.measure, 1e-3), "{text}");
        }
    }
}

#[test]
fn wings_loft_their_airfoil() {
    let g = shape(
        "{ wing: { root_chord: 3, tip_chord: 1, span: 6, sweep: 1.5, dihedral: 5, twist: -2 } }",
    );
    let Kind::Lofted(loft) = &g.subparts[0].kind else {
        unreachable!()
    };
    assert!(loft.mesh().is_closed());
    // The volume of a straight taper whose sections grow with the chord squared.
    let area = mesh::signed_area(loft.section());
    let (cr, ct, span) = (3.0, 1.0, 6.0);
    let volume = area * span * (cr * cr + cr * ct + ct * ct) / 3.0;
    let wing = moments(&g, "wing");
    assert_close(wing.measure, volume, 1e-9);
    // A NACA 0012 encloses about 0.082 of its chord squared.
    assert!((area - 0.0822).abs() < 0.001, "{area}");
    // The span runs along +X, the chord back along −Z from the root's leading edge, and the
    // dihedral lifts the tip towards −Y.
    let placed: Vec<DVec3> = loft
        .mesh()
        .positions
        .iter()
        .map(|&p| g.subparts[0].rotation * p)
        .collect();
    let (lo, hi) = placed
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
    assert_close(lo, 0.0, 1e-12);
    assert_close(hi, 6.0, 1e-12);
    let root_le = placed
        .iter()
        .filter(|p| p.x.abs() < 1e-9)
        .fold(f64::NEG_INFINITY, |m, p| m.max(p.z));
    assert_close(root_le, 0.0, 1e-9);
    let tip_y = placed
        .iter()
        .filter(|p| (p.x - span).abs() < 1e-9)
        .map(|p| p.y)
        .sum::<f64>()
        / loft.section().len() as f64;
    assert!(tip_y < -0.4, "{tip_y}");
    assert!(placed.iter().all(|p| p.z <= 1e-9));

    // A fin plate keeps its thickness whatever the chord; its bevels take 15 % of it.
    let fin = shape(
        "{ wing: { root_chord: 2, tip_chord: 1, span: 1.5, airfoil: { plate: { thickness: 0.02 } } } }",
    );
    assert_close(
        moments(&fin, "wing").measure,
        0.85 * 0.02 * 1.5 * 1.5,
        1e-12,
    );
}

#[test]
fn surface_points_sit_on_their_walls() {
    let g = shape("{ cylinder_tank: { diameter: 2, length: 6, dome: 0.5 } }");
    // A quarter of the way round, half way up the barrel: on +Y, facing out.
    let p = g.surface_point("barrel", 0.25, 0.5).unwrap();
    assert!(
        (p.position - DVec3::new(0.0, 1.0, 3.0)).length() < 1e-9,
        "{}",
        p.position
    );
    assert!((p.normal - DVec3::Y).length() < 1e-9);
    assert!(p.along_v.z > 0.0 && p.along_u.x < 0.0);
    // The bottom of the aft dome is on the axis, facing down.
    let bottom = g.surface_point("aft_dome", 0.0, 0.0).unwrap();
    assert!(bottom.position.length() < 1e-9 && (bottom.normal + DVec3::Z).length() < 1e-6);
    // The equator of a sphere faces straight out.
    let sphere = shape("{ sphere: { diameter: 2 } }");
    let equator = sphere.surface_point("wall", 0.0, 0.5).unwrap();
    assert!((equator.position - DVec3::new(1.0, 0.0, 1.0)).length() < 1e-6);
    assert!((equator.normal - DVec3::X).length() < 1e-6);
    // A box's skin faces out of each side.
    let cuboid = shape("{ box: { size: [2, 1, 4] } }");
    let side = cuboid.surface_point("box/skin", 0.5, 0.1).unwrap();
    assert!(side.normal.dot(side.position.with_z(0.0)) > 0.0);
    assert!(g.surface_point("nothing", 0.0, 0.0).is_none());
}

#[test]
fn trusses_add_up_their_members() {
    let g = shape("{ truss: { bottom_diameter: 2, length: 3, bays: 2, around: 6 } }");
    let (r, bay, n) = (1.0f64, 1.5f64, 6.0);
    let chord = 2.0 * r * (PI / n).sin();
    let length = 3.0 * n * chord + 2.0 * n * bay + 4.0 * n * (chord * chord + bay * bay).sqrt();
    let truss = g.element("truss").unwrap();
    assert_eq!(truss.measure, Measure::Length);
    assert_close(truss.moments.measure, length, 1e-12);
    assert_close(truss.moments.centroid().unwrap().z, 1.5, 1e-12);
    let warren = shape(
        "{ truss: { bottom_diameter: 2, length: 3, bays: 3, around: 8, bracing: 'warren', rings: false } }",
    );
    let Kind::Lattice(lattice) = &warren.subparts[0].kind else {
        unreachable!()
    };
    assert_eq!(lattice.members().len(), 2 * 8 * 3);
    // Every diagonal is the same length: each node sits half way between two below it.
    let lengths: Vec<f64> = lattice
        .members()
        .iter()
        .map(|&[a, b]| (lattice.nodes()[a] - lattice.nodes()[b]).length())
        .collect();
    assert!(lengths.iter().all(|l| close(*l, lengths[0], 1e-12)));
}

#[test]
fn compounds_place_and_name_their_shapes() {
    // A service module: an open shell with a floor and a battery standing on its side.
    let g = shape(
        "{ compound: [
            { name: 'skin', shape: { shell: { bottom_diameter: 3, sections: [{ length: 2 }], cap_bottom: true } } },
            { name: 'battery', position: [1, 0, 0.5], rotation: [90, 0, 0],
              shape: { box: { size: [0.4, 0.3, 0.6] } } },
        ] }",
    );
    assert_eq!(
        names(&g),
        [
            "skin/wall",
            "skin/bottom",
            "battery/box",
            "battery/box/skin"
        ]
        .map(String::from)
    );
    // Turned 90° about X, the box's +Z (its length) runs along −Y.
    let battery = moments(&g, "battery/box").centroid().unwrap();
    assert!(
        (battery - DVec3::new(1.0, -0.3, 0.5)).length() < 1e-12,
        "{battery}"
    );
    let skin = g.surface_point("battery/box/skin", 1.0, 0.0).unwrap();
    assert!(close(skin.position.y, -0.6, 1e-12));
    assert!(
        refused(
            "{ compound: [{ shape: { sphere: { diameter: 1 } } }, { shape: { sphere: { diameter: 2 } } }] }"
        )
        .contains("both named")
    );
}

#[test]
fn a_model_stands_in_a_proxy_without_mass() {
    let g = shape(
        "{ model: { path: 'models/pods/capsule.glb', proxy: { cone_tank: {
            length: 2, bottom_diameter: 2.3, top_diameter: 1, dome: 0 } } } }",
    );
    assert!(matches!(&g.subparts[0].kind, Kind::Model(m) if m.path.ends_with("capsule.glb")));
    let elements = g.elements();
    assert!(!elements.is_empty() && elements.iter().all(|e| e.proxy));
    assert!(g.surface_point("proxy/barrel", 0.0, 0.5).is_some());
}

#[test]
fn written_out_curves_are_followed() {
    // The torus preset written out by hand: a full circle from its bottom.
    let by_hand = shape(
        "{ revolved: { regions: [{ name: 'tank', boundary: { start: [1, 0], segments: [
            { ellipse: { center: [1, 0.5], radii: [0.5, 0.5], to: 270 } } ] } }] } }",
    );
    let preset = shape("{ torus: { diameter: 3, tube_diameter: 1 } }");
    assert_close(
        moments(&by_hand, "tank").measure,
        moments(&preset, "tank").measure,
        1e-12,
    );
    // A bell from Bézier and taper segments closes on the axis.
    let bell = shape(
        "{ revolved: { regions: [{ name: 'bell', boundary: { start: [0, 0], segments: [
            { line: [1, 0] }, { bezier: { c1: [1, 1], c2: [0.4, 1], to: [0.4, 2] } },
            { taper: { to: [0, 3], profile: 'haack' } } ] } }] } }",
    );
    assert!(moments(&bell, "bell").measure > 0.0);
    // Regions must close and must not cross themselves.
    assert!(
        refused(
            "{ revolved: { regions: [{ name: 'open', boundary: { start: [1, 0], segments: [{ line: [1, 2] }] } }] } }"
        )
        .contains("must close")
    );
    assert!(
        refused(
            "{ revolved: { regions: [{ name: 'bow', boundary: { start: [1, 0], segments: [
                { line: [2, 1] }, { line: [2, 0] }, { line: [1, 1] }, { line: [1, 0] }] } }] } }"
        )
        .contains("crosses itself")
    );
    // Misspelt fields are errors, not silently ignored.
    assert!(json5::from_str::<ShapeDef>("{ sphere: { diametre: 2 } }").is_err());
}

#[test]
fn shorter_tanks_than_their_domes_are_refused() {
    assert!(
        refused("{ cylinder_tank: { diameter: 4, length: 2 } }").contains("shorter than its domes")
    );
    assert!(refused("{ sphere: { diameter: 0 } }").contains("positive"));
}

#[track_caller]
fn assert_node(g: &Geometry, name: &str, position: DVec3, direction: DVec3, size: f64) {
    let node = g
        .node(name)
        .unwrap_or_else(|| panic!("no node `{name}` in {:?}", g.nodes));
    assert!(
        (node.position - position).length() < 1e-9,
        "`{name}` at {}",
        node.position
    );
    assert!(
        (node.direction - direction).length() < 1e-9,
        "`{name}` facing {}",
        node.direction
    );
    assert_close(node.size, size, 1e-9);
}

#[test]
fn shapes_have_nodes_where_others_stack_on_them() {
    // A tank stacks by its rims, at the tips of its domes or the ends of its skirts.
    let tank = shape("{ cylinder_tank: { diameter: 3, length: 10, aft_skirt: 2 } }");
    let top = 2.0 + 10.0 - 1.5 * FRAC_1_SQRT_2;
    assert_node(&tank, "bottom", DVec3::ZERO, DVec3::NEG_Z, 3.0);
    assert_node(&tank, "top", DVec3::Z * top, DVec3::Z, 3.0);
    // A nose cone's tip is a point.
    let nose = shape(
        "{ shell: { bottom_diameter: 2, sections: [{ length: 3, top_diameter: 0, profile: 'haack' }] } }",
    );
    assert_node(&nose, "bottom", DVec3::ZERO, DVec3::NEG_Z, 2.0);
    assert_node(&nose, "top", DVec3::Z * 3.0, DVec3::Z, 0.0);
    // A fairing panel's nodes sit at the middle of its rims.
    let panel =
        shape("{ shell: { bottom_diameter: 4, sections: [{ length: 6 }], span: [0, 180] } }");
    assert_node(&panel, "bottom", DVec3::Y * 2.0, DVec3::NEG_Z, 4.0);
    assert_node(&panel, "top", DVec3::new(0.0, 2.0, 6.0), DVec3::Z, 4.0);
    let cone = shape("{ cone_tank: { length: 19, bottom_diameter: 2.7, top_diameter: 0 } }");
    assert_node(&cone, "bottom", DVec3::ZERO, DVec3::NEG_Z, 2.7);
    assert_node(&cone, "top", DVec3::Z * 19.0, DVec3::Z, 0.0);
    let torus = shape("{ torus: { diameter: 3, tube_diameter: 1 } }");
    assert_node(&torus, "top", DVec3::Z, DVec3::Z, 3.0);
    let ring = shape("{ ring: { diameter: 3, height: 0.1 } }");
    assert_node(&ring, "top", DVec3::Z * 0.1, DVec3::Z, 3.0);
    // A box reaches its corners.
    let cuboid = shape("{ box: { size: [0.6, 0.8, 2] } }");
    assert_node(&cuboid, "top", DVec3::Z * 2.0, DVec3::Z, 1.0);
    let truss = shape("{ truss: { bottom_diameter: 3, top_diameter: 2, length: 2 } }");
    assert_node(&truss, "bottom", DVec3::ZERO, DVec3::NEG_Z, 3.0);
    assert_node(&truss, "top", DVec3::Z * 2.0, DVec3::Z, 2.0);
    let tube = shape("{ beam: { length: 4, section: { tube: { diameter: 0.2, wall: 0.01 } } } }");
    assert_node(&tube, "top", DVec3::Z * 4.0, DVec3::Z, 0.2);
}

#[test]
fn a_wing_joins_at_its_root_and_carries_things_at_its_tip() {
    let g = shape("{ wing: { root_chord: 3, tip_chord: 1, span: 5, sweep: 2, dihedral: 4 } }");
    assert_node(&g, "root", DVec3::ZERO, DVec3::NEG_X, 3.0);
    let rise = 5.0 * 4f64.to_radians().tan();
    assert_node(&g, "tip", DVec3::new(5.0, -rise, -2.0), DVec3::X, 1.0);
}

#[test]
fn models_and_compounds_name_their_nodes() {
    let model = shape(
        "{ model: { path: 'models/pods/capsule.glb', nodes: [
            { name: 'bottom', position: [0, 0, 0], direction: [0, 0, -2], size: 2.3 },
            { name: 'top', position: [0, 0, 1.8], direction: [0, 0, 1], size: 1 } ] } }",
    );
    assert_node(&model, "bottom", DVec3::ZERO, DVec3::NEG_Z, 2.3);
    assert_node(&model, "top", DVec3::Z * 1.8, DVec3::Z, 1.0);
    // A service module: the shell's nodes keep their names, the battery's take its own.
    let module = shape(
        "{ compound: [
            { shape: { shell: { bottom_diameter: 3, sections: [{ length: 2 }] } } },
            { name: 'battery', position: [1, 0, 0.5], rotation: [90, 0, 0],
              shape: { box: { size: [0.4, 0.3, 0.6] } } },
        ] }",
    );
    assert_node(&module, "top", DVec3::Z * 2.0, DVec3::Z, 3.0);
    assert_node(
        &module,
        "battery/top",
        DVec3::new(1.0, -0.6, 0.5),
        DVec3::NEG_Y,
        0.5,
    );
    assert!(
        refused(
            "{ model: { path: 'a.glb', nodes: [{ name: 'top', position: [0, 0, 0], direction: [0, 0, 0] }] } }"
        )
        .contains("direction")
    );
}

#[test]
fn a_mirrored_shape_is_the_reflection_of_its_original() {
    let flip = DMat3::from_diagonal(DVec3::new(-1.0, 1.0, 1.0));
    for text in [
        // A swept, twisted, cambered wing: no turn makes the left one of the right.
        "{ wing: { root_chord: 3, tip_chord: 1, span: 5, sweep: 2, dihedral: 6, twist: -3, \
          airfoil: { naca: { thickness: 0.12, camber: 0.04 } } } }",
        // A fairing panel off to one side, and a truss.
        "{ compound: [{ shape: { shell: { bottom_diameter: 3, sections: [{ length: 4 }], \
          span: [10, 100] } }, position: [1, 2, 0] }] }",
        "{ truss: { bottom_diameter: 2, top_diameter: 1, length: 1.5, around: 5 } }",
    ] {
        let original = shape(text);
        let mirrored = shape(&format!("{{ mirrored: {text} }}"));
        assert_eq!(names(&original), names(&mirrored));
        for (a, b) in original.elements().iter().zip(mirrored.elements()) {
            let m = &b.moments;
            assert_close(m.measure, a.moments.measure, 1e-9);
            let first = flip * a.moments.first;
            assert!(
                (m.first - first).length() < 1e-9 * m.measure.max(1.0),
                "{}",
                a.name
            );
            let second = flip * a.moments.second * flip;
            let off = (m.second - second).to_cols_array();
            assert!(off.iter().all(|x| x.abs() < 1e-9), "{}: {off:?}", a.name);
        }
        for (a, b) in original.nodes.iter().zip(&mirrored.nodes) {
            assert_eq!(a.name, b.name);
            assert!((b.position - flip * a.position).length() < 1e-12);
            assert!((b.direction - flip * a.direction).length() < 1e-12);
        }
    }
    // The left wing's tip is out on the left, its leading edge still forward.
    let left = shape("{ mirrored: { wing: { root_chord: 2, tip_chord: 1, span: 4 } } }");
    let tip = left.node("tip").unwrap();
    assert!(tip.position.x < -3.9 && tip.position.z.abs() < 1e-9);
    assert!(refused("{ mirrored: { model: { path: 'pod.glb' } } }").contains("mirrored"));
}
