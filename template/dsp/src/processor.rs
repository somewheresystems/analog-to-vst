use crate::circuit::Circuit;
use crate::model;
use crate::netlist::Netlist;
use crate::oversample::{factor_for, Oversampler};

const MIN_POT_OHMS: f64 = 1.0;

/// Stereo processor: one circuit per channel, oversampled.
pub struct Processor {
    channels: Vec<(Circuit, Oversampler)>,
    pots: Vec<(&'static str, usize)>,
    controls: Vec<f64>,
}

impl Processor {
    /// Builds the circuit at its DC operating point for the given knob rotations. Allocates; call
    /// off the audio thread.
    pub fn new(sample_rate: f64, rotations: &[f32]) -> Self {
        let controls: Vec<f64> = model::CONTROLS
            .iter()
            .zip(rotations)
            .map(|(spec, &r)| spec.plain(r))
            .collect();
        let mut netlist = Netlist::default();
        model::circuit(&mut netlist);
        let pots = netlist.pots.clone();
        model::pots(&controls, &mut |name, ohms| {
            let index = pots
                .iter()
                .find(|p| p.0 == name)
                .unwrap_or_else(|| panic!("unknown pot {name}"))
                .1;
            if let crate::circuit::Element::Resistor(_, _, r) = &mut netlist.elements[index] {
                *r = ohms.max(MIN_POT_OHMS);
            }
        });
        let factor = factor_for(sample_rate);
        let circuit = Circuit::new(netlist.elements, (netlist.output, 0), sample_rate * factor as f64);
        let channels = vec![
            (circuit.clone(), Oversampler::new(sample_rate, factor)),
            (circuit, Oversampler::new(sample_rate, factor)),
        ];
        Self {
            channels,
            pots,
            controls,
        }
    }

    /// Sets a control by knob rotation. Allocation-free.
    pub fn set_control(&mut self, index: usize, rotation: f32) {
        let value = model::CONTROLS[index].plain(rotation);
        if self.controls[index] == value {
            return;
        }
        self.controls[index] = value;
        let (pots, channels) = (&self.pots, &mut self.channels);
        model::pots(&self.controls, &mut |name, ohms| {
            if let Some(&(_, index)) = pots.iter().find(|p| p.0 == name) {
                for (circuit, _) in channels.iter_mut() {
                    circuit.set_resistor(index, ohms.max(MIN_POT_OHMS));
                }
            }
        });
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let scale = model::VOLTS_PER_FULL_SCALE;
        for ((circuit, os), buf) in self.channels.iter_mut().zip([left, right]) {
            for x in buf.iter_mut() {
                *x = (os.process(*x as f64 * scale, |u| circuit.tick(u)) / scale) as f32;
            }
        }
    }

    /// Delay in samples from the oversampling filters.
    pub fn latency(&self) -> usize {
        self.channels[0].1.latency()
    }

    /// Control values in physical units, as substituted into the SPICE netlist.
    pub fn plain_controls(&self) -> &[f64] {
        &self.controls
    }
}
