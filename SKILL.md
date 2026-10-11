---
name: analog-to-vst
description: Turn analog circuits into SPICE-validated VST3/CLAP plugins.
version: 0.1.0
author: Somewhere Systems (somewheresystems), Hermes Agent
license: MIT
platforms: [macos, linux]
prerequisites:
  commands: [git, cargo, uv, ngspice, python3]
metadata:
  hermes:
    editorial_name: Analog to VST
    editorial_description: Model a pedal, preamp, EQ or compressor from its schematic values, prove it matches a SPICE simulation, and build it as a VST3/CLAP plugin.
    tags: [Audio, DSP, VST, CLAP, SPICE, Plugins, Circuit Modeling]
    homepage: https://github.com/somewheresystems/analog-to-vst
---

# Analog to VST

Builds a plugin that behaves like a specific analog circuit, by simulating the circuit itself (nodal DK method, per sample, oversampled to 576 kHz or more) rather than approximating its sound. The same circuit is written twice, as a SPICE netlist and as a Rust model, and the model ships only when it matches the SPICE simulation within published tolerances. Works from published component values; no hardware needed.

## When to Use

- User wants a plugin of a pedal, preamp, EQ, compressor or other analog circuit
- User has a schematic, parts list or service manual and wants it modeled accurately
- User wants proof (measured error against a simulation) that a model matches its circuit

Don't use for: algorithmic effects with no circuit (digital reverbs, firmware delays), sample-based instruments, or "inspired by" sound design without a schematic.

## Prerequisites

`git`, Rust (`rustup`), `uv`, `ngspice` (XSPICE code models included, as in Homebrew's build), `python3`. Check with:

`terminal(command="for t in git cargo uv ngspice python3; do command -v $t >/dev/null || echo missing $t; done")`

Install on macOS: `brew install ngspice uv rustup && rustup-init -y`.

## Procedure

1. **Scaffold.** `terminal(command="git clone --depth 1 https://github.com/somewheresystems/analog-to-vst /tmp/analog-to-vst && python3 /tmp/analog-to-vst/scripts/new_project.py <project-dir> --name '<Plugin Name>'", timeout=120)`. Then `terminal(command="cargo test", workdir="<project-dir>", timeout=600)`.
   Done when: tests pass. The template contains a working example circuit; read `dsp/src/model.rs` and `reference/circuit.cir` side by side before changing either.

2. **Source the circuit.** Use `web_search`/`web_extract` for the schematic's topology and every component value (service manuals, published circuit analyses, parts lists). Write values from sources; never copy a schematic file into the project. List each source and every assumption (device models, op-amp parameters, source and load impedance, parts omitted) in the netlist header.
   Done when: every part has a value and a source, or is listed as an assumption.

3. **Write `reference/circuit.cir`.** Input node `in`, output node `out`, `$SOURCE` and `$ANALYSIS` placeholders, one `$NAME` placeholder per control (upper-cased control name, physical value). Pots are pairs of resistors with `{max(expr, 1)}` values. Use real SPICE device models (`.model`) and the `opamp` subcircuit for op-amps. Check bias: `terminal(command="uv run harness op ../reference/circuit.cir", workdir="<project-dir>/harness")`.
   Done when: the operating point is plausible (bias rails as designed, transistors forward-active, op-amp outputs at their bias, `out` at 0 V).

4. **Write `dsp/src/model.rs`** to match the netlist part for part with the `Netlist` helpers (`resistor`, `capacitor`, `inductor`, `pot`, `supply`, `diode`, `diode_pair`, `npn`, `pnp`, `opamp`). Set `NAME`, `CONTROLS` (taper per pot: `Audio` for log pots, `Linear` otherwise) and `pots()` with the same formulas as the netlist. Set a unique `CLAP_ID` and 16-byte `VST3_ID`.
   Done when: `cargo test` passes and `./target/release/render --plain` prints the values you expect for each control.

5. **List knob settings** in `settings.txt`: `default`, plus every control at both ends.
   Done when: each control is at 0 and at 1 in at least one setting.

6. **Validate.** `terminal(command="./validate.sh", workdir="<project-dir>", timeout=3600)`. Each setting runs a SPICE simulation (minutes) and writes `validation/<setting>/report.md` with plots. On FAIL, read the report and fix the cause with `references/diagnosing.md`; never loosen tolerances.
   Done when: `validation/summary.md` shows PASS for every setting.

7. **Build the plugin.** `terminal(command="./install.sh", workdir="<project-dir>", timeout=900)` builds VST3 and CLAP bundles into `target/bundled/` and installs them to the user's plugin folders (macOS). Measure CPU: `cargo run --release --example bench -p dsp 48000`.
   Done when: both bundles exist.

8. **Report** the summary table, the worst value of each check, CPU use, assumptions from step 2, and the plugin paths.

## Pitfalls

- **Supported parts:** resistors, capacitors, inductors, pots, DC supplies, diodes, NPN/PNP transistors (Gummel-Poon, no junction capacitance), single-pole op-amps without rail clipping. Not yet: transformers, JFETs/MOSFETs, tubes, optical cells, op-amp clipping at the rails. Extending the solver: `references/method.md`. Tell the user before modeling a circuit that depends on unsupported parts.
- **Model and netlist must stay identical.** Junction capacitance is a fixed `cjo` in the model; that is the main expected difference from SPICE and is usually well inside tolerance.
- **Frequency response failing only near 20 kHz** at every level is frequency warping, not a circuit error; see `references/diagnosing.md`.
- **CPU is high by plugin standards** (one stereo TS808-class circuit is roughly 15-25% of a core). This is the cost of full-circuit accuracy.
- **Legal:** model circuits from facts (values, topology); never copy schematic files, firmware, impulse responses or faceplate designs; use brand names only to describe what was modeled, never as the plugin name. See `references/legal.md`.

## Verification

- `validation/summary.md` lists PASS for every setting in `settings.txt`, and each `report.md` shows its checks.
- `cargo test` and `uv run pytest -q` (in `harness/`) pass.
- `target/bundled/` contains `<name>.vst3` and `<name>.clap`.
