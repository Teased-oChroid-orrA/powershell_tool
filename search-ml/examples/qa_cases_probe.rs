//! Runs the bundled Flan-T5-small generator over a TSV of cases: question, contexts joined by `|||`, expectation.
//! Prints `index<TAB>answer` for an external scorer. Usage: qa_cases_probe search-ml/examples/data/qa_cases.tsv
use search_ml::generate_answer;

fn main() {
    let path = std::env::args().nth(1).expect("cases.tsv path");
    let text = std::fs::read_to_string(path).expect("read cases");
    for (index, line) in text.lines().filter(|l| !l.is_empty()).enumerate() {
        let mut fields = line.split('\t');
        let question = fields.next().unwrap();
        let context = fields.next().unwrap().split("|||").collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let answer = generate_answer(question, &context)
            .map(|(answer, _)| answer)
            .unwrap_or_else(|error| format!("ERROR {error:?}"));
        println!("{index}\t{}\t{}", started.elapsed().as_millis(), answer.replace('\n', " "));
    }
}
