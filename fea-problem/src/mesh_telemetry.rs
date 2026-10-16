//! Local-first benchmark records. Only the numerical benchmark pipeline can mint
//! verified evidence; deserializing a record does not make it a training example.
use crate::{Problem, Solved};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;
pub const SCHEMA: u32 = 1;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_STORE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PerformanceRecord {
    schema: u32,
    workload: String,
    application_version: String,
    source_revision: String,
    architecture: String,
    logical_parallelism: usize,
    nodes: usize,
    elements: usize,
    dofs: usize,
    total_seconds: f64,
    /// Ordered observed stages, including validation/meshing/analysis. The analysis
    /// stage includes any adaptation and recovery; no invented sub-stage split.
    stages: Vec<(String, f64)>,
    /// No portable peak-RSS measurement is available in this pure library.
    peak_bytes: Option<u64>,
    benchmark_source: String,
    benchmark_errors: Vec<f64>,
    benchmark_tolerance: f64,
    equilibrium_error: f64,
    minimum_scaled_jacobian: f64,
}
impl PerformanceRecord {
    pub fn workload(&self) -> &str {
        &self.workload
    }
    pub fn total_seconds(&self) -> f64 {
        self.total_seconds
    }
    fn valid(&self) -> bool {
        self.source_revision.len() == 40
            && self.source_revision.bytes().all(|b| b.is_ascii_hexdigit())
            && self.schema == SCHEMA
            && !self.workload.is_empty()
            && self.workload.len() <= 128
            && !self.application_version.is_empty()
            && self.application_version.len() <= 128
            && !self.benchmark_source.is_empty()
            && self.benchmark_source.len() <= 1024
            && self.logical_parallelism > 0
            && self.elements > 0
            && self.nodes > 0
            && self.dofs > 0
            && self.total_seconds.is_finite()
            && self.total_seconds >= 0.
            && self.stages.len() <= 32
            && self
                .stages
                .iter()
                .all(|(s, t)| s.len() <= 128 && t.is_finite() && *t >= 0.)
            && self.benchmark_tolerance.is_finite()
            && self.benchmark_tolerance > 0.
            && !self.benchmark_errors.is_empty()
            && self.benchmark_errors.len() <= 32
            && self
                .benchmark_errors
                .iter()
                .all(|e| e.is_finite() && e.abs() <= self.benchmark_tolerance)
            && self.equilibrium_error.is_finite()
            && self.equilibrium_error.abs() < 1e-6
            && self.minimum_scaled_jacobian.is_finite()
            && self.minimum_scaled_jacobian > 0.
    }
}
/// Not Deserialize and no public constructor. A caller-provided verification boolean is impossible.
#[derive(Clone, Debug)]
pub struct VerifiedPerformance {
    record: PerformanceRecord,
}
impl VerifiedPerformance {
    pub fn record(&self) -> &PerformanceRecord {
        &self.record
    }
}

/// Run the existing solver and independent, unchanged template benchmark checks.
/// Edited/non-benchmark problems cannot become successful performance evidence.
/// This opt-in path does not change normal solves or run extra candidate analyses.
pub fn run_verified(
    p: &Problem,
    import: Option<&str>,
    source_revision: &str,
) -> Result<(Solved, VerifiedPerformance), String> {
    if source_revision.len() != 40 || !source_revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(
            "an exact 40-digit source revision is required for benchmark provenance".into(),
        );
    }
    if !crate::benchmark::supports(p) {
        return Err("no independent benchmark for this exact problem; do not use it as verified training data".into());
    }
    let start = Instant::now();
    let stages = RefCell::new(Vec::<(String, f64)>::new());
    let last = RefCell::new((String::new(), start));
    let solved = crate::solve::solve_observed(p, import, &|name| {
        let now = Instant::now();
        let mut old = last.borrow_mut();
        if !old.0.is_empty() {
            stages
                .borrow_mut()
                .push((old.0.clone(), now.duration_since(old.1).as_secs_f64()));
        }
        *old = (name.to_owned(), now);
        Ok(())
    })?;
    let end = Instant::now();
    let old = last.into_inner();
    if !old.0.is_empty() {
        stages
            .borrow_mut()
            .push((old.0, end.duration_since(old.1).as_secs_f64()));
    }
    let benchmark = crate::benchmark::check(&solved).ok_or(
        "no independent benchmark for this exact problem; do not use it as verified training data",
    )?;
    if !benchmark.passed() {
        return Err(
            "independent benchmark failed; exclude from successful performance evidence".into(),
        );
    }
    let quality = solved.model.mesh.quality();
    // Identity covers the entire serialized physical problem, not topology alone.
    // FNV is a reproducibility key, not a cryptographic or anonymization guarantee.
    let mut fingerprint = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(p).map_err(|e| e.to_string())? {
        fingerprint ^= byte as u64;
        fingerprint = fingerprint.wrapping_mul(0x100000001b3);
    }
    let record = PerformanceRecord {
        schema: SCHEMA,
        workload: format!("problem-fnv1a-v1-{fingerprint:016x}"),
        application_version: env!("CARGO_PKG_VERSION").into(),
        source_revision: source_revision.into(),
        architecture: std::env::consts::ARCH.into(),
        logical_parallelism: std::thread::available_parallelism().map_or(1, usize::from),
        nodes: solved.summary.nodes,
        elements: solved.summary.elements,
        dofs: solved.summary.dofs,
        total_seconds: end.duration_since(start).as_secs_f64(),
        stages: stages.into_inner(),
        peak_bytes: None,
        benchmark_source: benchmark.source.into(),
        benchmark_errors: benchmark.checks.iter().map(|c| c.error()).collect(),
        benchmark_tolerance: benchmark.tolerance,
        equilibrium_error: solved.summary.equilibrium_error,
        minimum_scaled_jacobian: quality.min_scaled_jacobian,
    };
    if quality.inverted != 0 || !record.valid() {
        return Err("benchmark telemetry failed numerical or schema acceptance".into());
    }
    Ok((solved, VerifiedPerformance { record }))
}

