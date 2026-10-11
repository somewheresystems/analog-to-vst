# Diagnosing a failed validation

Read `validation/<setting>/report.md` and its plots. Match the symptom, fix the cause, re-run `./validate.sh`. Never loosen a tolerance to pass.

| Symptom | Likely cause | Fix |
|---|---|---|
| Everything off by a constant (static gain, all sweep levels) | A resistor value, pot formula or `VOLTS_PER_FULL_SCALE` differs between model and netlist | Diff `model.rs` against `circuit.cir` part by part; compare `render --plain` with the netlist expressions |
| Frequency response off at low frequencies only | A coupling or bias capacitor differs, or a bias node is treated as ideal in one and not the other | Check every capacitor value and where it connects; model supply decoupling the same way in both |
| Frequency response off only near 20 kHz, the same at every level | Frequency warping from the solver's integration, worse where the response falls steeply | Raise `MIN_CIRCUIT_RATE` in `dsp/src/oversample.rs` (error falls with the square of the rate) |
| Odd harmonics match, even harmonics missing or too low | An asymmetric part is missing or symmetric in the model: transistor modeled as linear, diode pair where the circuit has one diode, mismatched diodes | Model each device as in the netlist; use `diode` for single diodes |
| Harmonics off only at the lowest levels (around -90 dBc) | Numerical floor or junction capacitance | Usually inside tolerance; if not, check diode `cjo` and op-amp `gbw` match the netlist |
| Compression curve (static gain) off at high levels | Device model parameters differ (Is, N, Bf) or a series resistance is missing | Copy `.model` parameters exactly into `DiodeModel`/`BjtModel`; transistor `rb`/`rc`/`re` come from RB/RC/RE |
| Attack or release times off | A time-constant capacitor or resistor differs | Check the components in the detector or envelope path |
| Burst null poor but other checks pass | Small phase difference, often op-amp GBW or a missing pole | Match op-amp `a0`/`gbw`; check small capacitors (tens of pF) |
| SPICE fails or the operating point looks wrong | Floating node, wrong pin order, missing bias resistor | `uv run harness op ../reference/circuit.cir`; every node needs a DC path to ground |
| Model panics with "singular circuit matrix" | Floating node in the model netlist | Same as above, in `model.rs` |
| Rendering is very slow | Strongly coupled transistors use the joint solver | Expected for feedback circuits; validation still applies |
