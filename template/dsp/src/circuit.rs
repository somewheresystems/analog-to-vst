//! Nodal DK-method circuit solver (Holters & Zölzer, DAFx 2015).
//!
//! A netlist of linear elements and nonlinear devices is discretized with the trapezoidal rule and
//! reduced to a state-space system. Each sample costs one matrix-vector product plus a Newton solve
//! over the nonlinear devices' port voltages only. Devices that do not load each other (for example
//! stages separated by op-amps) are solved one after another rather than jointly.
//!
//! The circuit starts at its DC operating point, found with a SPICE-style Newton solve. Variable
//! resistors (pots) can change at runtime; `update` rebuilds the matrices without allocating.

// Matrix code reads more clearly with explicit row and column indices.
#![allow(clippy::needless_range_loop)]

/// Conductance standing in for an inductor at DC.
const INDUCTOR_DC_SHORT: f64 = 1e9;

/// Node index; 0 is ground.
pub type Node = usize;

#[derive(Clone, Copy, Debug)]
pub struct Bjt {
    pub is: f64,
    pub bf: f64,
    pub br: f64,
    /// Forward Early voltage.
    pub vaf: f64,
    /// Forward high-injection knee current.
    pub ikf: f64,
    pub vt: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Element {
    Resistor(Node, Node, f64),
    Capacitor(Node, Node, f64),
    Inductor(Node, Node, f64),
    /// The circuit input: v(a) - v(b) equals the input sample.
    Input(Node, Node),
    /// Constant voltage source: v(a) - v(b) = volts.
    Dc(Node, Node, f64),
    /// Current `gm * (v(cp) - v(cn))` flowing from `p` to `n` through the source (SPICE G).
    Vccs {
        p: Node,
        n: Node,
        cp: Node,
        cn: Node,
        gm: f64,
    },
    /// `v(p) - v(n) = gain * (v(cp) - v(cn))` (SPICE E).
    Vcvs {
        p: Node,
        n: Node,
        cp: Node,
        cn: Node,
        gain: f64,
    },
    /// Antiparallel pair of identical diodes: `i = 2 Is sinh(v / (N Vt))` from `a` to `b`.
    DiodePair {
        a: Node,
        b: Node,
        is: f64,
        nvt: f64,
    },
    /// Single diode, anode `a`, cathode `b`: `i = Is (exp(v / (N Vt)) - 1)`.
    Diode {
        a: Node,
        b: Node,
        is: f64,
        nvt: f64,
    },
    /// NPN transistor, Gummel-Poon without charge storage or leakage. Add terminal resistances
    /// (RB, RC, RE) as resistors.
    Npn {
        c: Node,
        b: Node,
        e: Node,
        model: Bjt,
    },
    /// PNP transistor, same model with all junction polarities reversed.
    Pnp {
        c: Node,
        b: Node,
        e: Node,
        model: Bjt,
    },
}

#[derive(Clone, Copy)]
enum Device {
    DiodePair {
        is: f64,
        nvt: f64,
    },
    Diode {
        is: f64,
        nvt: f64,
    },
    /// `polarity` is 1 for NPN, -1 for PNP.
    Bjt {
        model: Bjt,
        polarity: f64,
    },
}

impl Device {
    fn ports(&self) -> usize {
        match self {
            Device::DiodePair { .. } | Device::Diode { .. } => 1,
            Device::Bjt { .. } => 2,
        }
    }

    /// Whether a Newton step this small leaves negligible error. Convergence is quadratic in the
    /// junction voltages: after a step under 10 uV the remaining error is about step^2 / (2 N Vt),
    /// under 2 nV. A reverse-biased junction is nearly linear, so its step does not matter.
    #[inline]
    fn converged(&self, v: &[f64], step: &[f64]) -> bool {
        const TOL: f64 = 1e-5;
        match self {
            Device::DiodePair { .. } => step[0].abs() < TOL,
            Device::Diode { .. } => step[0].abs() < TOL || v[0] < -0.2,
            Device::Bjt { polarity, .. } => {
                let (vbc, step_vbc) = (polarity * (v[1] - v[0]), step[1] - step[0]);
                step[1].abs() < TOL && (vbc < -0.2 || step_vbc.abs() < TOL)
            }
        }
    }