#[derive(Clone, Debug)]
struct Entry {
    record: PerformanceRecord,
    trusted: bool,
}
/// Bounded pure-data store; the application owns opt-in collection, atomic file I/O
/// and deletion. Imported evidence remains quarantined until numerical revalidation.
#[derive(Default, Debug)]
pub struct BenchmarkStore {
    entries: VecDeque<Entry>,
}
impl BenchmarkStore {
    pub fn record(&mut self, evidence: &VerifiedPerformance) {
        let r = &evidence.record;
        self.entries.retain(|e| {
            !(e.record.workload == r.workload
                && e.record.architecture == r.architecture
                && e.record.logical_parallelism == r.logical_parallelism
                && e.record.application_version == r.application_version
                && e.record.source_revision == r.source_revision)
        });
        self.entries.push_back(Entry {
            record: r.clone(),
            trusted: true,
        });
        while self.entries.len() > MAX_RECORDS
            || self.encode().map_or(true, |s| s.len() > MAX_STORE_BYTES)
        {
            self.entries.pop_front();
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn verified(&self) -> impl Iterator<Item = &PerformanceRecord> {
        self.entries.iter().filter(|e| e.trusted).map(|e| &e.record)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn encode(&self) -> Result<String, String> {
        let mut out = format!("{{\"schema\":{SCHEMA}}}\n");
        for e in &self.entries {
            out.push_str(&serde_json::to_string(&e.record).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        Ok(out)
    }
    /// Reject unknown schema; ignore corrupt/oversized records; cap input before
    /// parsing. No persisted `trusted` flag is read, so imports cannot mint proof.
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > MAX_STORE_BYTES {
            return Err("benchmark store exceeds byte budget".into());
        }
        let mut lines = text.lines();
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Header {
            schema: u32,
        }
        let h: Header = serde_json::from_str(lines.next().ok_or("missing benchmark schema")?)
            .map_err(|e| e.to_string())?;
        if h.schema != SCHEMA {
            return Err("unsupported benchmark schema; explicit migration required".into());
        }
        let mut out = Self::default();
        for line in lines {
            if line.len() > MAX_RECORD_BYTES {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<PerformanceRecord>(line) {
                if record.valid() {
                    out.entries.push_back(Entry {
                        record,
                        trusted: false,
                    });
                }
            }
            while out.entries.len() > MAX_RECORDS {
                out.entries.pop_front();
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_requires_a_real_independent_benchmark_and_imports_are_quarantined() {
        let p = crate::templates::templates()
            .into_iter()
            .find(|(n, _)| *n == "Cantilever beam")
            .unwrap()
            .1;
        let (_, proof) =
            run_verified(&p, None, "0123456789012345678901234567890123456789").unwrap();
        let mut store = BenchmarkStore::default();
        store.record(&proof);
        store.record(&proof);
        assert_eq!(store.len(), 1);
        assert_eq!(store.verified().count(), 1);
        let text = store.encode().unwrap();
        assert!(!text.contains("supports"));
        assert!(!text.contains("material"));
        let restored = BenchmarkStore::decode(&text).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored.verified().count(), 0);
        let corrupt = format!("{text}not json\n{{\"schema\":999}}\n");
        assert_eq!(BenchmarkStore::decode(&corrupt).unwrap().len(), 1);
        assert!(BenchmarkStore::decode("{\"schema\":999}\n").is_err());
        assert!(BenchmarkStore::decode(&"x".repeat(MAX_STORE_BYTES + 1)).is_err());
        store.clear();
        assert!(store.is_empty());
        let mut edited = p;
        edited.name = "unverified copy".into();
        assert!(
            matches!(run_verified(&edited,None,"0123456789012345678901234567890123456789"),Err(e) if e.contains("no independent benchmark"))
        );
    }
}
