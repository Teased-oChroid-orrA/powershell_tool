//! Facet-preserving tetrahedral recovery (`plc_conform::conform_preserving`): the triangular sides of a pyramid layer are frozen
//! facets of the tetrahedral region behind it and come out exactly as given. `docs/issue-46-phase-102.md`.

use solid_kernel::plc_conform::{conform, conform_preserving, Options};
use solid_kernel::V3;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The tetrahedral region of the block [0.5, 1] x [0, 1]^2 behind a pyramid whose base is the whole cross-section at x = 0.5 and
/// whose apex is `apex`: the five outer faces (two triangles each) and the four sides of the pyramid. Returns the vertices, the
/// facets (outward from the region) and which of them are the pyramid sides.
fn region(apex: V3) -> (Vec<V3>, Vec<[usize; 3]>, Vec<bool>) {
    let q = [[0.5, 0., 0.], [0.5, 1., 0.], [0.5, 1., 1.], [0.5, 0., 1.]];
    let f = [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]];
    let mut v: Vec<V3> = q.to_vec();
    v.extend(f);
    v.push(apex);
    let centre_block = [0.75, 0.5, 0.5];
    let centre_pyramid = [(0.5 + apex[0]) / 2., 0.5, 0.5];
    let mut facets = Vec::new();
    let mut frozen = Vec::new();
    let mut push = |t: [usize; 3], pyramid_side: bool, v: &Vec<V3>| {
        let n = cross(sub(v[t[1]], v[t[0]]), sub(v[t[2]], v[t[0]]));
        let c = [
            (v[t[0]][0] + v[t[1]][0] + v[t[2]][0]) / 3.,
            (v[t[0]][1] + v[t[1]][1] + v[t[2]][1]) / 3.,
            (v[t[0]][2] + v[t[1]][2] + v[t[2]][2]) / 3.,
        ];
        // outer faces point away from the block, pyramid sides into the pyramid
        let toward = if pyramid_side {
            sub(centre_pyramid, c)
        } else {
            sub(centre_block, c)
        };
        let ok = (dot(n, toward) > 0.) == pyramid_side;
        facets.push(if ok { t } else { [t[0], t[2], t[1]] });
        frozen.push(pyramid_side);
    };
    for i in 0..4 {
        let j = (i + 1) % 4;
        push([i, j, 8], true, &v);
        // lateral wall quad (q_i, q_j, f_j, f_i) as two triangles
        push([i, j, 4 + j], false, &v);
        push([i, 4 + j, 4 + i], false, &v);
    }
    push([4, 5, 6], false, &v);
    push([4, 6, 7], false, &v);
    (v, facets, frozen)
}

fn volume(points: &[V3], cells: &[[usize; 4]]) -> f64 {
    cells
        .iter()
        .map(|c| {
            let [a, b, cc, d] = c.map(|i| points[i]);
            dot(sub(a, d), cross(sub(b, d), sub(cc, d))).abs() / 6.
        })
        .sum()
}

#[test]
fn the_pyramid_sides_of_a_single_pyramid_survive_as_given() {
    let apex = [0.75, 0.5, 0.5];
    let (v, facets, frozen) = region(apex);
    let options = Options {
        max_steiner: 20_000,
        protect_encroached: false,
        ..Options::default()
    };
    let m = conform_preserving(&v, &facets, &frozen, options, |_| true).unwrap();
    // the region is the block minus the pyramid
    let want = 0.5 - (1. / 3.) * 1. * 0.25;
    assert!(
        (volume(&m.points, &m.cells) - want).abs() < 1e-12,
        "{} vs {want}",
        volume(&m.points, &m.cells)
    );
    // every frozen facet comes out exactly once, with its own three vertices
    for (i, _) in frozen.iter().enumerate().filter(|(_, f)| **f) {
        let mut want: Vec<usize> = facets[i].to_vec();
        want.sort();
        let got: Vec<_> = m
            .facets
            .iter()
            .filter(|(_, parent)| *parent == i)
            .map(|(t, _)| {
                let mut s = t.to_vec();
                s.sort();
                s
            })
            .collect();
        assert_eq!(got, vec![want], "frozen facet {i}");
    }
    // the unfrozen faces may have been split; the segments of frozen facets are untouched
    assert!(
        m.segments
            .iter()
            .filter(|s| s.iter().all(|&i| i < v.len()))
            .count()
            >= 8 + 4
    );
    // the unfrozen recovery of the same shell is also available (it is free to put Steiner points on the sides)
    conform(&v, &facets, options, |_| true).unwrap();
}

