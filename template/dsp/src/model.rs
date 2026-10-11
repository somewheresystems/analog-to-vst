//! The circuit being modeled. For a new circuit, edit this file and `reference/circuit.cir` together:
//! same parts, same values, same pot formulas.
//!
//! Example: op-amp gain stage into a diode clipper to ground, with gain and level pots.

use crate::control::{ControlSpec, Taper};
use crate::netlist::{Netlist, D1N4148, JRC4558};

pub const NAME: &str = "Example Clipper";
pub const VENDOR: &str = "Analog to VST";
/// Reverse-domain id, unique per plugin.
pub const CLAP_ID: &str = "com.example.example-clipper";
/// Exactly 16 bytes, unique per plugin.
pub const VST3_ID: [u8; 16] = *b"ExampleClipper01";

/// Volts at the jacks for a full-scale (0 dBFS) sample. 0.2 V is a hot guitar pickup's peak. At full
/// scale the circuit must stay inside its supply rails, since op-amp rail clipping isn't modeled.
pub const VOLTS_PER_FULL_SCALE: f64 = 0.2;

/// Front-panel controls, in order. Their names (upper-cased) are the `$NAME` placeholders in the
/// SPICE netlist, filled with `plain` values.
pub const CONTROLS: &[ControlSpec] = &[
    ControlSpec {
        name: "Gain",
        taper: Taper::Audio { max: 100e3 },
        default: 0.5,
    },
    ControlSpec {
        name: "Level",
        taper: Taper::Audio { max: 1.0 },
        default: 0.5,
    },
];

pub fn circuit(n: &mut Netlist) {
    n.supply("vcc", 9.0);
    n.resistor("vcc", "vref", 100e3);
    n.resistor("vref", "0", 100e3);
    n.capacitor("vref", "0", 10e-6);

    n.input("in");
    n.capacitor("in", "a", 10e-9);
    n.resistor("a", "vref", 1e6);
    n.opamp("a", "m", "o", JRC4558);
    n.resistor("m", "g", 4.7e3);
    n.capacitor("g", "vref", 47e-9);
    n.pot("RGAIN", "m", "o");

    n.capacitor("o", "d", 1e-6);
    n.resistor("d", "c", 10e3);
    n.diode_pair("c", "0", D1N4148);
    n.capacitor("c", "0", 1e-9);
    n.pot("RLEVEL_TOP", "c", "out");
    n.pot("RLEVEL_BOTTOM", "out", "0");
    n.resistor("out", "0", 1e6);
    n.output("out");
}

/// Pot resistances from control values in physical units. Same formulas as the netlist's `{...}`
/// expressions. Called with `set(pot name, ohms)`; values are clamped to at least 1 ohm.
pub fn pots(plain: &[f64], set: &mut dyn FnMut(&'static str, f64)) {
    let (gain, level) = (plain[0], plain[1]);
    set("RGAIN", gain);
    set("RLEVEL_TOP", 10e3 * (1.0 - level));
    set("RLEVEL_BOTTOM", 10e3 * level);
}
