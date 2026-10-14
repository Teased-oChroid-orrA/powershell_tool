//! Load cases: which supports and loads act in a solve, and how strongly. A problem without cases solves every
//! support and load as entered. With cases, `Problem::active_case` picks one; its entries start as "included at
//! factor 1" (a support or load added later therefore joins every case), and a case records only its differences.
use crate::problem::{Load, Problem};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadCase {
    pub name: String,
    /// Indices of `Problem::supports` that do not act in this case.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supports_off: Vec<usize>,
    /// `(index into Problem::loads, factor)` for every load whose factor is not 1 (0 switches it off).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_factors: Vec<(usize, f64)>,
}

impl LoadCase {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), supports_off: Vec::new(), load_factors: Vec::new() }
    }
    pub fn support_on(&self, i: usize) -> bool {
        !self.supports_off.contains(&i)
    }
    pub fn factor(&self, load: usize) -> f64 {
        self.load_factors.iter().find(|(l, _)| *l == load).map_or(1.0, |(_, f)| *f)
    }
    pub fn set_support(&mut self, i: usize, on: bool) {
        self.supports_off.retain(|s| *s != i);
        if !on {
            self.supports_off.push(i);
            self.supports_off.sort_unstable();
        }
    }
    pub fn set_factor(&mut self, load: usize, factor: f64) {
        self.load_factors.retain(|(l, _)| *l != load);
        if factor != 1.0 {
            self.load_factors.push((load, factor));
            self.load_factors.sort_by_key(|(l, _)| *l);
        }
    }
    /// Forget entry `removed` of the supports (the later ones move up).
    fn support_removed(&mut self, removed: usize) {
        self.supports_off = self.supports_off.iter().filter(|s| **s != removed).map(|s| if *s > removed { s - 1 } else { *s }).collect();
    }
    fn load_removed(&mut self, removed: usize) {
        self.load_factors = self.load_factors.iter().filter(|(l, _)| *l != removed).map(|(l, f)| (if *l > removed { l - 1 } else { *l }, *f)).collect();
    }
}

impl Load {
    /// The load with every magnitude multiplied by `f` (a prescribed position or hole number is unchanged).
    pub fn scaled(&self, f: f64) -> Load {
        let mut l = self.clone();
        match &mut l {
            Load::Pressure { p, .. } => *p *= f,
            Load::Traction { tx, ty, tz, .. } => (*tx, *ty, *tz) = (*tx * f, *ty * f, *tz * f),
            Load::Force { fx, fy, fz, .. } => (*fx, *fy, *fz) = (*fx * f, *fy * f, *fz * f),
            Load::Bearing { fx, fy, .. } => (*fx, *fy) = (*fx * f, *fy * f),
            Load::Point { fx, fy, fz, .. } => (*fx, *fy, *fz) = (*fx * f, *fy * f, *fz * f),
            Load::Body { bx, by, bz } => (*bx, *by, *bz) = (*bx * f, *by * f, *bz * f),
        }
        l
    }
}

impl Problem {
    /// The active case, when there are cases.
    pub fn case(&self) -> Option<&LoadCase> {
        if self.cases.is_empty() {
            None
        } else {
            self.cases.get(self.active_case.min(self.cases.len() - 1))
        }
    }
    /// Index of the active case (0 when there are none).
    pub fn active_case_index(&self) -> usize {
        self.active_case.min(self.cases.len().saturating_sub(1))
    }
    /// The problem as one solve sees it: only the active case's supports and loads, at its factors. Without cases it
    /// is the problem itself. Everything a solve reads comes from this, so a case needs no other support in the solvers.
    pub fn resolved(&self) -> Problem {
        let Some(case) = self.case() else { return self.clone() };
        let mut p = self.clone();
        p.cases.clear();
        p.active_case = 0;
        p.supports = self.supports.iter().enumerate().filter(|(i, _)| case.support_on(*i)).map(|(_, s)| s.clone()).collect();
        p.loads = self.loads.iter().enumerate().filter(|(i, _)| case.factor(*i) != 0.0).map(|(i, l)| if case.factor(i) == 1.0 { l.clone() } else { l.scaled(case.factor(i)) }).collect();
        p
    }
    /// Add a case (it becomes the active one) and return its index. The first case also keeps what the problem solved
    /// as before as "Case 1", so naming a second case never changes the first one's result.
    pub fn add_case(&mut self, name: &str) -> usize {
        if self.cases.is_empty() {
            self.cases.push(LoadCase::new("Case 1"));
        }
        self.cases.push(LoadCase::new(name));
        self.active_case = self.cases.len() - 1;
        self.active_case
    }
    pub fn remove_case(&mut self, i: usize) {
        if i < self.cases.len() {
            self.cases.remove(i);
            // A single case left is the plain problem again.
            if self.cases.len() <= 1 {
                self.cases.clear();
            }
            self.active_case = self.active_case.min(self.cases.len().saturating_sub(1));
        }
    }
    pub fn remove_support(&mut self, i: usize) {
        if i < self.supports.len() {
            self.supports.remove(i);
            self.cases.iter_mut().for_each(|c| c.support_removed(i));
        }
    }
    pub fn remove_load(&mut self, i: usize) {
        if i < self.loads.len() {
            self.loads.remove(i);
            self.cases.iter_mut().for_each(|c| c.load_removed(i));
        }
    }
}
