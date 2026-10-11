//! Renders a WAV file through the circuit model, using the same code as the plugin.

use dsp::model::CONTROLS;
use dsp::Processor;
use std::process::ExitCode;

const USAGE: &str = "usage: render <in.wav> <out.wav> [control=rotation ...]
       render --plain [control=rotation ...]   print control values in physical units
       render --list                           list controls
  rotation is a knob position from 0 to 1";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--list") => {
            for c in CONTROLS {
                println!("{}\tdefault {}", c.name.to_lowercase(), c.default);
            }
            Ok(())
        }
        Some("--plain") => rotations(&args[1..]).map(|r| {
            for (spec, rot) in CONTROLS.iter().zip(r) {
                println!("{}={}", spec.name.to_uppercase(), spec.plain(rot));
            }
            // Read by the harness to drive SPICE at the model's signal level.
            println!("_VOLTS_PER_FULL_SCALE={}", dsp::model::VOLTS_PER_FULL_SCALE);
        }),
        _ => run(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn rotations(args: &[String]) -> Result<Vec<f32>, String> {
    let mut r: Vec<f32> = CONTROLS.iter().map(|c| c.default).collect();
    for a in args {
        let (key, value) = a.split_once('=').ok_or(format!("bad control {a}"))?;
        let i = CONTROLS
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(key))
            .ok_or(format!("unknown control {key}"))?;
        r[i] = value.parse().map_err(|_| format!("bad value {value}"))?;
    }
    Ok(r)
}

fn run(args: &[String]) -> Result<(), String> {
    let [input, output, controls @ ..] = args else {
        return Err("missing arguments".into());
    };
    let rot = rotations(controls)?;
    let mut reader = hound::WavReader::open(input).map_err(|e| format!("{input}: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels > 2 {
        return Err("only mono and stereo files are supported".into());
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect()
        }
    }
    .map_err(|e| e.to_string())?;
    let mut left: Vec<f32> = samples.iter().step_by(channels).copied().collect();
    let mut right: Vec<f32> = samples.iter().skip(channels - 1).step_by(channels).copied().collect();

    let mut processor = Processor::new(spec.sample_rate as f64, &rot);
    // Run past the end by the latency so the output lines up with the input.
    let latency = processor.latency();
    left.extend(std::iter::repeat_n(0.0, latency));
    right.extend(std::iter::repeat_n(0.0, latency));
    processor.process(&mut left, &mut right);

    let out_spec = hound::WavSpec {
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
        ..spec
    };
    let mut writer = hound::WavWriter::create(output, out_spec).map_err(|e| format!("{output}: {e}"))?;
    for i in latency..left.len() {
        writer.write_sample(left[i]).map_err(|e| e.to_string())?;
        if channels == 2 {
            writer.write_sample(right[i]).map_err(|e| e.to_string())?;
        }
    }
    writer.finalize().map_err(|e| e.to_string())
}