    /// Port currents and their Jacobian (row-major, ports x ports) at port voltages `v`.
    #[inline]
    fn eval(&self, v: &[f64], i: &mut [f64; 2], j: &mut [f64; 4]) {
        match *self {
            Device::DiodePair { is, nvt } => {
                let ep = (v[0] / nvt).clamp(-200.0, 200.0).exp();
                let en = 1.0 / ep;
                i[0] = is * (ep - en);
                j[0] = is / nvt * (ep + en);
            }
            Device::Diode { is, nvt } => {
                let e = (v[0] / nvt).min(200.0).exp();
                i[0] = is * (e - 1.0);
                j[0] = is / nvt * e;
            }
            // Ports: (c, e) carries Ic, (b, e) carries Ib. vbc = vbe - vce. A PNP is an NPN with
            // voltages and currents negated; the Jacobian is unchanged by the double negation.
            Device::Bjt { model: m, polarity: s } => {
                let (vce, vbe) = (s * v[0], s * v[1]);
                let vbc = vbe - vce;
                let ef = (vbe / m.vt).min(80.0).exp();
                let er = (vbc / m.vt).min(80.0).exp();
                let (if_, ir) = (m.is * (ef - 1.0), m.is * (er - 1.0));
                let (gf, gr) = (m.is * ef / m.vt, m.is * er / m.vt);
                let q1 = 1.0 / (1.0 - vbc / m.vaf);
                let dq1_dvbc = q1 * q1 / m.vaf;
                let sq = (1.0 + 4.0 * if_ / m.ikf).max(0.0).sqrt();
                let qb = q1 * (1.0 + sq) / 2.0;
                let dqb_dvbe = q1 * gf / m.ikf / sq;
                let dqb_dvbc = dq1_dvbc * (1.0 + sq) / 2.0;
                let ic = (if_ - ir) / qb - ir / m.br;
                let dic_dvbe = gf / qb - (if_ - ir) / (qb * qb) * dqb_dvbe;
                let dic_dvbc = -gr / qb - (if_ - ir) / (qb * qb) * dqb_dvbc - gr / m.br;
                let ib = if_ / m.bf + ir / m.br;
                let (dib_dvbe, dib_dvbc) = (gf / m.bf, gr / m.br);
                i[0] = s * ic;
                i[1] = s * ib;
                // d/dvce = -d/dvbc; d/dvbe (port) = d/dvbe + d/dvbc.
                j[0] = -dic_dvbc;
                j[1] = dic_dvbe + dic_dvbc;
                j[2] = -dib_dvbc;
                j[3] = dib_dvbe + dib_dvbc;
            }
        }
    }
}

/// Dense row-major matrix.
#[derive(Clone)]
struct Mat {
    cols: usize,
    data: Vec<f64>,
}

impl Mat {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            cols,
            data: vec![0.0; rows * cols],
        }
    }
    fn at(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }
    fn at_mut(&mut self, r: usize, c: usize) -> &mut f64 {
        &mut self.data[r * self.cols + c]
    }
    fn row(&self, r: usize) -> &[f64] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }
}

#[derive(Clone)]
pub struct Circuit {
    elements: Vec<Element>,
    output: (Node, Node),
    nodes: usize,
    size: usize,
    rate: f64,
    dirty: bool,
    caps: Vec<(Node, Node, f64)>,
    ports: Vec<(Node, Node)>,
    /// Devices with their first port index, in solve order.
    devices: Vec<(Device, usize)>,
    /// True when one sweep of per-device solves is enough; otherwise sweeps repeat until the device
    /// currents agree (block Gauss-Seidel).
    sequential: bool,
    /// True when devices only depend on earlier ones, so later currents start each sample at zero.
    zero_currents: bool,
    /// True when devices load each other strongly enough to need one joint Newton solve.
    joint: bool,
    jac: Mat,
    input_row: usize,
    // Workspace for `update`.
    s: Mat,
    sinv: Mat,
    perm: Vec<usize>,
    w: Mat,
    // State-space over z = [x, u, 1, i]. `lin` is column-major with nx + 1 + nn rows: capacitor
    // voltages scaled by 2 Gc (so the state update is x' = row - x), the output, then the port
    // voltages. `port_v` repeats the port rows for device coupling.
    lin: Vec<f64>,
    port_v: Mat,
    /// State update x' = gc2 * v + beta * x per reactive element.
    gc2: Vec<f64>,
    beta: Vec<f64>,
    // Runtime state.
    x: Vec<f64>,
    z: Vec<f64>,
    acc: Vec<f64>,
    v: Vec<f64>,
    v_prev: Vec<f64>,
    i_used: Vec<f64>,
    i_prev: Vec<f64>,
    operating_point: Vec<f64>,
}