/// Schonhardt's polyhedron: a triangular prism whose top is twisted against its bottom, each lateral quadrilateral split along
/// a diagonal. One choice of diagonals gives a polyhedron that no tetrahedralization without new vertices can fill; the other
/// gives a convex-position one that Delaunay fills at once. `reflex` picks the first kind.
fn twisted_prism(reflex: bool) -> (Vec<V3>, Vec<[usize; 3]>) {
    let ring = |z: f64, phase: f64| -> Vec<V3> {
        (0..3)
            .map(|i| {
                let a = phase + i as f64 * 2. * std::f64::consts::PI / 3.;
                [a.cos(), a.sin(), z]
            })
            .collect()
    };
    let mut v = ring(0., 0.);
    v.extend(ring(1., std::f64::consts::PI / 6.));
    let mut facets = vec![[0, 2, 1], [3, 4, 5]];
    for i in 0..3 {
        let (a0, a1, b0, b1) = (i, (i + 1) % 3, 3 + i, 3 + (i + 1) % 3);
        if reflex {
            facets.extend([[a0, a1, b1], [a0, b1, b0]]);
        } else {
            facets.extend([[a0, a1, b0], [a1, b1, b0]]);
        }
    }
    // outward orientation by the volume sign of the shell
    let signed: f64 = facets
        .iter()
        .map(|t| dot(v[t[0]], cross(v[t[1]], v[t[2]])) / 6.)
        .sum();
    if signed < 0. {
        for t in &mut facets {
            t.swap(1, 2);
        }
    }
    (v, facets)
}

#[test]
fn a_polyhedron_that_needs_a_new_vertex_is_reported_when_every_facet_is_frozen() {
    let options = Options {
        max_steiner: 2_000,
        protect_encroached: false,
        ..Options::default()
    };
    let mut outcomes = Vec::new();
    for reflex in [true, false] {
        let (v, facets) = twisted_prism(reflex);
        // free to add vertices on the shell, both are recovered
        conform(&v, &facets, options, |_| true)
            .unwrap_or_else(|e| panic!("free recovery of the twisted prism ({reflex}): {e}"));
        let frozen = vec![true; facets.len()];
        outcomes.push(
            match conform_preserving(&v, &facets, &frozen, options, |_| true) {
                Ok(m) => {
                    assert_eq!(m.facets.len(), facets.len(), "kept as given");
                    assert!(m.report.segment_steiner + m.report.facet_steiner == 0);
                    true
                }
                Err(e) => {
                    assert!(e.detail.contains("frozen"), "{e:?}");
                    false
                }
            },
        );
    }
    // the reflex choice cannot be tetrahedralized without a Steiner point, the other can
    assert_eq!(outcomes, vec![false, true], "reflex, convex");
}

