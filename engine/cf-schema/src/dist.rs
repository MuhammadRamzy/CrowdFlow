//! Probability distributions used by scenario authoring and by the engine.
//!
//! # Why inverse-CDF sampling
//!
//! [`Distribution::sample_icdf`] maps a uniform `u ∈ [0,1)` to a variate. It
//! takes no RNG. That buys three things at once:
//!
//! 1. **Determinism.** The engine owns the PRNG stream; this module is pure. A
//!    given `u` always yields the same value on every target, which is what
//!    CI gate G2 checks (docs/04-track-b-simulation-engine.md §5).
//! 2. **One definition of "lognormal".** The authoring UI previewing a histogram
//!    and the engine spawning agents run the *same* function, so they cannot
//!    disagree about what a parameter means.
//! 3. **Portability.** The same algorithm ports to TypeScript in ~40 lines,
//!    so the browser-side preview matches the Rust engine exactly.
//!
//! # Float determinism
//!
//! `ln`/`exp` are not bit-identical across libm implementations, so every
//! transcendental call here goes through the private [`fmath`] module. Swapping
//! the whole crate onto the pure-Rust `libm` crate is then a one-file change.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How far `sample_icdf` keeps its input away from the open interval's ends.
const U_EPS: f64 = 1e-12;

/// Transcendental functions, funnelled through one place so they can be swapped
/// for a bit-reproducible implementation without touching call sites.
mod fmath {
    #[inline]
    pub fn ln(x: f64) -> f64 {
        x.ln()
    }
    #[inline]
    pub fn exp(x: f64) -> f64 {
        x.exp()
    }
    #[inline]
    pub fn sqrt(x: f64) -> f64 {
        // IEEE-754 exact on every target; safe to use directly.
        x.sqrt()
    }
}

/// A sampleable distribution.
///
/// Serialised as an internally-tagged union on the `dist` field, e.g.
/// `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "dist", rename_all = "camelCase")]
pub enum Distribution {
    /// A fixed value. Useful for pinning a parameter during sensitivity analysis.
    Constant {
        value: f64,
    },

    Uniform {
        min: f64,
        max: f64,
    },

    Normal {
        mean: f64,
        sd: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },

    /// Parameterised by the mean and sd **of the underlying normal**, not of the
    /// resulting variate. Named `muLn`/`sigmaLn` rather than `mu`/`sigma` so the
    /// distinction is impossible to miss at a call site — getting this wrong is
    /// the single most common modelling error with lognormals.
    ///
    /// Note: `rename_all` on the enum renames *variants*, not their fields, so
    /// multi-word fields need their own attribute.
    #[serde(rename_all = "camelCase")]
    Lognormal {
        mu_ln: f64,
        sigma_ln: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },

    Exponential {
        lambda: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },

    /// Discrete outcomes with weights. Weights need not sum to 1; they are
    /// normalised. Keys are strings so that group sizes (`"1"`, `"2"`, …) and
    /// named categories share one representation.
    Categorical {
        p: BTreeMap<String, f64>,
    },

    /// Empirical distribution over observed samples, sampled by linear
    /// interpolation of the order statistics.
    Empirical {
        samples: Vec<f64>,
    },
}

impl Distribution {
    pub fn constant(value: f64) -> Self {
        Distribution::Constant { value }
    }

