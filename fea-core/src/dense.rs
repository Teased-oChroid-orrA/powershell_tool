//! Small dense symmetric eigenproblems (row-major `n x n`): the projected problems of the iterative eigensolvers
//! (`eigen.rs`) and the oracle of the conditioning tests. Cyclic Jacobi: unconditionally accurate (small relative
//! error in every eigenvalue of a positive definite matrix), O(n^3) per sweep, meant for n up to a few hundred.

/// Eigenvalues (ascending) and eigenvectors of the symmetric matrix `a`. `vec[k * n + i]` is component `i` of the
/// unit eigenvector of `val[k]`.
pub fn sym_eigen(a: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    assert_eq!(a.len(), n * n);
    let mut a = a.to_vec();
    let mut v = vec![0.0; n * n]; // columns accumulate rotations; stored row-major, column k = eigenvector k
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _sweep in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..i).map(move |j| (i, j))).map(|(i, j)| a[i * n + j] * a[i * n + j]).sum();
        let diag: f64 = (0..n).map(|i| a[i * n + i] * a[i * n + i]).sum();
        if off <= 1e-30 * diag.max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq == 0.0 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| a[i * n + i].total_cmp(&a[j * n + j]));
    let val: Vec<f64> = order.iter().map(|&i| a[i * n + i]).collect();
    let mut vec = vec![0.0; n * n];
    for (k, &col) in order.iter().enumerate() {
        for i in 0..n {
            vec[k * n + i] = v[i * n + col];
        }
    }
    (val, vec)
}

/// Solve the generalised problem `a x = lambda b x` (`b` symmetric positive definite) through the Cholesky factor
/// of `b`: eigenvalues ascending, `b`-orthonormal eigenvectors (`vec[k * n + i]`).
pub fn sym_gen_eigen(a: &[f64], b: &[f64], n: usize) -> Result<(Vec<f64>, Vec<f64>), String> {
    // b = L L^T
    let mut l = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = (0..j).map(|k| l[i * n + k] * l[j * n + k]).sum();
            if i == j {
                let d = b[i * n + i] - s;
                if d <= 0.0 {
                    return Err("the mass-like matrix is not positive definite".into());
                }
                l[i * n + i] = d.sqrt();
            } else {
                l[i * n + j] = (b[i * n + j] - s) / l[j * n + j];
            }
        }
    }
    // c = L^-1 a L^-T
    let mut y = a.to_vec(); // y = L^-1 a
    for col in 0..n {
        for i in 0..n {
            let s: f64 = (0..i).map(|k| l[i * n + k] * y[k * n + col]).sum();
            y[i * n + col] = (y[i * n + col] - s) / l[i * n + i];
        }
    }
    let mut c = vec![0.0; n * n]; // c = y L^-T
    for row in 0..n {
        for j in 0..n {
            let s: f64 = (0..j).map(|k| l[j * n + k] * c[row * n + k]).sum();
            c[row * n + j] = (y[row * n + j] - s) / l[j * n + j];
        }
    }
    for i in 0..n {
        for j in 0..i {
            let m = 0.5 * (c[i * n + j] + c[j * n + i]);
            c[i * n + j] = m;
            c[j * n + i] = m;
        }
    }
    let (val, z) = sym_eigen(&c, n);
    // x = L^-T z
    let mut vec = vec![0.0; n * n];
    for k in 0..n {
        for i in (0..n).rev() {
            let s: f64 = (i + 1..n).map(|m| l[m * n + i] * vec[k * n + m]).sum();
            vec[k * n + i] = (z[k * n + i] - s) / l[i * n + i];
        }
    }
    Ok((val, vec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eigenvalues_of_the_1d_laplacian_are_known() {
        let n = 12;
        let mut a = vec![0.0; n * n];
        for i in 0..n {
            a[i * n + i] = 2.0;
            if i + 1 < n {
                a[i * n + i + 1] = -1.0;
                a[(i + 1) * n + i] = -1.0;
            }
        }
        let (val, vec) = sym_eigen(&a, n);
        for (k, v) in val.iter().enumerate() {
            let exact = 2.0 - 2.0 * ((k + 1) as f64 * std::f64::consts::PI / (n + 1) as f64).cos();
            assert!((v - exact).abs() < 1e-12, "{k}: {v} vs {exact}");
            // A v = lambda v
            for i in 0..n {
                let av: f64 = (0..n).map(|j| a[i * n + j] * vec[k * n + j]).sum();
                assert!((av - v * vec[k * n + i]).abs() < 1e-11);
            }
        }
    }

    #[test]
    fn generalised_problem_with_a_diagonal_mass_scales_the_eigenvalues() {
        let n = 5;
        let mut a = vec![0.0; n * n];
        let mut b = vec![0.0; n * n];
        for i in 0..n {
            a[i * n + i] = 2.0;
            if i + 1 < n {
                a[i * n + i + 1] = -1.0;
                a[(i + 1) * n + i] = -1.0;
            }
            b[i * n + i] = 4.0;
        }
        let (val, vec) = sym_gen_eigen(&a, &b, n).unwrap();
        let (plain, _) = sym_eigen(&a, n);
        for k in 0..n {
            assert!((val[k] - plain[k] / 4.0).abs() < 1e-12);
            // b-orthonormal
            let norm: f64 = (0..n).map(|i| 4.0 * vec[k * n + i] * vec[k * n + i]).sum();
            assert!((norm - 1.0).abs() < 1e-12);
        }
    }
}