/// The same block with an `n x n` interface: one pyramid (apex at `height` above its quad's centre) per quad, the outer faces
/// gridded to match the interface edges. The pyramid sides are frozen. Returns vertices, facets, frozen flags and the volume of
/// the region (block minus pyramids).
fn grid_region(n: usize, height: f64) -> (Vec<V3>, Vec<[usize; 3]>, Vec<bool>, f64) {
    let h = 1.0 / n as f64;
    let mut v: Vec<V3> = Vec::new();
    let mut id = std::collections::BTreeMap::new();
    let mut node = |v: &mut Vec<V3>, p: V3| -> usize {
        let key = [
            (p[0] * 1e6).round() as i64,
            (p[1] * 1e6).round() as i64,
            (p[2] * 1e6).round() as i64,
        ];
        *id.entry(key).or_insert_with(|| {
            v.push(p);
            v.len() - 1
        })
    };
    let block_centre = [0.75, 0.5, 0.5];
    let mut facets = Vec::new();
    let mut frozen = Vec::new();
    let mut add = |v: &Vec<V3>,
                   t: [usize; 3],
                   toward: V3,
                   pyramid: bool,
                   facets: &mut Vec<[usize; 3]>,
                   frozen: &mut Vec<bool>| {
        let nrm = cross(sub(v[t[1]], v[t[0]]), sub(v[t[2]], v[t[0]]));
        let c = [
            (v[t[0]][0] + v[t[1]][0] + v[t[2]][0]) / 3.,
            (v[t[0]][1] + v[t[1]][1] + v[t[2]][1]) / 3.,
            (v[t[0]][2] + v[t[1]][2] + v[t[2]][2]) / 3.,
        ];
        let ok = dot(nrm, sub(toward, c)) > 0.;
        facets.push(if ok == pyramid { t } else { [t[0], t[2], t[1]] });
        frozen.push(pyramid);
    };
    // pyramids: sides frozen, oriented into the pyramid
    for i in 0..n {
        for j in 0..n {
            let q = [
                [0.5, i as f64 * h, j as f64 * h],
                [0.5, (i + 1) as f64 * h, j as f64 * h],
                [0.5, (i + 1) as f64 * h, (j + 1) as f64 * h],
                [0.5, i as f64 * h, (j + 1) as f64 * h],
            ];
            let apex = [0.5 + height, (i as f64 + 0.5) * h, (j as f64 + 0.5) * h];
            let qi: Vec<usize> = q.iter().map(|p| node(&mut v, *p)).collect();
            let a = node(&mut v, apex);
            let inside = [0.5 + height / 2., apex[1], apex[2]];
            for k in 0..4 {
                add(
                    &v,
                    [qi[k], qi[(k + 1) % 4], a],
                    inside,
                    true,
                    &mut facets,
                    &mut frozen,
                );
            }
        }
    }
    // far face x = 1 and the four lateral walls, gridded (n cells along the interface directions, 1 along x)
    for i in 0..n {
        for j in 0..n {
            let (y0, y1, z0, z1) = (
                i as f64 * h,
                (i + 1) as f64 * h,
                j as f64 * h,
                (j + 1) as f64 * h,
            );
            let c =
                [[1., y0, z0], [1., y1, z0], [1., y1, z1], [1., y0, z1]].map(|p| node(&mut v, p));
            add(
                &v,
                [c[0], c[1], c[2]],
                block_centre,
                false,
                &mut facets,
                &mut frozen,
            );
            add(
                &v,
                [c[0], c[2], c[3]],
                block_centre,
                false,
                &mut facets,
                &mut frozen,
            );
        }
        // walls: y = 0, y = 1 (running along z -> use index i for z), z = 0, z = 1 (running along y)
        let (a0, a1) = (i as f64 * h, (i + 1) as f64 * h);
        for (fixed_axis, fixed) in [(1usize, 0.0), (1, 1.0), (2, 0.0), (2, 1.0)] {
            let mk = |x: f64, a: f64| -> V3 {
                if fixed_axis == 1 {
                    [x, fixed, a]
                } else {
                    [x, a, fixed]
                }
            };
            let c = [mk(0.5, a0), mk(1.0, a0), mk(1.0, a1), mk(0.5, a1)].map(|p| node(&mut v, p));
            add(
                &v,
                [c[0], c[1], c[2]],
                block_centre,
                false,
                &mut facets,
                &mut frozen,
            );
            add(
                &v,
                [c[0], c[2], c[3]],
                block_centre,
                false,
                &mut facets,
                &mut frozen,
            );
        }
    }
    let volume = 0.5 - height / 3.;
    (v, facets, frozen, volume)
}

#[test]
fn a_grid_of_pyramids_keeps_every_side_and_fills_the_region_between_them() {
    for (n, height) in [(2usize, 0.25), (3, 0.5 / 3.)] {
        let (v, facets, frozen, want) = grid_region(n, height);
        let options = Options {
            max_steiner: 50_000,
            protect_encroached: false,
            ..Options::default()
        };
        let m = conform_preserving(&v, &facets, &frozen, options, |_| true)
            .unwrap_or_else(|e| panic!("n = {n}: {e}"));
        assert!(
            (volume(&m.points, &m.cells) - want).abs() < 1e-12,
            "n = {n}: {} vs {want}",
            volume(&m.points, &m.cells)
        );
        for (i, _) in frozen.iter().enumerate().filter(|(_, f)| **f) {
            let mut w = facets[i].to_vec();
            w.sort();
            let got: Vec<_> = m
                .facets
                .iter()
                .filter(|(_, p)| *p == i)
                .map(|(t, _)| {
                    let mut s = t.to_vec();
                    s.sort();
                    s
                })
                .collect();
            assert_eq!(got, vec![w], "n = {n}: frozen facet {i}");
        }
        // with quality refinement: the sides still come out as given; cells whose centre would split one stay (counted)
        let refined = Options {
            max_radius_edge: Some(2.),
            ..options
        };
        let q = conform_preserving(&v, &facets, &frozen, refined, |_| true)
            .unwrap_or_else(|e| panic!("n = {n} refined: {e}"));
        assert!((volume(&q.points, &q.cells) - want).abs() < 1e-12);
        for (i, _) in frozen.iter().enumerate().filter(|(_, f)| **f) {
            assert_eq!(
                q.facets.iter().filter(|(_, p)| *p == i).count(),
                1,
                "n = {n}: refined, frozen facet {i}"
            );
        }
        eprintln!("n = {n} refined to radius-edge 2: {} cells, worst {:.2}, {} cells left that would have split a frozen side", q.cells.len(), q.report.max_radius_edge_ratio, q.report.unresolved_bad_cells);
        eprintln!("n = {n}: {} cells, {} Steiner points (segments {}, facets {}), worst radius-edge {:.2}", m.cells.len(), m.report.segment_steiner + m.report.facet_steiner, m.report.segment_steiner, m.report.facet_steiner, m.report.max_radius_edge_ratio);
    }
}
