//! Circuit-level audio model: a SPICE-style netlist solved per sample (nodal DK method),
//! oversampled so the response holds to 20 kHz.

mod circuit;
pub mod control;
pub mod model;
pub mod netlist;
mod oversample;
mod processor;

pub use control::{ControlSpec, Taper};
pub use processor::Processor;