    /// Map a uniform `u ∈ [0,1)` to a variate.
    ///
    /// For [`Distribution::Categorical`] this returns the *index* of the chosen
    /// key in sorted-key order; use [`Distribution::sample_category`] to get the
    /// key itself.
    pub fn sample_icdf(&self, u: f64) -> f64 {
        // Clamp away from *both* endpoints, not just 1.0. An unbounded normal's
        // quantile function is -inf at exactly 0.0, and uniform PRNGs routinely
        // emit 0.0 — which would silently inject NaN agents into the sim.
        // 1e-12 corresponds to roughly ±7 sigma, far outside any real cohort.
        let u = u.clamp(U_EPS, 1.0 - U_EPS);
        match self {
            Distribution::Constant { value } => *value,

            Distribution::Uniform { min, max } => min + (max - min) * u,

            Distribution::Normal { mean, sd, min, max } => {
                let v = mean + sd * normal_icdf(u);
                clamp_opt(v, *min, *max)
            }

            Distribution::Lognormal {
                mu_ln,
                sigma_ln,
                min,
                max,
            } => {
                let v = fmath::exp(mu_ln + sigma_ln * normal_icdf(u));
                clamp_opt(v, *min, *max)
            }

            Distribution::Exponential { lambda, max } => {
                let v = if *lambda > 0.0 {
                    -fmath::ln(1.0 - u) / lambda
                } else {
                    0.0
                };
                clamp_opt(v, None, *max)
            }

            Distribution::Categorical { p } => {
                let total: f64 = p.values().filter(|w| **w > 0.0).sum();
                if total <= 0.0 {
                    return 0.0;
                }
                let mut acc = 0.0;
                for (i, w) in p.values().enumerate() {
                    if *w <= 0.0 {
                        continue;
                    }
                    acc += w / total;
                    if u < acc {
                        return i as f64;
                    }
                }
                (p.len().saturating_sub(1)) as f64
            }

            Distribution::Empirical { samples } => {
                if samples.is_empty() {
                    return 0.0;
                }
                let mut sorted = samples.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                if sorted.len() == 1 {
                    return sorted[0];
                }
                let pos = u * (sorted.len() - 1) as f64;
                let i = pos.floor() as usize;
                let frac = pos - i as f64;
                let j = (i + 1).min(sorted.len() - 1);
                sorted[i] + (sorted[j] - sorted[i]) * frac
            }
        }
    }

    /// Sample a categorical key. Returns `None` for non-categorical distributions.
    pub fn sample_category(&self, u: f64) -> Option<&str> {
        match self {
            Distribution::Categorical { p } => {
                let idx = self.sample_icdf(u) as usize;
                p.keys().nth(idx).map(|s| s.as_str())
            }
            _ => None,
        }
    }

    /// Analytic mean where one exists, otherwise the empirical mean.
    /// Used for the authoring UI's summary readout and for smoke tests.
    pub fn mean(&self) -> f64 {
        match self {
            Distribution::Constant { value } => *value,
            Distribution::Uniform { min, max } => (min + max) * 0.5,
            Distribution::Normal { mean, .. } => *mean,
            Distribution::Lognormal {
                mu_ln, sigma_ln, ..
            } => fmath::exp(mu_ln + sigma_ln * sigma_ln * 0.5),
            Distribution::Exponential { lambda, .. } => {
                if *lambda > 0.0 {
                    1.0 / lambda
                } else {
                    0.0
                }
            }
            Distribution::Categorical { p } => {
                let total: f64 = p.values().sum();
                if total <= 0.0 {
                    return 0.0;
                }
                p.iter()
                    .filter_map(|(k, w)| k.parse::<f64>().ok().map(|v| v * w / total))
                    .sum()
            }
            Distribution::Empirical { samples } => {
                if samples.is_empty() {
                    0.0
                } else {
                    samples.iter().sum::<f64>() / samples.len() as f64
                }
            }
        }
    }

    /// Structural validity. Returns a human-readable reason when the parameters
    /// cannot produce samples.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Distribution::Constant { value } => finite("value", *value),
            Distribution::Uniform { min, max } => {
                finite("min", *min)?;
                finite("max", *max)?;
                if max < min {
                    return Err(format!("uniform: max ({max}) < min ({min})"));
                }
                Ok(())
            }
            Distribution::Normal { mean, sd, min, max } => {
                finite("mean", *mean)?;
                finite("sd", *sd)?;
                if *sd < 0.0 {
                    return Err(format!("normal: sd ({sd}) must be >= 0"));
                }
                check_bounds(*min, *max)
            }
            Distribution::Lognormal {
                mu_ln,
                sigma_ln,
                min,
                max,
            } => {
                finite("muLn", *mu_ln)?;
                finite("sigmaLn", *sigma_ln)?;
                if *sigma_ln < 0.0 {
                    return Err(format!("lognormal: sigmaLn ({sigma_ln}) must be >= 0"));
                }
                check_bounds(*min, *max)
            }
            Distribution::Exponential { lambda, .. } => {
                finite("lambda", *lambda)?;
                if *lambda <= 0.0 {
                    return Err(format!("exponential: lambda ({lambda}) must be > 0"));
                }
                Ok(())
            }
            Distribution::Categorical { p } => {
                if p.is_empty() {
                    return Err("categorical: no outcomes".into());
                }
                if p.values().any(|w| !w.is_finite() || *w < 0.0) {
                    return Err("categorical: weights must be finite and >= 0".into());
                }
                if p.values().sum::<f64>() <= 0.0 {
                    return Err("categorical: weights sum to 0".into());
                }
                Ok(())
            }
            Distribution::Empirical { samples } => {
                if samples.is_empty() {
                    return Err("empirical: no samples".into());
                }
                if samples.iter().any(|v| !v.is_finite()) {
                    return Err("empirical: samples must be finite".into());
                }
                Ok(())
            }
        }
    }
}

