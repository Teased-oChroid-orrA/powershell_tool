//! Shared stable native geometry direction normalization.
use crate::V3;
pub(crate) fn normalize_direction(v: V3) -> Result<V3, &'static str> {
    let m = v.iter().map(|x| x.abs()).fold(0., f64::max);
    if v.iter().any(|x| !x.is_finite()) || m == 0. {
        return Err("zero or non-finite direction");
    }
    let scaled = v.map(|x| x / m);
    if v.iter()
        .zip(scaled)
        .any(|(before, after)| *before != 0. && after == 0.)
    {
        return Err("direction normalization underflows a nonzero component");
    }
    let v = scaled;
    let n = v[0].hypot(v[1]).hypot(v[2]);
    if !n.is_finite() || n == 0. {
        return Err("unresolved direction");
    }
    let inverse = 1. / n;
    Ok(v.map(|x| x * inverse))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrepresentable_normalized_direction_components_are_explicit_errors() {
        assert_eq!(normalize_direction([1e-300, 0., 0.]).unwrap(), [1., 0., 0.]);
        assert_eq!(normalize_direction([1e300, 0., 0.]).unwrap(), [1., 0., 0.]);
        let error = normalize_direction([1e-300, 1e300, 0.]).unwrap_err();
        assert!(error.contains("underflow"));
    }
}
