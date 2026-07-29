//! Transcendental functions, funnelled through one module.
//!
//! `exp`, `ln`, `sin` and friends are **not** bit-identical across libm
//! implementations. The same expression can differ in the last bit between
//! glibc on x86-64, musl on aarch64, and whatever the wasm toolchain links —
//! which breaks the guarantee that a browser preview and a server report agree.
//!
//! Every such call in `cf-sim` goes through here, so moving the whole crate
//! onto the pure-Rust `libm` crate is a one-file change rather than an audit.
//!
//! `sqrt` is exempt: IEEE-754 mandates it to be correctly rounded, so it is
//! exact on every target and can be called directly.

/// `e^x`. **Not yet bit-reproducible across targets** — see the module docs.
#[inline]
pub fn exp(x: f32) -> f32 {
    x.exp()
}

/// Natural logarithm. **Not yet bit-reproducible across targets.**
#[inline]
pub fn ln(x: f32) -> f32 {
    x.ln()
}

/// IEEE-754 mandates correct rounding, so this is exact everywhere.
#[inline]
pub fn sqrt(x: f32) -> f32 {
    x.sqrt()
}

/// Length of a 2D vector. Uses only multiply, add and sqrt, so it is exact.
#[inline]
pub fn hypot2(x: f32, y: f32) -> f32 {
    sqrt(x * x + y * y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot2_matches_pythagoras() {
        assert_eq!(hypot2(3.0, 4.0), 5.0);
        assert_eq!(hypot2(0.0, 0.0), 0.0);
        assert_eq!(hypot2(-3.0, -4.0), 5.0);
    }

    #[test]
    fn exp_and_ln_round_trip() {
        for x in [0.1f32, 1.0, 2.5, 10.0] {
            assert!((ln(exp(x)) - x).abs() < 1e-5, "round trip failed at {x}");
        }
    }
}