fn finite(name: &str, v: f64) -> Result<(), String> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(format!("{name} must be finite (got {v})"))
    }
}

fn check_bounds(min: Option<f64>, max: Option<f64>) -> Result<(), String> {
    if let (Some(lo), Some(hi)) = (min, max) {
        if hi < lo {
            return Err(format!("max ({hi}) < min ({lo})"));
        }
    }
    Ok(())
}

fn clamp_opt(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let v = match min {
        Some(lo) => v.max(lo),
        None => v,
    };
    match max {
        Some(hi) => v.min(hi),
        None => v,
    }
}

/// Inverse CDF of the standard normal, via Acklam's rational approximation.
///
/// Relative error < 1.15e-9 over the open interval, which is far below any
/// modelling uncertainty in walking-speed data. Chosen over a Box–Muller
/// transform because it consumes exactly one uniform, which keeps the PRNG
/// stream aligned regardless of which distribution a population uses — a
/// prerequisite for reproducing a run after editing an unrelated parameter.
// Coefficients are transcribed verbatim from the published algorithm so they can
// be checked against the source. Some carry a digit more than f64 can represent;
// truncating them to satisfy the lint would make that check impossible.
#[allow(clippy::excessive_precision)]
pub fn normal_icdf(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    const P_LOW: f64 = 0.02425;
    const P_HIGH: f64 = 1.0 - P_LOW;

    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }

    if p < P_LOW {
        let q = fmath::sqrt(-2.0 * fmath::ln(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= P_HIGH {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = fmath::sqrt(-2.0 * fmath::ln(1.0 - p));
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_icdf_known_quantiles() {
        assert!((normal_icdf(0.5)).abs() < 1e-9);
        assert!((normal_icdf(0.975) - 1.959963985).abs() < 1e-6);
        assert!((normal_icdf(0.025) + 1.959963985).abs() < 1e-6);
        assert!((normal_icdf(0.8413447461) - 1.0).abs() < 1e-6);
        // Tail branches (p < 0.02425 and p > 0.97575) use a different rational form.
        assert!((normal_icdf(0.001) + 3.090232306).abs() < 1e-5);
        assert!((normal_icdf(0.999) - 3.090232306).abs() < 1e-5);
    }

    #[test]
    fn normal_icdf_is_symmetric() {
        for i in 1..1000 {
            let p = i as f64 / 1000.0;
            let a = normal_icdf(p);
            let b = normal_icdf(1.0 - p);
            assert!((a + b).abs() < 1e-6, "asymmetry at p={p}: {a} vs {b}");
        }
    }

    #[test]
    fn icdf_is_monotonic_for_every_variant() {
        let dists = vec![
            Distribution::Constant { value: 3.0 },
            Distribution::Uniform { min: 1.0, max: 2.0 },
            Distribution::Normal {
                mean: 1.34,
                sd: 0.26,
                min: None,
                max: None,
            },
            Distribution::Lognormal {
                mu_ln: 0.1,
                sigma_ln: 0.35,
                min: None,
                max: None,
            },
            Distribution::Exponential {
                lambda: 0.004,
                max: None,
            },
            Distribution::Empirical {
                samples: vec![3.0, 1.0, 2.0, 5.0],
            },
        ];
        for d in dists {
            let mut prev = f64::NEG_INFINITY;
            for i in 0..=100 {
                let v = d.sample_icdf(i as f64 / 100.0);
                assert!(v.is_finite(), "{d:?} produced non-finite at u={i}");
                assert!(v >= prev - 1e-12, "{d:?} not monotonic at u={i}");
                prev = v;
            }
            // The exact endpoints are what a uniform PRNG will eventually hand us.
            for u in [0.0, 1.0, -0.0] {
                assert!(
                    d.sample_icdf(u).is_finite(),
                    "{d:?} produced non-finite at the endpoint u={u}"
                );
            }
        }
    }

    #[test]
    fn normal_samples_recover_their_mean() {
        let d = Distribution::Normal {
            mean: 1.34,
            sd: 0.26,
            min: None,
            max: None,
        };
        let n = 10_000;
        // Midpoint rule over the quantile function integrates to the mean.
        let sum: f64 = (0..n)
            .map(|i| d.sample_icdf((i as f64 + 0.5) / n as f64))
            .sum();
        assert!((sum / n as f64 - 1.34).abs() < 1e-3);
    }

    #[test]
    fn bounds_are_respected() {
        let d = Distribution::Normal {
            mean: 1.34,
            sd: 0.5,
            min: Some(0.6),
            max: Some(2.2),
        };
        for i in 0..=1000 {
            let v = d.sample_icdf(i as f64 / 1000.0);
            assert!((0.6..=2.2).contains(&v), "out of bounds: {v}");
        }
    }

    #[test]
    fn categorical_respects_weights() {
        let mut p = BTreeMap::new();
        p.insert("1".to_string(), 0.45);
        p.insert("2".to_string(), 0.30);
        p.insert("3".to_string(), 0.15);
        p.insert("4".to_string(), 0.10);
        let d = Distribution::Categorical { p };

        assert_eq!(d.sample_category(0.0), Some("1"));
        assert_eq!(d.sample_category(0.44), Some("1"));
        assert_eq!(d.sample_category(0.50), Some("2"));
        assert_eq!(d.sample_category(0.80), Some("3"));
        assert_eq!(d.sample_category(0.95), Some("4"));
        // Mean group size: 1(.45)+2(.30)+3(.15)+4(.10) = 1.90
        assert!((d.mean() - 1.90).abs() < 1e-9);
    }

    #[test]
    fn lognormal_mean_matches_closed_form() {
        let d = Distribution::Lognormal {
            mu_ln: 0.10,
            sigma_ln: 0.35,
            min: None,
            max: None,
        };
        let n = 200_000;
        let sum: f64 = (0..n)
            .map(|i| d.sample_icdf((i as f64 + 0.5) / n as f64))
            .sum();
        assert!((sum / n as f64 - d.mean()).abs() < 1e-2);
    }

    #[test]
    fn validation_rejects_nonsense() {
        assert!(Distribution::Uniform { min: 5.0, max: 1.0 }
            .validate()
            .is_err());
        assert!(Distribution::Exponential {
            lambda: 0.0,
            max: None
        }
        .validate()
        .is_err());
        assert!(Distribution::Empirical { samples: vec![] }
            .validate()
            .is_err());
        assert!(Distribution::Normal {
            mean: 1.0,
            sd: -1.0,
            min: None,
            max: None
        }
        .validate()
        .is_err());
        assert!(Distribution::Normal {
            mean: 1.34,
            sd: 0.26,
            min: None,
            max: None
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn wire_format_is_tagged_on_dist() {
        let d = Distribution::Normal {
            mean: 1.34,
            sd: 0.26,
            min: Some(0.6),
            max: Some(2.2),
        };
        let j = serde_json::to_string(&d).unwrap();
        assert_eq!(
            j,
            r#"{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}"#
        );
        assert_eq!(serde_json::from_str::<Distribution>(&j).unwrap(), d);

        // Optional bounds are omitted rather than emitted as null.
        let d2 = Distribution::Exponential {
            lambda: 0.004,
            max: None,
        };
        assert_eq!(
            serde_json::to_string(&d2).unwrap(),
            r#"{"dist":"exponential","lambda":0.004}"#
        );
    }
}
