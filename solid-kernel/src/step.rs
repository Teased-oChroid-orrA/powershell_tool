//! Bounded, native ISO 10303-21 entity inventory. This parses a documented physical
//! file subset, not geometric B-Rep validity or CAD import success. Encoded STEP
//! strings are retained without claiming Unicode escape/authoring-tool semantics.
//!
//! Supported: mandatory header records, unnamed DATA sections, simple/complex
//! entity calls, references, lists, i64 integers, finite f64 reals, quoted strings,
//! enums and omitted/derived values. Named DATA/external sections, binary literals,
//! encoded-string interpretation and schema/geometric validity are not implemented.
//! Unsupported syntax is refused; no entity is silently dropped.
pub use crate::step_topology::{
    EdgeUse, RawBody, RawBound, RawEdge, RawFace, RawVertex, TopologyLimits,
};
pub use crate::step_units::LengthContext;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i64),
    Real(f64),
    String(String),
    Enum(String),
    Ref(u64),
    List(Vec<Value>),
    Typed(Call),
    Omitted,
    Derived,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub name: String,
    pub args: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: u64,
    pub offset: usize,
    pub components: Vec<Call>,
}
#[derive(Debug, Clone)]
pub struct Inventory {
    header: Vec<Call>,
    entities: BTreeMap<u64, Entity>,
    counts: BTreeMap<String, usize>,
}
impl Inventory {
    /// Resolve only explicit assigned length units. Separate contexts are not
    /// silently unified; geometry must bind to its own representation context.
    pub fn length_contexts(&self) -> Result<Vec<LengthContext>, Error> {
        crate::step_units::contexts(self)
    }
    /// Raw body connectivity, not geometric validity or placed/normalized solids.
    pub fn raw_bodies(&self) -> Result<Vec<RawBody>, Error> {
        crate::step_topology::bodies(self, TopologyLimits::default())
    }
    pub fn raw_bodies_with_limits(&self, limits: TopologyLimits) -> Result<Vec<RawBody>, Error> {
        crate::step_topology::bodies(self, limits)
    }
    pub fn header(&self) -> &[Call] {
        &self.header
    }
    pub fn entities(&self) -> &BTreeMap<u64, Entity> {
        &self.entities
    }
    /// Component counts, including every component of a complex entity.
    pub fn counts(&self) -> &BTreeMap<String, usize> {
        &self.counts
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub entities: usize,
    pub values: usize,
    pub depth: usize,
    pub string_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 32 * 1024 * 1024,
            entities: 100_000,
            values: 1_000_000,
            depth: 32,
            string_bytes: 64 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Syntax,
    Limit,
    InvalidNumber,
    DuplicateEntity,
    DanglingReference,
    UnsupportedSection,
    Units,
    Topology,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub kind: ErrorKind,
    pub detail: String,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "STEP {:?} at byte {}: {}",
            self.kind, self.offset, self.detail
        )
    }
}
impl std::error::Error for Error {}
struct Parser<'a> {
    input: &'a str,
    at: usize,
    nodes: usize,
    limits: Limits,
}
impl Parser<'_> {
    fn error(&self, kind: ErrorKind, detail: impl Into<String>) -> Error {
        Error {
            offset: self.at,
            kind,
            detail: detail.into(),
        }
    }
    fn rest(&self) -> &[u8] {
        &self.input.as_bytes()[self.at..]
    }
    fn space(&mut self) -> Result<(), Error> {
        loop {
            while self.rest().first().is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            if self.rest().starts_with(b"/*") {
                let start = self.at;
                self.at += 2;
                while !self.rest().starts_with(b"*/") {
                    if self.rest().is_empty() {
                        return Err(Error {
                            offset: start,
                            kind: ErrorKind::Syntax,
                            detail: "unterminated comment".into(),
                        });
                    }
                    self.at += 1;
                }
                self.at += 2;
            } else {
                return Ok(());
            }
        }
    }
    fn byte(&mut self, want: u8) -> Result<(), Error> {
        self.space()?;
        if self.rest().first() != Some(&want) {
            return Err(self.error(ErrorKind::Syntax, format!("expected '{}'", want as char)));
        }
        self.at += 1;
        Ok(())
    }
    fn peek(&mut self, want: u8) -> Result<bool, Error> {
        self.space()?;
        Ok(self.rest().first() == Some(&want))
    }
    fn word_is(&mut self, want: &str) -> Result<bool, Error> {
        self.space()?;
        let bytes = want.as_bytes();
        Ok(self
            .rest()
            .get(..bytes.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(bytes))
            && !self
                .rest()
                .get(bytes.len())
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_'))
    }
    fn word(&mut self, want: &str) -> Result<(), Error> {
        if !self.word_is(want)? {
            return Err(self.error(ErrorKind::Syntax, format!("expected {want}")));
        }
        self.at += want.len();
        Ok(())
    }
    fn node(&mut self, depth: usize) -> Result<(), Error> {
        if depth > self.limits.depth || self.nodes >= self.limits.values {
            return Err(self.error(ErrorKind::Limit, "value/depth budget exceeded"));
        }
        self.nodes += 1;
        Ok(())
    }
    fn name(&mut self) -> Result<String, Error> {
        self.space()?;
        let start = self.at;
        if !self.rest().first().is_some_and(u8::is_ascii_alphabetic) {
            return Err(self.error(ErrorKind::Syntax, "expected entity/type name"));
        }
        while self
            .rest()
            .first()
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.at += 1;
        }
        if self.at - start > 256 {
            return Err(self.error(ErrorKind::Limit, "identifier exceeds 256 bytes"));
        }
        Ok(self.input[start..self.at].to_ascii_uppercase())
    }
    fn reference(&mut self) -> Result<u64, Error> {
        self.byte(b'#')?;
        let start = self.at;
        while self.rest().first().is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        let id = self.input[start..self.at]
            .parse::<u64>()
            .map_err(|_| self.error(ErrorKind::Syntax, "invalid entity reference"))?;
        if id == 0 {
            return Err(self.error(ErrorKind::Syntax, "entity identifiers must be positive"));
        }
        Ok(id)
    }
    fn args(&mut self, depth: usize) -> Result<Vec<Value>, Error> {
        self.byte(b'(')?;
        let mut args = Vec::new();
        if self.peek(b')')? {
            self.at += 1;
            return Ok(args);
        }
        loop {
            args.push(self.value(depth)?);
            if self.peek(b')')? {
                self.at += 1;
                return Ok(args);
            }
            self.byte(b',')?;
        }
    }
    fn call(&mut self, depth: usize) -> Result<Call, Error> {
        self.node(depth)?;
        let name = self.name()?;
        let args = self.args(depth + 1)?;
        Ok(Call { name, args })
    }
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        self.node(depth)?;
        self.space()?;
        let b = *self
            .rest()
            .first()
            .ok_or_else(|| self.error(ErrorKind::Syntax, "expected value"))?;
        match b {
            b'$' => {
                self.at += 1;
                Ok(Value::Omitted)
            }
            b'*' => {
                self.at += 1;
                Ok(Value::Derived)
            }
            b'#' => Ok(Value::Ref(self.reference()?)),
            b'(' => Ok(Value::List(self.args(depth + 1)?)),
            b'\'' => {
                self.at += 1;
                let start = self.at;
                loop {
                    if self.at - start > self.limits.string_bytes {
                        return Err(self.error(ErrorKind::Limit, "string budget exceeded"));
                    }
                    match self.rest().first() {
                        None => return Err(self.error(ErrorKind::Syntax, "unterminated string")),
                        Some(b'\'') if self.rest().get(1) == Some(&b'\'') => {
                            self.at += 2;
                        }
                        Some(b'\'') => {
                            let s = self.input[start..self.at].replace("''", "'");
                            self.at += 1;
                            return Ok(Value::String(s));
                        }
                        _ => self.at += 1,
                    }
                }
            }
            b'.' if self.rest().get(1).is_some_and(u8::is_ascii_alphabetic) => {
                self.at += 1;
                let name = self.name()?;
                self.byte(b'.')?;
                Ok(Value::Enum(name))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.at;
                while self.rest().first().is_some_and(|b| {
                    b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'E' | b'e')
                }) {
                    self.at += 1;
                }
                let literal = &self.input[start..self.at];
                if !literal.bytes().any(|b| matches!(b, b'.' | b'E' | b'e')) {
                    return literal.parse::<i64>().map(Value::Integer).map_err(|_| {
                        self.error(
                            ErrorKind::InvalidNumber,
                            "integer outside the supported exact range",
                        )
                    });
                }
                let n = literal.parse::<f64>().map_err(|_| {
                    self.error(ErrorKind::InvalidNumber, "invalid STEP numeric literal")
                })?;
                let nonzero_mantissa = literal
                    .split(['e', 'E'])
                    .next()
                    .unwrap_or("")
                    .bytes()
                    .any(|b| matches!(b, b'1'..=b'9'));
                if !n.is_finite() || (n == 0. && nonzero_mantissa) {
                    return Err(
                        self.error(ErrorKind::InvalidNumber, "numeric overflow or underflow")
                    );
                }
                Ok(Value::Real(n))
            }
            b'A'..=b'Z' | b'a'..=b'z' => Ok(Value::Typed(self.call(depth + 1)?)),
            _ => Err(self.error(ErrorKind::Syntax, "unsupported value token")),
        }
    }
}
/// Parse the HEADER and one or more DATA sections. Unsupported physical-file
/// sections are explicit errors, never ignored. Forward references are resolved
/// after reading the whole file. Cycles are retained, not mistaken for geometry validation.
pub fn inventory(text: &str, limits: Limits) -> Result<Inventory, Error> {
    if text.len() > limits.bytes {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::Limit,
            detail: "file byte budget exceeded".into(),
        });
    }
    let mut p = Parser {
        input: text,
        at: if text.starts_with('\u{feff}') { 3 } else { 0 },
        nodes: 0,
        limits,
    };
    p.word("ISO-10303-21")?;
    p.byte(b';')?;
    p.word("HEADER")?;
    p.byte(b';')?;
    let mut header = Vec::new();
    while !p.word_is("ENDSEC")? {
        header.push(p.call(0)?);
        p.byte(b';')?;
    }
    p.word("ENDSEC")?;
    p.byte(b';')?;
    for (index, (name, arity)) in [
        ("FILE_DESCRIPTION", 2),
        ("FILE_NAME", 7),
        ("FILE_SCHEMA", 1),
    ]
    .into_iter()
    .enumerate()
    {
        if header
            .get(index)
            .is_none_or(|call| call.name != name || call.args.len() != arity)
            || header.iter().filter(|call| call.name == name).count() != 1
        {
            return Err(p.error(
                ErrorKind::Syntax,
                format!("missing, duplicated or malformed required {name} header"),
            ));
        }
    }
    let mut entities = BTreeMap::new();
    let mut counts = BTreeMap::new();
    let mut sections = 0;
    while !p.word_is("END-ISO-10303-21")? {
        if !p.word_is("DATA")? {
            return Err(p.error(
                ErrorKind::UnsupportedSection,
                "only HEADER/DATA sections are supported",
            ));
        }
        p.word("DATA")?;
        // Named data sections are outside the supported subset until their
        // cross-section scope semantics are implemented; do not discard metadata.
        if p.peek(b'(')? {
            return Err(p.error(
                ErrorKind::UnsupportedSection,
                "named DATA section semantics are not implemented",
            ));
        }
        p.byte(b';')?;
        sections += 1;
        while !p.word_is("ENDSEC")? {
            if entities.len() >= limits.entities {
                return Err(p.error(ErrorKind::Limit, "entity budget exceeded"));
            }
            let offset = p.at;
            let id = p.reference()?;
            p.byte(b'=')?;
            let mut components = Vec::new();
            if p.peek(b'(')? {
                p.at += 1;
                while !p.peek(b')')? {
                    components.push(p.call(0)?);
                }
                p.at += 1;
                if components.is_empty() {
                    return Err(p.error(ErrorKind::Syntax, "empty complex entity"));
                }
            } else {
                components.push(p.call(0)?);
            }
            p.byte(b';')?;
            for call in &components {
                *counts.entry(call.name.clone()).or_insert(0) += 1;
            }
            if entities
                .insert(
                    id,
                    Entity {
                        id,
                        offset,
                        components,
                    },
                )
                .is_some()
            {
                return Err(Error {
                    offset,
                    kind: ErrorKind::DuplicateEntity,
                    detail: format!("duplicate entity #{id}"),
                });
            }
        }
        p.word("ENDSEC")?;
        p.byte(b';')?;
    }
    if sections == 0 {
        return Err(p.error(ErrorKind::Syntax, "missing DATA section"));
    }
    p.word("END-ISO-10303-21")?;
    p.byte(b';')?;
    p.space()?;
    if p.at != text.len() {
        return Err(p.error(ErrorKind::Syntax, "trailing data after file terminator"));
    }
    fn references(value: &Value, entities: &BTreeMap<u64, Entity>) -> Result<(), u64> {
        match value {
            Value::Ref(id) if !entities.contains_key(id) => Err(*id),
            Value::List(values) => {
                for value in values {
                    references(value, entities)?;
                }
                Ok(())
            }
            Value::Typed(call) => {
                for value in &call.args {
                    references(value, entities)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    for call in &header {
        for value in &call.args {
            if let Err(id) = references(value, &entities) {
                return Err(Error {
                    offset: 0,
                    kind: ErrorKind::DanglingReference,
                    detail: format!("header {} references missing #{id}", call.name),
                });
            }
        }
    }
    for entity in entities.values() {
        for call in &entity.components {
            for value in &call.args {
                if let Err(id) = references(value, &entities) {
                    return Err(Error {
                        offset: entity.offset,
                        kind: ErrorKind::DanglingReference,
                        detail: format!("entity #{} references missing #{id}", entity.id),
                    });
                }
            }
        }
    }
    Ok(Inventory {
        header,
        entities,
        counts,
    })
}
