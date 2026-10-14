//! Reverse-mode automatic differentiation on a **local** tape: the record of one kernel evaluation (one element, one Gauss
//! point), never of a whole solve. The tape is a flat arena reused across evaluations (`reset` keeps the capacity), has a hard
//! node limit (exceeding it is an error, not unbounded growth) and is recorded on the path the plain evaluation took.
//!
//! `Tape<T>` is generic over its value type: `Tape<f64>` is ordinary reverse mode; `Tape<Dual<f64, 1>>` swept with dual seeds is
//! forward-over-reverse, which yields Hessian-vector products without forming a Hessian.

use super::scalar::{powi_by_mul, Scalar};
use std::cell::{Cell, RefCell};
use std::ops::{Add, Div, Mul, Neg, Sub};

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Node<T: Scalar> {
    a: u32,
    b: u32,
    da: T,
    db: T,
}

/// Why a reverse sweep could not be trusted.
#[derive(Debug, Clone, PartialEq)]
pub enum TapeError {
    /// The kernel recorded more operations than the tape's limit; the gradient would be silently incomplete.
    Overflow { limit: usize },
}

impl std::fmt::Display for TapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overflow { limit } => write!(f, "the local tape overflowed its limit of {limit} operations"),
        }
    }
}

pub struct Tape<T: Scalar> {
    nodes: RefCell<Vec<Node<T>>>,
    limit: usize,
    overflow: Cell<bool>,
}

impl<T: Scalar> Tape<T> {
    /// A tape of at most `limit` recorded operations.
    pub fn with_limit(limit: usize) -> Self {
        Self { nodes: RefCell::new(Vec::with_capacity(limit.min(1 << 16))), limit, overflow: Cell::new(false) }
    }

    /// Forget the recording, keep the memory.
    pub fn reset(&self) {
        self.nodes.borrow_mut().clear();
        self.overflow.set(false);
    }

    pub fn len(&self) -> usize {
        self.nodes.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes the recorded operations occupy.
    pub fn bytes(&self) -> usize {
        self.len() * std::mem::size_of::<Node<T>>()
    }

    pub fn capacity_bytes(&self) -> usize {
        self.nodes.borrow().capacity() * std::mem::size_of::<Node<T>>()
    }

    pub fn check(&self) -> Result<(), TapeError> {
        if self.overflow.get() {
            Err(TapeError::Overflow { limit: self.limit })
        } else {
            Ok(())
        }
    }

    /// An independent variable of the recording.
    pub fn var(&self, v: T) -> Rev<'_, T> {
        let idx = self.push(NONE, NONE, T::cst(0.0), T::cst(0.0));
        Rev { tape: Some(self), idx, v }
    }

    fn push(&self, a: u32, b: u32, da: T, db: T) -> u32 {
        let mut n = self.nodes.borrow_mut();
        if n.len() >= self.limit {
            self.overflow.set(true);
            return NONE;
        }
        n.push(Node { a, b, da, db });
        (n.len() - 1) as u32
    }

    /// Adjoints of every recorded node for the output `out`, seeded with `seed`: `adj[x.index()]` is `seed * d out / d x` for
    /// each variable `x`.
    pub fn sweep(&self, out: &Rev<'_, T>, seed: T) -> Vec<T> {
        let nodes = self.nodes.borrow();
        let mut adj = vec![T::cst(0.0); nodes.len()];
        if out.idx != NONE {
            adj[out.idx as usize] = seed;
            for i in (0..=out.idx as usize).rev() {
                let n = nodes[i];
                let ai = adj[i];
                if n.a != NONE {
                    adj[n.a as usize] = adj[n.a as usize] + ai * n.da;
                }
                if n.b != NONE {
                    adj[n.b as usize] = adj[n.b as usize] + ai * n.db;
                }
            }
        }
        adj
    }
}

/// A value on a tape (or a constant, which has no tape and records nothing).
#[derive(Clone, Copy)]
pub struct Rev<'t, T: Scalar> {
    tape: Option<&'t Tape<T>>,
    idx: u32,
    pub v: T,
}

impl<'t, T: Scalar> Rev<'t, T> {
    /// Position of the variable on its tape, for reading its adjoint.
    pub fn index(&self) -> Option<usize> {
        (self.idx != NONE).then_some(self.idx as usize)
    }

    fn unary(self, v: T, d: T) -> Self {
        match self.tape {
            Some(t) if self.idx != NONE => Rev { tape: Some(t), idx: t.push(self.idx, NONE, d, T::cst(0.0)), v },
            _ => Rev { tape: None, idx: NONE, v },
        }
    }

    fn binary(a: Self, b: Self, v: T, da: T, db: T) -> Self {
        let tape = a.tape.or(b.tape);
        let (ia, ib) = (if a.tape.is_some() { a.idx } else { NONE }, if b.tape.is_some() { b.idx } else { NONE });
        match tape {
            Some(t) if ia != NONE || ib != NONE => Rev { tape: Some(t), idx: t.push(ia, ib, da, db), v },
            _ => Rev { tape: None, idx: NONE, v },
        }
    }
}

impl<'t, T: Scalar> Add for Rev<'t, T> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::binary(self, o, self.v + o.v, T::cst(1.0), T::cst(1.0))
    }
}

impl<'t, T: Scalar> Sub for Rev<'t, T> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::binary(self, o, self.v - o.v, T::cst(1.0), T::cst(-1.0))
    }
}

impl<'t, T: Scalar> Mul for Rev<'t, T> {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::binary(self, o, self.v * o.v, o.v, self.v)
    }
}

impl<'t, T: Scalar> Div for Rev<'t, T> {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        let q = self.v / o.v;
        Self::binary(self, o, q, o.v.recip(), -(q / o.v))
    }
}

impl<'t, T: Scalar> Neg for Rev<'t, T> {
    type Output = Self;
    fn neg(self) -> Self {
        self.unary(-self.v, T::cst(-1.0))
    }
}

impl<'t, T: Scalar> Scalar for Rev<'t, T> {
    fn cst(v: f64) -> Self {
        Rev { tape: None, idx: NONE, v: T::cst(v) }
    }
    fn value(&self) -> f64 {
        self.v.value()
    }
    fn sqrt(self) -> Self {
        let s = self.v.sqrt();
        self.unary(s, T::cst(0.5) / s)
    }
    fn exp(self) -> Self {
        let e = self.v.exp();
        self.unary(e, e)
    }
    fn ln(self) -> Self {
        self.unary(self.v.ln(), self.v.recip())
    }
    fn sin(self) -> Self {
        self.unary(self.v.sin(), self.v.cos())
    }
    fn cos(self) -> Self {
        self.unary(self.v.cos(), -self.v.sin())
    }
    fn atan(self) -> Self {
        self.unary(self.v.atan(), (T::cst(1.0) + self.v * self.v).recip())
    }
    fn tanh(self) -> Self {
        let t = self.v.tanh();
        self.unary(t, T::cst(1.0) - t * t)
    }
    fn powi(self, n: i32) -> Self {
        if n == 0 {
            return Self::cst(1.0);
        }
        if self.tape.is_none() {
            return Rev { tape: None, idx: NONE, v: powi_by_mul(self.v, n) };
        }
        let low = powi_by_mul(self.v, n - 1);
        self.unary(low * self.v, T::cst(n as f64) * low)
    }
    fn powf(self, c: f64) -> Self {
        self.unary(self.v.powf(c), T::cst(c) * self.v.powf(c - 1.0))
    }
}
