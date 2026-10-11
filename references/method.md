# Method

## Two descriptions of one circuit

- `reference/circuit.cir`: SPICE netlist, simulated by ngspice. This is the ground truth when no hardware is available.
- `dsp/src/model.rs`: the same netlist in Rust, solved in real time.

Both use the same device models and assumptions, so any difference is an error in one of them or the solver's discretization, and the validation report measures it.

## Solver (dsp/src/circuit.rs)

Nodal DK method (Holters and Zölzer, DAFx 2015):

1. Modified nodal analysis over all elements. Capacitors and inductors become trapezoidal companions (a conductance plus a history state).
2. The linear part reduces to a state-space system; only the nonlinear devices' port voltages need solving each sample.
3. Each device (diode, transistor) is solved with Newton's method from a linear extrapolation of the last two samples. Devices are ordered along their dependencies; weak feedback (shared bias nodes) uses block Gauss-Seidel, strong feedback a joint Newton solve.
4. The circuit starts at its DC operating point, found with a SPICE-style Newton solve with source stepping.

The circuit runs at no less than 576 kHz (`MIN_CIRCUIT_RATE`), because trapezoidal integration evaluates the response at a frequency stretched by about (pi f / rate)^2 / 3; at 576 kHz the error at 20 kHz stays under about 0.05 dB on steep responses.

## Validation (harness/)

`validate.sh` plays one test-signal pack through both the SPICE netlist and the model:

| Section | Measures | Tolerance |
|---|---|---|
| Exponential sweeps at six levels | Frequency response | ±0.1 dB, 20 Hz to 20 kHz |
| Stepped tones, 100 Hz / 1 kHz / 5 kHz, -40 to 0 dBFS | Compression curve, harmonics 2 to 5 | ±0.1 dB, ±1 dB (above -90 dBc) |
| 1 kHz level steps | Attack and release (90%) | ±5% |
| 60 Hz + 7 kHz | Intermodulation | ±1 dB |
| Bursts | Waveform null | -40 dB or better |

The SPICE runner upsamples the pack 4x, simulates sections in parallel from the DC operating point, compensates ngspice's linear input interpolation and limits the time step to a quarter sample; it adds under 0.002 dB of its own error at 20 kHz.

## Extending the solver

Add a variant to `Element` and, for a nonlinear part, to `Device` with `ports()`, `eval()` (port currents and Jacobian) and `converged()`. Mirror the SPICE device equations exactly, add a finite-difference Jacobian test like `npn_jacobian_matches_finite_differences`, and a `Netlist` helper. Candidates: JFET (Shichman-Hodges), triode (Koren), transformers (coupled inductors), op-amp output clipping.
