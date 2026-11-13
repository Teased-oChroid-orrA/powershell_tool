//! cargo run -p search-core --release --example pdf_grep -- file.pdf word... | @line
//! Prints the extracted lines of a PDF that contain a word (what the index sees), or 40 lines around `@line`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("read");
    let (lines, _) = search_core::extraction::extract_pdf_lines(&bytes, 600, None, false);
    let lines = lines.expect("no text");
    println!("{} lines", lines.len());
    let centres: Vec<usize> = args[2..]
        .iter()
        .filter_map(|a| a.strip_prefix('@')?.parse().ok())
        .collect();
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        let by_word = args[2..]
            .iter()
            .any(|w| !w.starts_with('@') && lower.contains(w.as_str()));
        let by_place = centres.iter().any(|&c| i + 20 >= c && i <= c + 20);
        if by_word || by_place {
            println!("{i}: {line}");
        }
    }
}
