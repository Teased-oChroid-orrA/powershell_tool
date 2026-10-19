//! Run `plc_conform::conform` on a shell dumped by the ABC harness (`ABC_DUMP_DIR=dir abc-corpus ...` writes `v x y z` /
//! `t a b c` lines per body): `plc-run FILE.plc`. `PLC_MERGE=1` sets `Options::merge_coplanar`. For offline debugging of
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
    let o = Options { max_steiner: 200000, max_cells: 20_000_000, merge_coplanar: std::env::var_os("PLC_MERGE").is_some(), ..Default::default() };
    let start = std::time::Instant::now();
    match conform(&v, &t, o, |_| start.elapsed().as_secs() < 100) {
        Ok(c) => println!("ok {:?}", c.report),
        Err(e) => println!("err {e}"),
    }
}
