//! Item 8 measurement: how many pages of each PDF `yolowpdf` calls scanned (no text layer), and whether the whole
//! document is then handed to the scanner/OCR path (`doc.scanned`). Pages scanned inside a text document get no OCR.
//!
//!   cargo run -p search-core --release --features pdf-yolow --example pdf_scanned_pages -- <folder-of-pdfs>
fn main() {
    let dir = std::env::args().nth(1).expect("folder of pdfs");
    let mut pdfs: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "pdf")).collect();
    pdfs.sort();
    let (mut mixed, mut pages_total, mut scanned_total) = (0, 0, 0);
    for p in &pdfs {
        let bytes = std::fs::read(p).unwrap();
        let Ok(doc) = yolowpdf::extract_with(&bytes, &yolowpdf::Options::default()) else { println!("{}\tparse error", p.display()); continue };
        let scanned_pages = doc.pages.iter().filter(|pg| pg.markdown.contains("is a scanned image with no text layer")).count();
        pages_total += doc.pages.len();
        scanned_total += scanned_pages;
        if scanned_pages > 0 && !doc.scanned {
            mixed += 1;
        }
        println!("{}\tpages {}\tscanned pages {}\tdoc.scanned {}", p.file_name().unwrap().to_string_lossy(), doc.pages.len(), scanned_pages, doc.scanned);
    }
    println!("{} pdfs, {pages_total} pages, {scanned_total} scanned pages, {mixed} mixed documents (scanned pages inside a text document: no OCR)", pdfs.len());
}