impl Circuit {
    pub fn new(elements: Vec<Element>, output: (Node, Node), rate: f64) -> Self {
        let node_max = |e: &Element| match *e {
            Element::Resistor(a, b, _)
            | Element::Capacitor(a, b, _)
            | Element::Inductor(a, b, _)
            | Element::Input(a, b)
            | Element::Dc(a, b, _) => a.max(b),
            Element::Vccs { p, n, cp, cn, .. } | Element::Vcvs { p, n, cp, cn, .. } => p.max(n).max(cp).max(cn),
            Element::DiodePair { a, b, .. } | Element::Diode { a, b, .. } => a.max(b),
            Element::Npn { c, b, e, .. } | Element::Pnp { c, b, e, .. } => c.max(b).max(e),
        };
        let nodes = elements
            .iter()
            .map(node_max)
            .max()
            .unwrap_or(0)
            .max(output.0)
            .max(output.1);
        let extra = elements
            .iter()
            .filter(|e| matches!(e, Element::Input(..) | Element::Dc(..) | Element::Vcvs { .. }))
            .count();
        let inputs = elements.iter().filter(|e| matches!(e, Element::Input(..))).count();
        assert_eq!(inputs, 1, "a circuit needs exactly one input");

        // Reactive elements as trapezoidal companions: conductance g and history state x, with
        // current a->b of g v - x. Capacitors update x' = 2g v - x; inductors x' = -2g v + x.
        let caps: Vec<(Node, Node, f64)> = elements
            .iter()
            .filter_map(|e| match *e {
                Element::Capacitor(a, b, c) => Some((a, b, 2.0 * c * rate)),
                Element::Inductor(a, b, l) => Some((a, b, 1.0 / (2.0 * l * rate))),
                _ => None,
            })
            .collect();
        let beta: Vec<f64> = elements
            .iter()
            .filter_map(|e| match e {
                Element::Capacitor(..) => Some(-1.0),
                Element::Inductor(..) => Some(1.0),
                _ => None,
            })
            .collect();
        let (mut ports, mut devices) = (Vec::new(), Vec::new());
        for el in &elements {
            match *el {
                Element::DiodePair { a, b, is, nvt } => {
                    devices.push((Device::DiodePair { is, nvt }, ports.len()));
                    ports.push((a, b));
                }
                Element::Diode { a, b, is, nvt } => {
                    devices.push((Device::Diode { is, nvt }, ports.len()));
                    ports.push((a, b));
                }
                Element::Npn { c, b, e, model } | Element::Pnp { c, b, e, model } => {
                    let polarity = if matches!(el, Element::Pnp { .. }) { -1.0 } else { 1.0 };
                    devices.push((Device::Bjt { model, polarity }, ports.len()));
                    ports.extend([(c, e), (b, e)]);
                }
                _ => {}
            }
        }
        let (size, nx, nn) = (nodes + extra, caps.len(), ports.len());
        let m = nx + 2 + nn;
        let mut circuit = Self {
            output,
            nodes,
            size,
            rate,
            dirty: true,
            sequential: true,
            zero_currents: true,
            joint: false,
            jac: Mat::new(nn, nn + 1),
            input_row: 0,
            s: Mat::new(size, size),
            sinv: Mat::new(size, size),
            perm: vec![0; size],
            w: Mat::new(size, m),
            lin: vec![0.0; (nx + 1 + nn) * m],
            port_v: Mat::new(nn, m),
            gc2: caps.iter().zip(&beta).map(|(&(_, _, g), &b)| -2.0 * b * g).collect(),
            beta,
            x: vec![0.0; nx],
            z: vec![0.0; m],
            acc: vec![0.0; nx + 1 + nn],
            v: vec![0.0; nn],
            v_prev: vec![0.0; nn],
            i_used: vec![0.0; nn],
            i_prev: vec![0.0; nn],
            operating_point: vec![0.0; nodes],
            caps,
            ports,
            devices,
            elements,
        };
        circuit.update();
        circuit.order_devices();
        circuit.solve_operating_point();
        circuit
    }

    /// Changes the value of the resistor at `index` in the netlist. Takes effect on the next sample.
    pub fn set_resistor(&mut self, index: usize, ohms: f64) {
        if let Element::Resistor(_, _, r) = &mut self.elements[index] {
            if *r != ohms {
                *r = ohms;
                self.dirty = true;
            }
        } else {
            panic!("element {index} is not a resistor");
        }
    }

    /// Node voltages at the DC operating point found at construction (index = node - 1).
    #[cfg(test)]
    pub fn operating_point(&self) -> &[f64] {
        &self.operating_point
    }

    /// Fills the MNA matrix. Capacitors stamp their companion conductance, or are open when `dc`.
    fn stamp_all(&mut self, dc: bool) {
        self.s.data.fill(0.0);
        let s = &mut self.s;
        let mut extra = self.nodes;
        for e in &self.elements {
            match *e {
                Element::Resistor(a, b, r) => conductance(s, a, b, 1.0 / r),
                Element::Capacitor(a, b, c) => {
                    if !dc {
                        conductance(s, a, b, 2.0 * c * self.rate)
                    }
                }
                // At DC an inductor is a short; a large conductance keeps the matrix shape.
                Element::Inductor(a, b, l) => conductance(
                    s,
                    a,
                    b,
                    if dc {
                        INDUCTOR_DC_SHORT
                    } else {
                        1.0 / (2.0 * l * self.rate)
                    },
                ),
                Element::Vccs { p, n, cp, cn, gm } => {
                    stamp(s, p, cp, gm);
                    stamp(s, p, cn, -gm);
                    stamp(s, n, cp, -gm);
                    stamp(s, n, cn, gm);
                }
                Element::Input(a, b) | Element::Dc(a, b, _) | Element::Vcvs { p: a, n: b, .. } => {
                    extra += 1;
                    for (node, sign) in [(a, 1.0), (b, -1.0)] {
                        stamp(s, node, extra, sign);
                        stamp(s, extra, node, sign);
                    }
                    match *e {
                        Element::Vcvs { cp, cn, gain, .. } => {
                            stamp(s, extra, cp, -gain);
                            stamp(s, extra, cn, gain);
                        }
                        Element::Input(..) => self.input_row = extra - 1,
                        _ => {}
                    }
                }
                Element::DiodePair { .. } | Element::Diode { .. } | Element::Npn { .. } | Element::Pnp { .. } => {}
            }
        }
    }

    /// Value of the DC column of the right-hand side at matrix row `r`.
    fn dc_rhs(&self, r: usize) -> f64 {
        let mut extra = self.nodes;
        for e in &self.elements {
            if let Element::Input(..) | Element::Dc(..) | Element::Vcvs { .. } = e {
                if extra == r {
                    return if let Element::Dc(_, _, v) = e { *v } else { 0.0 };
                }
                extra += 1;
            }
        }
        0.0
    }

