//! Run `plc_conform::conform` on a shell dumped by the ABC harness (`ABC_DUMP_DIR=dir abc-corpus ...` writes `v x y z` /
//! `t a b c` lines per body): `plc-run FILE.plc`. `PLC_MERGE=1` sets `Options::merge_coplanar`; `PLC_REFINE=1` adds the sliver recipe. For offline debugging of
//! the conform stage of one failing shell.
use solid_kernel::plc_conform::{conform, Options};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut v, mut t) = (Vec::new(), Vec::new());
    for l in text.lines() {
        let x: Vec<&str> = l.split_whitespace().collect();
        match x[0] {
            "v" => v.push([x[1].parse::<f64>().unwrap(), x[2].parse().unwrap(), x[3].parse().unwrap()]),
            _ => t.push([x[1].parse::<usize>().unwrap(), x[2].parse().unwrap(), x[3].parse().unwrap()]),
        }
    }
    let o = Options { max_steiner: 200000, max_cells: 20_000_000, merge_coplanar: std::env::var_os("PLC_MERGE").is_some(), protect_encroached: std::env::var_os("PLC_NOPROTECT").is_none(), unprotected_fallback: std::env::var_os("PLC_NOFALLBACK").is_none(), ..Default::default() };
    // PLC_REFINE=1: the abc-corpus `--refine` sliver recipe (scaled Jacobian 0.2, six flip passes); PLC_QSEC sets its time
    let mut o = o;
    if std::env::var_os("PLC_REFINE").is_some() {
        o.min_scaled_jacobian = Some(0.2);
        o.max_quality_seconds = std::env::var("PLC_QSEC").ok().and_then(|s| s.parse().ok()).unwrap_or(20.);
        o.reconnect = Some(fea_core::meshing::tet_flip::FlipOptions { max_passes: 6, ..Default::default() });
    }
    let secs: u64 = std::env::var("PLC_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(100);
    let start = std::time::Instant::now();
    let last = std::cell::Cell::new(0usize);
    let result = conform(&v, &t, o, |p| {
        last.set(p.steiner);
        // read once: the callback runs for every split, and reading the environment there took a quarter of the run
        start.elapsed().as_secs() < secs
    });
    println!("last progress steiner {}", last.get());
    match result {
        Ok(c) => println!("ok {:?} secs {:.2}", c.report, start.elapsed().as_secs_f64()),
        Err(e) => println!("err {e} secs {:.2}", start.elapsed().as_secs_f64()),
    }
}
