# Analog to VST

An agent skill (Hermes Agent, Claude Code) and project template for turning an analog circuit (pedal, preamp, EQ, compressor) into a VST3/CLAP plugin that provably matches the circuit.

The circuit is written twice from its published component values: as a SPICE netlist, simulated by ngspice, and as a real-time circuit model in Rust (nodal DK method, oversampled to at least 576 kHz). Both get the same test signals; the plugin ships only when it matches SPICE within tight tolerances on frequency response, compression, harmonics, dynamics and a sample-by-sample null. No hardware or captured audio needed.

- **Not a coder?** Read [GUIDE.md](GUIDE.md): plain-language steps from installing tools to loading the plugin in your DAW.
- **Agent instructions:** [SKILL.md](SKILL.md)
- **How it works:** [references/method.md](references/method.md), failures and fixes in [references/diagnosing.md](references/diagnosing.md), legal lines in [references/legal.md](references/legal.md)

## Install the skill

```bash
# Hermes Agent
hermes skills install somewheresystems/analog-to-vst

# Claude Code
git clone --depth 1 https://github.com/somewheresystems/analog-to-vst ~/.claude/skills/analog-to-vst
```

Then ask your agent: *"Use the analog-to-vst skill to make a plugin of the [circuit], from [schematic link or parts list]."*

## Use the template directly

```bash
git clone --depth 1 https://github.com/somewheresystems/analog-to-vst
python3 analog-to-vst/scripts/new_project.py my-plugin --name "My Plugin"
cd my-plugin
./validate.sh    # SPICE vs model at every knob setting in settings.txt
./install.sh     # build VST3 + CLAP and install (macOS)
```

Needs `git`, Rust, `uv` and `ngspice` (`brew install git uv ngspice rustup && rustup-init -y`).

| Path in a project | What it is |
|---|---|
| `reference/circuit.cir` | SPICE netlist of the circuit |
| `dsp/src/model.rs` | The same circuit for the plugin: parts, controls, pot formulas |
| `dsp/src/circuit.rs` | Circuit solver (resistors, capacitors, inductors, pots, supplies, diodes, NPN/PNP, op-amps) |
| `settings.txt` | Knob settings to validate |
| `harness/` | Test signals, SPICE runner, measurements, reports |
| `validate.sh`, `install.sh` | One-command validation and installation |

The template ships with a working example (op-amp gain stage into a diode clipper), validated against SPICE in CI. Its reports: [docs/example-validation](docs/example-validation/summary.md).

## License

MIT. nih-plug's VST3 support uses GPLv3 bindings, so distributed VST3 binaries are GPLv3; CLAP binaries can stay MIT.