    /// Rebuilds the state-space matrices from the netlist. Allocation-free.
    fn update(&mut self) {
        self.dirty = false;
        self.stamp_all(false);
        invert(&mut self.s, &mut self.sinv, &mut self.perm);

        // W = S^-1 J; J's columns inject capacitor history, the input, DC sources and port currents.
        let (n, nx, nn) = (self.size, self.caps.len(), self.ports.len());
        let col_diff = |sinv: &Mat, r: usize, a: Node, b: Node| node_val(sinv.row(r), a) - node_val(sinv.row(r), b);
        for r in 0..n {
            for (k, &(a, b, _)) in self.caps.iter().enumerate() {
                *self.w.at_mut(r, k) = col_diff(&self.sinv, r, a, b);
            }
            *self.w.at_mut(r, nx) = self.sinv.at(r, self.input_row);
            let mut dc = 0.0;
            for c in self.nodes..n {
                let v = self.dc_rhs(c);
                if v != 0.0 {
                    dc += self.sinv.at(r, c) * v;
                }
            }
            *self.w.at_mut(r, nx + 1) = dc;
            for (k, &(a, b)) in self.ports.iter().enumerate() {
                *self.w.at_mut(r, nx + 2 + k) = -col_diff(&self.sinv, r, a, b);
            }
        }
        let row_diff = |w: &Mat, c: usize, a: Node, b: Node| {
            (if a > 0 { w.at(a - 1, c) } else { 0.0 }) - (if b > 0 { w.at(b - 1, c) } else { 0.0 })
        };
        let rows = nx + 1 + nn;
        for c in 0..nx + 2 + nn {
            for (k, &(a, b, _)) in self.caps.iter().enumerate() {
                self.lin[c * rows + k] = self.gc2[k] * row_diff(&self.w, c, a, b);
            }
            self.lin[c * rows + nx] = row_diff(&self.w, c, self.output.0, self.output.1);
            for (k, &(a, b)) in self.ports.iter().enumerate() {
                let v = row_diff(&self.w, c, a, b);
                self.lin[c * rows + nx + 1 + k] = v;
                *self.port_v.at_mut(k, c) = v;
            }
        }
    }

    /// Orders devices along their strong dependencies (upstream first), so block Gauss-Seidel
    /// sweeps only have to absorb weak feedback, such as currents through a shared bias node.
    fn order_devices(&mut self) {
        let base = self.caps.len() + 2;
        let nd = self.devices.len();
        let span = |d: &(Device, usize)| d.1..d.1 + d.0.ports();
        // coupling(a, b): largest effect of device b's currents on device a's port voltages,
        // relative to a's own driving-point impedance.
        let coupling = |a: usize, b: usize, devs: &[(Device, usize)], pv: &Mat| {
            span(&devs[a])
                .map(|r| {
                    let own = span(&devs[a]).map(|c| pv.at(r, base + c).abs()).fold(1e-30, f64::max);
                    span(&devs[b]).map(|c| pv.at(r, base + c).abs()).fold(0.0, f64::max) / own
                })
                .fold(0.0, f64::max)
        };
        let mut order: Vec<usize> = Vec::with_capacity(nd);
        let mut placed = vec![false; nd];
        while order.len() < nd {
            // Next: the unplaced device least driven by other unplaced devices.
            let next = (0..nd)
                .filter(|&a| !placed[a])
                .min_by(|&a, &b| {
                    let drive = |x: usize| {
                        (0..nd)
                            .filter(|&y| y != x && !placed[y])
                            .map(|y| coupling(x, y, &self.devices, &self.port_v))
                            .fold(0.0, f64::max)
                    };
                    drive(a).total_cmp(&drive(b))
                })
                .unwrap();
            placed[next] = true;
            order.push(next);
        }
        // One sweep suffices when devices only depend on earlier ones, or when every reverse
        // coupling's loop gain (a Gauss-Seidel sweep's convergence factor) is tiny.
        let order_ref = &order;
        let pairs = || (0..nd).flat_map(move |i| (i + 1..nd).map(move |j| (order_ref[i], order_ref[j])));
        let exact = pairs().all(|(a, b)| coupling(a, b, &self.devices, &self.port_v) < 1e-12);
        let loop_gain = pairs()
            .map(|(a, b)| coupling(a, b, &self.devices, &self.port_v) * coupling(b, a, &self.devices, &self.port_v))
            .fold(0.0, f64::max);
        self.zero_currents = exact;
        self.sequential = exact || loop_gain < 1e-3;
        // Strong mutual feedback (e.g. two-transistor fuzz circuits): solve all ports jointly.
        self.joint = !exact && loop_gain >= 0.1;
        self.devices = order.iter().map(|&i| self.devices[i]).collect();
    }

