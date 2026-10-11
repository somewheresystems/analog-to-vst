/// A front-panel control. Hosts and the renderer pass knob rotation from 0 (fully
/// counter-clockwise) to 1; `plain` converts it to a physical value (ohms, fraction of travel).
#[derive(Clone, Copy, Debug)]
pub struct ControlSpec {
    pub name: &'static str,
    pub taper: Taper,
    pub default: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum Taper {
    Linear {
        min: f64,
        max: f64,
    },
    /// Logarithmic, for frequencies and times. `min` must be positive.
    Log {
        min: f64,
        max: f64,
    },
    /// Audio (A, logarithmic) pot from 0 to `max`: 10% at mid-rotation, two linear segments.
    Audio {
        max: f64,
    },
    /// Detented switch: the rotation range is split evenly between the positions.
    Stepped(&'static [f64]),
}

impl ControlSpec {
    pub fn plain(&self, rotation: f32) -> f64 {
        let t = rotation.clamp(0.0, 1.0) as f64;
        match self.taper {
            Taper::Linear { min, max } => min + (max - min) * t,
            Taper::Log { min, max } => min * (max / min).powf(t),
            Taper::Audio { max } => max * if t < 0.5 { 0.2 * t } else { 0.1 + 1.8 * (t - 0.5) },
            Taper::Stepped(positions) => positions[((t * positions.len() as f64) as usize).min(positions.len() - 1)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tapers() {
        let spec = |taper| ControlSpec {
            name: "",
            taper,
            default: 0.0,
        };
        let lin = spec(Taper::Linear { min: -24.0, max: 24.0 });
        assert_eq!((lin.plain(0.0), lin.plain(0.5), lin.plain(1.0)), (-24.0, 0.0, 24.0));
        let log = spec(Taper::Log {
            min: 20.0,
            max: 20_000.0,
        });
        assert!((log.plain(0.5) - 632.456).abs() < 0.01);
        let audio = spec(Taper::Audio { max: 500e3 });
        assert_eq!(
            (audio.plain(0.0), audio.plain(0.5), audio.plain(1.0)),
            (0.0, 50e3, 500e3)
        );
        let steps = spec(Taper::Stepped(&[4.0, 8.0, 12.0, 20.0]));
        assert_eq!([0.0, 0.26, 0.74, 1.0].map(|t| steps.plain(t)), [4.0, 8.0, 12.0, 20.0]);
    }
}
