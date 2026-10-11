//! Netlist builder with named nodes and SPICE-equivalent part models.
//!
//! Each helper adds the same elements the matching SPICE line would simulate, so the model and
//! `reference/circuit.cir` describe one circuit.

use crate::circuit::{Bjt, Element, Node};

/// Thermal voltage at 27 C, SPICE's default temperature.
pub const VT: f64 = 0.025_864_6;

/// SPICE `D` model parameters: saturation current, emission coefficient, series resistance.
/// `cjo` is added as a fixed capacitance (zero-bias junction capacitance).
#[derive(Clone, Copy, Debug)]
pub struct DiodeModel {
    pub is: f64,
    pub n: f64,
    pub rs: f64,
    pub cjo: f64,
}

/// SPICE Gummel-Poon `NPN`/`PNP` parameters used by the solver (charge storage and leakage omitted).
#[derive(Clone, Copy, Debug)]
pub struct BjtModel {
    pub is: f64,
    pub bf: f64,
    pub br: f64,
    pub vaf: f64,
    pub ikf: f64,
    pub rb: f64,
    pub rc: f64,
    pub re: f64,
}

/// Single-pole op-amp: open-loop gain `a0` (V/V) and gain-bandwidth product `gbw` (Hz).
/// Matches the `opamp` subcircuit in the SPICE template.
#[derive(Clone, Copy, Debug)]
pub struct OpAmp {
    pub a0: f64,
    pub gbw: f64,
}

pub const D1N4148: DiodeModel = DiodeModel {
    is: 2.52e-9,
    n: 1.752,
    rs: 0.568,
    cjo: 4e-12,
};
pub const Q2N3904: BjtModel = BjtModel {
    is: 1e-14,
    bf: 300.0,
    br: 4.0,
    vaf: 100.0,
    ikf: 0.4,
    rb: 20.0,
    rc: 0.1,
    re: 0.1,
};
pub const JRC4558: OpAmp = OpAmp { a0: 1e5, gbw: 3e6 };

#[derive(Default)]
pub struct Netlist {
    names: Vec<String>,
    pub(crate) elements: Vec<Element>,
    pub(crate) pots: Vec<(&'static str, usize)>,
    pub(crate) output: Node,
    internal: usize,
}

impl Netlist {
    /// Node by name; "0" and "gnd" are ground.
    pub fn node(&mut self, name: &str) -> Node {
        if name == "0" || name.eq_ignore_ascii_case("gnd") {
            return 0;
        }
        match self.names.iter().position(|n| n == name) {
            Some(i) => i + 1,
            None => {
                self.names.push(name.to_string());
                self.names.len()
            }
        }
    }

    fn fresh(&mut self) -> Node {
        self.internal += 1;
        let name = format!("_internal{}", self.internal);
        self.node(&name)
    }

    pub fn resistor(&mut self, a: &str, b: &str, ohms: f64) {
        let (a, b) = (self.node(a), self.node(b));
        self.elements.push(Element::Resistor(a, b, ohms));
    }

    pub fn capacitor(&mut self, a: &str, b: &str, farads: f64) {
        let (a, b) = (self.node(a), self.node(b));
        self.elements.push(Element::Capacitor(a, b, farads));
    }

    pub fn inductor(&mut self, a: &str, b: &str, henries: f64) {
        let (a, b) = (self.node(a), self.node(b));
        self.elements.push(Element::Inductor(a, b, henries));
    }

    /// A resistor whose value comes from `model::pots` (a pot section or variable resistor).
    pub fn pot(&mut self, name: &'static str, a: &str, b: &str) {
        self.resistor(a, b, 1e3);
        self.pots.push((name, self.elements.len() - 1));
    }

    /// The audio input, a voltage source from `node` to ground.
    pub fn input(&mut self, node: &str) {
        let n = self.node(node);
        self.elements.push(Element::Input(n, 0));
    }

    /// The audio output, measured from `node` to ground.
    pub fn output(&mut self, node: &str) {
        self.output = self.node(node);
    }

    /// A supply: constant voltage from `node` to ground.
    pub fn supply(&mut self, node: &str, volts: f64) {
        let n = self.node(node);
        self.elements.push(Element::Dc(n, 0, volts));
    }

    pub fn diode(&mut self, anode: &str, cathode: &str, m: DiodeModel) {
        let (a, b) = (self.node(anode), self.node(cathode));
        let j = self.series_rs(a, m.rs);
        self.elements.push(Element::Diode {
            a: j,
            b,
            is: m.is,
            nvt: m.n * VT,
        });
        self.elements.push(Element::Capacitor(j, b, m.cjo));
    }

    /// Two identical diodes in antiparallel between `a` and `b`. One series resistance stands in
    /// for both diodes' RS, exact while only one diode conducts at a time.
    pub fn diode_pair(&mut self, a: &str, b: &str, m: DiodeModel) {
        let (a, b) = (self.node(a), self.node(b));
        let j = self.series_rs(a, m.rs);
        self.elements.push(Element::DiodePair {
            a: j,
            b,
            is: m.is,
            nvt: m.n * VT,
        });
        self.elements.push(Element::Capacitor(j, b, 2.0 * m.cjo));
    }

    /// Node behind a diode's series resistance (the node itself when there is none).
    fn series_rs(&mut self, a: Node, rs: f64) -> Node {
        if rs <= 0.0 {
            return a;
        }
        let j = self.fresh();
        self.elements.push(Element::Resistor(a, j, rs));
        j
    }

    pub fn npn(&mut self, c: &str, b: &str, e: &str, m: BjtModel) {
        self.bjt(c, b, e, m, false);
    }

    pub fn pnp(&mut self, c: &str, b: &str, e: &str, m: BjtModel) {
        self.bjt(c, b, e, m, true);
    }

    fn bjt(&mut self, c: &str, b: &str, e: &str, m: BjtModel, pnp: bool) {
        let (c, b, e) = (self.node(c), self.node(b), self.node(e));
        let (ci, bi, ei) = (self.fresh(), self.fresh(), self.fresh());
        let model = Bjt {
            is: m.is,
            bf: m.bf,
            br: m.br,
            vaf: m.vaf,
            ikf: m.ikf,
            vt: VT,
        };
        self.elements.extend([
            Element::Resistor(b, bi, m.rb.max(1e-3)),
            Element::Resistor(c, ci, m.rc.max(1e-3)),
            Element::Resistor(ei, e, m.re.max(1e-3)),
            if pnp {
                Element::Pnp {
                    c: ci,
                    b: bi,
                    e: ei,
                    model,
                }
            } else {
                Element::Npn {
                    c: ci,
                    b: bi,
                    e: ei,
                    model,
                }
            },
        ]);
    }

    /// Op-amp with ideal output, inputs drawing no current, no rail clipping.
    pub fn opamp(&mut self, plus: &str, minus: &str, out: &str, m: OpAmp) {
        let (p, n, o, x) = (self.node(plus), self.node(minus), self.node(out), self.fresh());
        // Transconductance into 1k || C gives gain a0 with its pole at gbw / a0.
        let r = 1e3;
        self.elements.extend([
            Element::Vccs {
                p: 0,
                n: x,
                cp: p,
                cn: n,
                gm: m.a0 / r,
            },
            Element::Resistor(x, 0, r),
            Element::Capacitor(x, 0, m.a0 / (2.0 * std::f64::consts::PI * m.gbw * r)),
            Element::Vcvs {
                p: o,
                n: 0,
                cp: x,
                cn: 0,
                gain: 1.0,
            },
        ]);
    }
}