    /// SPICE-style DC operating point: Newton on the full nodal equations with capacitors open,
    /// stepping the DC sources up from zero. Sets the capacitor states and port voltages.
    fn solve_operating_point(&mut self) {
        let n = self.size;
        self.stamp_all(true);
        let s_dc = self.s.clone();
        let mut z = vec![0.0; n];
        let mut jac = Mat::new(n, n);
        let mut inv = Mat::new(n, n);
        let mut perm = vec![0; n];
        let mut residual = vec![0.0; n];
        let (mut i, mut j) = ([0.0; 2], [0.0; 4]);
        let port_voltage = |z: &[f64], (a, b): (Node, Node)| node_val(z, a) - node_val(z, b);
        for step in 1..=20 {
            let lambda = step as f64 / 20.0;
            for _ in 0..200 {
                // F = S z + sum over ports of (e_a - e_b) i - lambda b; J = S + port Jacobian stamps.
                jac.data.copy_from_slice(&s_dc.data);
                for r in 0..n {
                    residual[r] = (0..n).map(|c| s_dc.at(r, c) * z[c]).sum::<f64>() - lambda * self.dc_rhs(r);
                }
                for &(dev, first) in &self.devices {
                    let np = dev.ports();
                    let mut v = [0.0; 2];
                    for k in 0..np {
                        v[k] = port_voltage(&z, self.ports[first + k]);
                    }
                    dev.eval(&v[..np], &mut i, &mut j);
                    for k in 0..np {
                        let (a, b) = self.ports[first + k];
                        add_node(&mut residual, a, i[k]);
                        add_node(&mut residual, b, -i[k]);
                        for l in 0..np {
                            let (c, d) = self.ports[first + l];
                            let g = j[k * np + l];
                            for (row, rs) in [(a, 1.0), (b, -1.0)] {
                                for (col, cs) in [(c, 1.0), (d, -1.0)] {
                                    stamp(&mut jac, row, col, rs * cs * g);
                                }
                            }
                        }
                    }
                }
                invert(&mut jac, &mut inv, &mut perm);
                let delta: Vec<f64> = (0..n)
                    .map(|r| (0..n).map(|c| inv.at(r, c) * residual[c]).sum())
                    .collect();
                let max_port_step = self
                    .ports
                    .iter()
                    .map(|&p| port_voltage(&delta, p).abs())
                    .fold(0.0, f64::max);
                let damp = if max_port_step > 0.1 { 0.1 / max_port_step } else { 1.0 };
                for (zi, d) in z.iter_mut().zip(&delta) {
                    *zi -= damp * d;
                }
                if damp == 1.0 && delta.iter().all(|d| d.abs() < 1e-12) {
                    break;
                }
            }
        }
        self.operating_point.copy_from_slice(&z[..self.nodes]);
        for (k, &(a, b, g)) in self.caps.iter().enumerate() {
            let v = node_val(&z, a) - node_val(&z, b);
            // Steady state: no capacitor current (x = g v); inductor current i = g v - x carries the
            // DC current through the short.
            self.x[k] = if self.beta[k] < 0.0 {
                g * v
            } else {
                g * v - INDUCTOR_DC_SHORT * v
            };
        }
        for (k, &p) in self.ports.iter().enumerate() {
            self.v[k] = port_voltage(&z, p);
        }
        self.v_prev.copy_from_slice(&self.v);
        let base = self.caps.len() + 2;
        for &(dev, first) in &self.devices {
            let np = dev.ports();
            dev.eval(&self.v[first..first + np], &mut i, &mut j);
            self.z[base + first..base + first + np].copy_from_slice(&i[..np]);
            self.i_prev[first..first + np].copy_from_slice(&i[..np]);
        }
    }

    /// Processes one sample at the circuit's rate.
    pub fn tick(&mut self, u: f64) -> f64 {
        if self.dirty {
            self.update();
        }
        let (nx, nn) = (self.caps.len(), self.ports.len());
        let (base, rows) = (nx + 2, nx + 1 + nn);
        self.z[nx] = u;
        self.z[nx + 1] = 1.0;
        // Pass 1: capacitor voltages, output and port voltages from the state, input and supplies,
        // as a column-major matrix-vector product (vectorizes across rows).
        self.acc.fill(0.0);
        for (c, col) in self.lin.chunks_exact(rows).take(base).enumerate() {
            let zc = if c < nx { self.x[c] } else { self.z[c] };
            for (a, &m) in self.acc.iter_mut().zip(col) {
                *a += m * zc;
            }
        }
        // Port voltages and device currents start from a linear extrapolation of the last two samples.
        for k in 0..nn {
            let guess = 2.0 * self.v[k] - self.v_prev[k];
            self.v_prev[k] = self.v[k];
            self.v[k] = guess;
            let i = self.z[base + k];
            self.z[base + k] = if self.zero_currents {
                0.0
            } else {
                2.0 * i - self.i_prev[k]
            };
            self.i_prev[k] = i;
        }
        if self.joint {
            self.solve_joint();
        } else {
            for _ in 0..32 {
                self.i_used.copy_from_slice(&self.z[base..]);
                for d in 0..self.devices.len() {
                    self.solve_device(d);
                }
                if self.sequential || self.sweep_error() < 1e-7 {
                    break;
                }
            }
        }
        // Pass 2: add the device currents' contribution to the capacitor voltages and output.
        for (col, &zc) in self.lin.chunks_exact(rows).skip(base).zip(&self.z[base..]) {
            for (a, &m) in self.acc[..nx + 1].iter_mut().zip(col) {
                *a += m * zc;
            }
        }
        for ((x, &a), &b) in self.x.iter_mut().zip(&self.acc).zip(&self.beta) {
            *x = a + b * *x;
        }
        self.acc[nx]
    }

