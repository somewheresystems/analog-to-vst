//! VST3/CLAP wrapper around the circuit model. Controls come from `dsp::model::CONTROLS`.

use dsp::model::{self, CONTROLS};
use dsp::Processor;
use nih_plug::prelude::*;
use std::sync::Arc;

/// Plugins expose a fixed parameter list; unused slots are hidden.
const MAX_CONTROLS: usize = 8;

struct CircuitPlugin {
    params: Arc<CircuitParams>,
    processor: Option<Processor>,
    rate: f32,
}

#[derive(Params)]
struct CircuitParams {
    #[nested(array, group = "Controls")]
    controls: [ControlParam; MAX_CONTROLS],
}

#[derive(Params)]
struct ControlParam {
    #[id = "knob"]
    knob: FloatParam,
}

impl Default for CircuitParams {
    fn default() -> Self {
        Self {
            controls: std::array::from_fn(|i| {
                let (name, default) = CONTROLS.get(i).map_or(("Unused", 0.0), |c| (c.name, c.default));
                let knob = FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                    .with_value_to_string(Arc::new(|v| format!("{:.1}", v * 10.0)));
                ControlParam {
                    knob: if i < CONTROLS.len() { knob } else { knob.hide() },
                }
            }),
        }
    }
}

impl Default for CircuitPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(CircuitParams::default()),
            processor: None,
            rate: 0.0,
        }
    }
}

impl CircuitPlugin {
    fn rotations(&self) -> Vec<f32> {
        (0..CONTROLS.len())
            .map(|i| self.params.controls[i].knob.value())
            .collect()
    }
}

impl Plugin for CircuitPlugin {
    const NAME: &'static str = model::NAME;
    const VENDOR: &'static str = model::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(&mut self, _: &AudioIOLayout, config: &BufferConfig, context: &mut impl InitContext<Self>) -> bool {
        // Building the circuit solves its DC operating point; this runs off the audio thread.
        if self.processor.is_none() || self.rate != config.sample_rate {
            self.processor = Some(Processor::new(config.sample_rate as f64, &self.rotations()));
            self.rate = config.sample_rate;
        }
        if let Some(p) = &self.processor {
            context.set_latency_samples(p.latency() as u32);
        }
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(processor) = &mut self.processor else {
            return ProcessStatus::Normal;
        };
        for i in 0..CONTROLS.len() {
            processor.set_control(i, self.params.controls[i].knob.value());
        }
        if let [left, right] = buffer.as_slice() {
            processor.process(left, right);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for CircuitPlugin {
    const CLAP_ID: &'static str = model::CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for CircuitPlugin {
    const VST3_CLASS_ID: [u8; 16] = model::VST3_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

nih_export_clap!(CircuitPlugin);
nih_export_vst3!(CircuitPlugin);
