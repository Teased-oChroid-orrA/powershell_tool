fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap();
    let (l, _) = search_core::extraction::extract_pdf_lines(&b, 600, None, false);
    for x in l.unwrap_or_default() { println!("{x}"); }
}