    /// Newton over all ports at once: v = p + K f(v), with p the linear part of each port voltage.
    fn solve_joint(&mut self) {
        let nx = self.caps.len();
        let (base, nn) = (nx + 2, self.ports.len());
        let (mut i, mut j) = ([0.0; 2], [0.0; 4]);
        for _ in 0..64 {
            self.jac.data.fill(0.0);
            for &(dev, first) in &self.devices {
                let np = dev.ports();
                dev.eval(&self.v[first..first + np], &mut i, &mut j);
                self.z[base + first..base + first + np].copy_from_slice(&i[..np]);
                // J = I - K Jf: column (first + b) gets -sum_a K[:, first + a] Jf[a][b].
                for r in 0..nn {
                    for b in 0..np {
                        let kj: f64 = (0..np)
                            .map(|a| self.port_v.at(r, base + first + a) * j[a * np + b])
                            .sum();
                        *self.jac.at_mut(r, first + b) -= kj;
                    }
                }
            }
            for r in 0..nn {
                *self.jac.at_mut(r, r) += 1.0;
                let ki = dot(&self.port_v.row(r)[base..], &self.z[base..]);
                *self.jac.at_mut(r, nn) = self.v[r] - self.acc[nx + 1 + r] - ki;
            }
            solve_small(&mut self.jac, nn);
            let big = (0..nn).fold(0.0f64, |m, k| m.max(self.jac.at(k, nn).abs()));
            let scale = if big > 0.1 { 0.1 / big } else { 1.0 };
            for k in 0..nn {
                self.v[k] -= self.jac.at(k, nn) * scale;
            }
            if big * scale < 1e-9 {
                break;
            }
        }
        for &(dev, first) in &self.devices {
            let np = dev.ports();
            dev.eval(&self.v[first..first + np], &mut i, &mut j);
            self.z[base + first..base + first + np].copy_from_slice(&i[..np]);
        }
    }

    /// Bound on how far earlier devices' port voltages are from consistent with the currents that
    /// later devices settled on during the last sweep (in volts).
    fn sweep_error(&self) -> f64 {
        let base = self.caps.len() + 2;
        let mut worst = 0.0f64;
        for (d, &(dev, first)) in self.devices.iter().enumerate() {
            let later = self.devices.get(d + 1).map_or(self.ports.len(), |x| x.1);
            for r in first..first + dev.ports() {
                let row = &self.port_v.row(r)[base..];
                let err: f64 = (later..self.ports.len())
                    .map(|c| (row[c] * (self.z[base + c] - self.i_used[c])).abs())
                    .sum();
                worst = worst.max(err);
            }
        }
        worst
    }

    /// Newton solve of one device's ports: v = p + K_dd f(v), where p holds the linear part and the
    /// other devices' latest currents. Returns the largest change in this device's port voltages.
    fn solve_device(&mut self, d: usize) -> f64 {
        let nx = self.caps.len();
        let base = nx + 2;
        let (dev, first) = self.devices[d];
        let np = dev.ports();
        let (mut p, mut k, mut v) = ([0.0; 2], [0.0; 4], [0.0; 2]);
        for r in 0..np {
            let row = &self.port_v.row(first + r)[base..];
            p[r] = self.acc[nx + 1 + first + r] + dot(row, &self.z[base..]);
            for c in 0..np {
                k[r * np + c] = row[first + c];
                p[r] -= k[r * np + c] * self.z[base + first + c];
            }
            v[r] = self.v[first + r];
        }
        let (mut i, mut j, mut cur) = ([0.0; 2], [0.0; 4], [0.0; 2]);
        for _ in 0..64 {
            dev.eval(&v[..np], &mut i, &mut j);
            let mut step = [0.0; 2];
            if np == 1 {
                step[0] = (v[0] - p[0] - k[0] * i[0]) / (1.0 - k[0] * j[0]);
            } else {
                // F = v - p - K i; J = I - K Jf (2x2, solved in closed form).
                let f0 = v[0] - p[0] - (k[0] * i[0] + k[1] * i[1]);
                let f1 = v[1] - p[1] - (k[2] * i[0] + k[3] * i[1]);
                let a = 1.0 - (k[0] * j[0] + k[1] * j[2]);
                let b = -(k[0] * j[1] + k[1] * j[3]);
                let c = -(k[2] * j[0] + k[3] * j[2]);
                let e = 1.0 - (k[2] * j[1] + k[3] * j[3]);
                let det = a * e - b * c;
                step = [(e * f0 - b * f1) / det, (a * f1 - c * f0) / det];
            }
            let big = step[..np].iter().fold(0.0f64, |m, s| m.max(s.abs()));
            let scale = if big > 0.1 { 0.1 / big } else { 1.0 };
            for r in 0..np {
                step[r] *= scale;
                v[r] -= step[r];
            }
            let done = dev.converged(&v[..np], &step[..np]);
            // Currents at the updated v from the linearization; exact to second order in the step.
            for r in 0..np {
                cur[r] = i[r] - (0..np).map(|c| j[r * np + c] * step[c]).sum::<f64>();
            }
            if done {
                break;
            }
        }
        let mut change = 0.0f64;
        for r in 0..np {
            change = change.max((v[r] - self.v[first + r]).abs());
            self.z[base + first + r] = cur[r];
            self.v[first + r] = v[r];
        }
        change
    }
}

/// Value at a node from a node-indexed vector (index = node - 1); ground is zero.
fn node_val(v: &[f64], node: Node) -> f64 {
    if node > 0 {
        v[node - 1]
    } else {
        0.0
    }
}

