//! Integer oversampling with linear-phase Kaiser FIR filters.
//!
//! Same design as `harness/src/oc_harness/spice.py`, so SPICE references and models are band-limited
//! identically: flat to 20 kHz, 120 dB down from fs - 20 kHz.

/// Circuit models run at no less than this rate. Trapezoidal integration evaluates the analog
/// response at a frequency stretched by about (pi f / rate)^2 / 3, so the error at 20 kHz grows with
/// how steeply the response falls there. 384 kHz measured 0.12 dB on the TS808 at full treble;
/// 576 kHz measures 0.05 dB.
pub const MIN_CIRCUIT_RATE: f64 = 576_000.0;
const PASSBAND_HZ: f64 = 20_000.0;
const ATTENUATION_DB: f64 = 120.0;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..64 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Oversampling factor that brings `base_rate` up to at least `MIN_CIRCUIT_RATE`.
pub fn factor_for(base_rate: f64) -> usize {
    (MIN_CIRCUIT_RATE / base_rate).ceil().max(1.0) as usize
}

/// Low-pass at the base Nyquist frequency, normalized to unity gain at DC (as scipy's `firwin`).
/// Length is rounded up to `factor * k + 1` taps so the round trip delay is a whole base sample count.
fn design(base_rate: f64, factor: usize) -> Vec<f64> {
    let rate = base_rate * factor as f64;
    let width = 2.0 * std::f64::consts::PI * (base_rate - 2.0 * PASSBAND_HZ) / rate;
    let mut taps = ((ATTENUATION_DB - 7.95) / (2.285 * width)).ceil() as usize + 1;
    taps += (factor - (taps - 1) % factor) % factor;
    let beta = 0.1102 * (ATTENUATION_DB - 8.7);
    let cutoff = base_rate / 2.0 / (rate / 2.0);
    let mid = (taps - 1) as f64 / 2.0;
    let mut h: Vec<f64> = (0..taps)
        .map(|i| {
            let t = i as f64 - mid;
            let sinc = if t == 0.0 {
                1.0
            } else {
                (std::f64::consts::PI * cutoff * t).sin() / (std::f64::consts::PI * cutoff * t)
            };
            let r = 2.0 * i as f64 / (taps - 1) as f64 - 1.0;
            cutoff * sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.iter_mut().for_each(|x| *x /= sum);
    h
}

/// Ring buffer stored twice in a row, so the newest `len` samples are always one contiguous slice.
struct History {
    buf: Vec<f64>,
    pos: usize,
}

impl History {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; 2 * len],
            pos: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f64) {
        let len = self.buf.len() / 2;
        self.pos = if self.pos == 0 { len - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + len] = x;
    }

    /// Newest sample first.
    #[inline]
    fn recent(&self) -> &[f64] {
        &self.buf[self.pos..self.pos + self.buf.len() / 2]
    }
}

pub struct Oversampler {
    factor: usize,
    /// Polyphase upsampling filters, scaled by the factor and zero-padded to equal length.
    phases: Vec<Vec<f64>>,
    h: Vec<f64>,
    up: History,
    down: History,
}

impl Oversampler {
    pub fn new(base_rate: f64, factor: usize) -> Self {
        let h = design(base_rate, factor);
        let phase_len = h.len().div_ceil(factor);
        let phases = (0..factor)
            .map(|p| {
                (0..phase_len)
                    .map(|j| h.get(j * factor + p).map_or(0.0, |c| c * factor as f64))
                    .collect()
            })
            .collect();
        Self {
            factor,
            phases,
            up: History::new(phase_len),
            down: History::new(h.len()),
            h,
        }
    }

    /// Round-trip delay in base-rate samples.
    pub fn latency(&self) -> usize {
        (self.h.len() - 1) / self.factor
    }

    /// Upsamples `x` into `factor` samples, runs `f` on each, and returns the decimated result.
    #[inline]
    pub fn process(&mut self, x: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
        self.up.push(x);
        let mut out = 0.0;
        for (phase, coeffs) in self.phases.iter().enumerate() {
            let y = dot(coeffs, self.up.recent());
            self.down.push(f(y));
            // Keep every factor-th sample, aligned with phase 0 so the round trip delay is whole.
            if phase == 0 {
                out = dot(&self.h, self.down.recent());
            }
        }
        out
    }
}

#[inline]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut acc = [0.0; 4];
    let (ac, bc) = (a.chunks_exact(4), b.chunks_exact(4));
    let tail: f64 = ac.remainder().iter().zip(bc.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ac.zip(bc) {
        for j in 0..4 {
            acc[j] += x[j] * y[j];
        }
    }
    acc.iter().sum::<f64>() + tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn round_trip_is_flat_to_20k_and_delayed_by_latency() {
        for base in [44_100.0, 48_000.0, 96_000.0] {
            for freq in [20.0, 1000.0, 20_000.0] {
                let mut os = Oversampler::new(base, factor_for(base));
                let lat = os.latency();
                let n = (base * 0.2) as usize;
                let x = |i: usize| (2.0 * PI * freq * i as f64 / base).sin();
                let (mut err, mut amp) = (0.0f64, 0.0f64);
                for i in 0..n {
                    let y = os.process(x(i), |v| v);
                    if i > n / 2 {
                        err = err.max((y - x(i - lat)).abs());
                        amp = amp.max(y.abs());
                    }
                }
                assert!(err < 1e-4, "{base} Hz base, {freq} Hz: error {err}");
                assert!((20.0 * amp.log10()).abs() < 0.001, "{base}/{freq}: {amp}");
            }
        }
    }

    #[test]
    fn matches_harness_tap_counts() {
        // Same lengths as oc_harness.spice.oversampling_filter (4x).
        assert_eq!(design(44_100.0, 4).len(), 337);
        assert_eq!(design(48_000.0, 4).len(), 189);
        assert_eq!(design(96_000.0, 4).len(), 57);
        assert_eq!(
            [44_100.0, 48_000.0, 96_000.0, 192_000.0].map(factor_for),
            [14, 12, 6, 3]
        );
    }
}
