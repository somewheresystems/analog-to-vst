//! CPU use per stereo instance: `cargo run --release --example bench -p dsp [sample rate]`.

use dsp::model::CONTROLS;
use dsp::Processor;
use std::time::Instant;

fn main() {
    let rate: f64 = std::env::args().nth(1).map_or(48_000.0, |r| r.parse().unwrap());
    let defaults: Vec<f32> = CONTROLS.iter().map(|c| c.default).collect();
    let mut processor = Processor::new(rate, &defaults);
    let seconds = 5.0;
    let n = (rate * seconds) as usize;
    let tone: Vec<f32> = (0..n)
        .map(|i| 0.3 * (i as f64 * 440.0 * std::f64::consts::TAU / rate).sin() as f32)
        .collect();
    let (mut l, mut r) = (tone.clone(), tone);
    let start = Instant::now();
    for (lb, rb) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
        processor.process(lb, rb);
    }
    let cpu = start.elapsed().as_secs_f64() / seconds * 100.0;
    println!(
        "{} at {rate} Hz: {cpu:.1}% of one core per stereo instance",
        dsp::model::NAME
    );
}