fn add_node(v: &mut [f64], node: Node, x: f64) {
    if node > 0 {
        v[node - 1] += x;
    }
}

/// Adds `v` at (r, c) of the MNA matrix; rows and columns are 1-based with 0 as ground.
fn stamp(s: &mut Mat, r: Node, c: Node, v: f64) {
    if r > 0 && c > 0 {
        *s.at_mut(r - 1, c - 1) += v;
    }
}

fn conductance(s: &mut Mat, a: Node, b: Node, g: f64) {
    stamp(s, a, a, g);
    stamp(s, b, b, g);
    stamp(s, a, b, -g);
    stamp(s, b, a, -g);
}

/// Dot product with four accumulators, so the adds are not one serial dependency chain.
#[inline]
pub(crate) fn dot(a: &[f64], b: &[f64]) -> f64 {
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

/// Inverts `a` into `inv` with partial-pivot LU. `a` is overwritten.
fn invert(a: &mut Mat, inv: &mut Mat, perm: &mut [usize]) {
    let n = perm.len();
    for (i, p) in perm.iter_mut().enumerate() {
        *p = i;
    }
    for k in 0..n {
        let pivot = (k..n)
            .max_by(|&i, &j| a.at(i, k).abs().total_cmp(&a.at(j, k).abs()))
            .unwrap();
        assert!(
            a.at(pivot, k).abs() > 1e-300,
            "singular circuit matrix (floating node?)"
        );
        if pivot != k {
            for c in 0..n {
                a.data.swap(k * n + c, pivot * n + c);
            }
            perm.swap(k, pivot);
        }
        for i in k + 1..n {
            let f = a.at(i, k) / a.at(k, k);
            *a.at_mut(i, k) = f;
            for c in k + 1..n {
                let v = a.at(k, c);
                *a.at_mut(i, c) -= f * v;
            }
        }
    }
    // Solve L U inv[:, j] = P e_j for each column.
    for j in 0..n {
        for i in 0..n {
            let mut v = if perm[i] == j { 1.0 } else { 0.0 };
            for k in 0..i {
                v -= a.at(i, k) * inv.at(k, j);
            }
            *inv.at_mut(i, j) = v;
        }
        for i in (0..n).rev() {
            let mut v = inv.at(i, j);
            for k in i + 1..n {
                v -= a.at(i, k) * inv.at(k, j);
            }
            *inv.at_mut(i, j) = v / a.at(i, i);
        }
    }
}

/// Gaussian elimination on an augmented n x (n + 1) system; the solution replaces the last column.
fn solve_small(m: &mut Mat, n: usize) {
    for k in 0..n {
        let pivot = (k..n)
            .max_by(|&i, &j| m.at(i, k).abs().total_cmp(&m.at(j, k).abs()))
            .unwrap();
        for c in 0..=n {
            m.data.swap(k * m.cols + c, pivot * m.cols + c);
        }
        for i in 0..n {
            if i != k {
                let f = m.at(i, k) / m.at(k, k);
                for c in k..=n {
                    let v = m.at(k, c);
                    *m.at_mut(i, c) -= f * v;
                }
            }
        }
    }
    for k in 0..n {
        *m.at_mut(k, n) /= m.at(k, k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const Q2N3904: Bjt = Bjt {
        is: 1e-14,
        bf: 300.0,
        br: 4.0,
        vaf: 100.0,
        ikf: 0.4,
        vt: 0.025_864_6,
    };

    /// Steady-state gain of a circuit at `freq`, by sine fit after settling.
    fn gain(c: &mut Circuit, freq: f64, amp: f64) -> f64 {
        let rate = c.rate;
        let n = (rate / freq * 200.0) as usize;
        let (mut s, mut co) = (0.0, 0.0);
        for i in 0..2 * n {
            let t = i as f64 / rate;
            let y = c.tick(amp * (2.0 * PI * freq * t).sin());
            if i >= n {
                s += y * (2.0 * PI * freq * t).sin();
                co += y * (2.0 * PI * freq * t).cos();
            }
        }
        2.0 * (s * s + co * co).sqrt() / n as f64 / amp
    }

    #[test]
    fn rc_lowpass_matches_bilinear_response() {
        let (r, cap, rate) = (1e3, 1e-6, 48_000.0);
        let mut c = Circuit::new(
            vec![
                Element::Input(1, 0),
                Element::Resistor(1, 2, r),
                Element::Capacitor(2, 0, cap),
            ],
            (2, 0),
            rate,
        );
        for f in [50.0, 159.15, 1000.0, 10_000.0] {
            // Trapezoidal integration is the bilinear transform: analog response at the warped frequency.
            let warped = 2.0 * rate * (PI * f / rate).tan();
            let expected = 1.0 / (1.0 + (warped * r * cap).powi(2)).sqrt();
            assert!((gain(&mut c, f, 1.0) - expected).abs() < 1e-4, "{f} Hz");
        }
    }

    #[test]
    fn rl_highpass_matches_bilinear_response_and_carries_dc() {
        // Series R into a shunt L, with a DC supply through a second resistor: the AC response is an
        // RL high-pass and the operating point puts the supply's current through the inductor.
        let (r, l, rate) = (1e3, 0.1, 96_000.0);
        let mut c = Circuit::new(
            vec![
                Element::Input(1, 0),
                Element::Resistor(1, 2, r),
                Element::Inductor(2, 0, l),
                Element::Dc(3, 0, 1.0),
                Element::Resistor(3, 2, 1e6),
            ],
            (2, 0),
            rate,
        );
        assert!(c.operating_point()[1].abs() < 1e-6, "inductor shorts its node at DC");
        let rp = r * 1e6 / (r + 1e6);
        for f in [100.0, 1591.5, 10_000.0] {
            let warped = 2.0 * rate * (PI * f / rate).tan();
            let x = warped * l / rp;
            let expected = x / (1.0 + x * x).sqrt() * 1e6 / (r + 1e6);
            assert!((gain(&mut c, f, 1.0) - expected).abs() < 1e-4, "{f} Hz");
        }
    }

    #[test]
    fn op_amp_follower_and_pot_update() {
        // Single-pole op-amp (same macro as the SPICE references) as a non-inverting amp, gain 1 + R2/R1.
        let opamp = |p, n, out, internal| {
            [
                Element::Vccs {
                    p: 0,
                    n: internal,
                    cp: p,
                    cn: n,
                    gm: 100.0,
                },
                Element::Resistor(internal, 0, 1e3),
                Element::Capacitor(internal, 0, 5.305e-6),
                Element::Vcvs {
                    p: out,
                    n: 0,
                    cp: internal,
                    cn: 0,
                    gain: 1.0,
                },
            ]
        };
        let mut els = vec![
            Element::Input(1, 0),
            Element::Resistor(2, 0, 1e3),
            Element::Resistor(3, 2, 9e3),
        ];
        els.extend(opamp(1, 2, 3, 4));
        let mut c = Circuit::new(els, (3, 0), 192_000.0);
        assert!((gain(&mut c, 1000.0, 0.1) - 10.0).abs() / 10.0 < 1e-3);
        c.set_resistor(2, 99e3);
        assert!((gain(&mut c, 1000.0, 0.01) - 100.0).abs() / 100.0 < 3e-3);
    }

    #[test]
    fn diode_clipper_satisfies_kcl() {
        // Series resistor into a diode pair to ground; check the output solves the nonlinear KCL.
        let (r, is, nvt) = (10e3, 2.52e-9, 1.752 * 0.025864);
        let mut c = Circuit::new(
            vec![
                Element::Input(1, 0),
                Element::Resistor(1, 2, r),
                Element::Capacitor(2, 0, 1e-9),
                Element::DiodePair { a: 2, b: 0, is, nvt },
            ],
            (2, 0),
            96_000.0,
        );
        for u in [0.01, 0.5, 2.0, 10.0, -3.0] {
            let mut v = 0.0;
            for _ in 0..2000 {
                v = c.tick(u);
            }
            let kcl = (u - v) / r - 2.0 * is * (v / nvt).sinh();
            assert!(kcl.abs() < 1e-12, "u={u} v={v} residual={kcl}");
        }
    }

    #[test]
    fn emitter_follower_operating_point_holds_at_rest() {
        // 9 V supply, base biased by a divider, 10k emitter resistor. The operating point satisfies
        // the transistor equations, and silence keeps the circuit there.
        let els = vec![
            Element::Dc(1, 0, 9.0),
            Element::Resistor(1, 2, 10e3),
            Element::Resistor(2, 0, 10e3),
            Element::Input(4, 0),
            Element::Capacitor(4, 2, 1e-6),
            Element::Npn {
                c: 1,
                b: 2,
                e: 3,
                model: Q2N3904,
            },
            Element::Resistor(3, 0, 10e3),
        ];
        let mut c = Circuit::new(els, (3, 0), 96_000.0);
        let op = c.operating_point().to_vec();
        let (vb, ve) = (op[1], op[2]);
        let (mut i, mut j) = ([0.0; 2], [0.0; 4]);
        Device::Bjt {
            model: Q2N3904,
            polarity: 1.0,
        }
        .eval(&[9.0 - ve, vb - ve], &mut i, &mut j);
        assert!((ve / 10e3 - (i[0] + i[1])).abs() < 1e-12, "emitter KCL");
        assert!(((9.0 - vb) / 10e3 - vb / 10e3 - i[1]).abs() < 1e-12, "base KCL");
        for _ in 0..1000 {
            assert!((c.tick(0.0) - ve).abs() < 1e-9);
        }
    }

    #[test]
    fn npn_jacobian_matches_finite_differences() {
        let dev = Device::Bjt {
            model: Q2N3904,
            polarity: 1.0,
        };
        let v = [5.0, 0.65];
        let (mut i0, mut j) = ([0.0; 2], [0.0; 4]);
        dev.eval(&v, &mut i0, &mut j);
        for c in 0..2 {
            let h = 1e-7;
            let mut vp = v;
            vp[c] += h;
            let (mut ip, mut jp) = ([0.0; 2], [0.0; 4]);
            dev.eval(&vp, &mut ip, &mut jp);
            for r in 0..2 {
                let fd = (ip[r] - i0[r]) / h;
                assert!(
                    (fd - j[r * 2 + c]).abs() <= 1e-4 * fd.abs().max(1e-9),
                    "d i{r} / d v{c}"
                );
            }
        }
    }
}
