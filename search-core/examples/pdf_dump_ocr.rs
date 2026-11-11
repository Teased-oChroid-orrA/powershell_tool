fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap();
    let t = std::time::Instant::now();
    let (l, _) = search_core::extraction::extract_pdf_lines(&b, 600, None, true);
    eprintln!("{:.2}s", t.elapsed().as_secs_f64());
    for x in l.unwrap_or_default() { println!("{x}"); }
}
