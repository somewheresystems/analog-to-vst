# Turn an analog circuit into a plugin: step-by-step guide

This guide takes you from "I have a pedal (or preamp, EQ, compressor) I want as a plugin" to a working VST3/CLAP plugin, with a report that proves it behaves like the real circuit. You don't need to write code: an AI coding agent does that, using the instructions in this repository. You copy and paste commands, answer the agent's questions, and check the results.

**What you end up with**

- A plugin file you can load in Ableton Live, Reaper, Bitwig, Cubase, Studio One or any other host that supports VST3 or CLAP. (Logic Pro needs Audio Units, which this doesn't make.)
- A validation report showing, in numbers and graphs, how closely the plugin matches a simulation of the original circuit.

**How it works, in one paragraph.** The agent writes down the circuit twice from its published component values: once for SPICE, the standard circuit simulator engineers use, and once as the plugin's code. Both are fed the same test sounds. If the plugin's output matches SPICE's within tight limits (about 1% in level), it passes. If not, the agent finds the difference and fixes it. No hardware is needed, and no sound is copied from anywhere: the plugin computes what the circuit's parts do.

**Time:** about 30 minutes of setup the first time, then 1 to 3 hours per circuit, mostly waiting for simulations.

---

## Step 1: What you need

- A Mac (Apple Silicon or Intel) with about 10 GB of free disk space. Linux also works if you install the tools with Homebrew (Ubuntu's own ngspice package is too old and far too slow); plugin installation in step 6 is Mac-only.
- An AI coding agent that can run commands on your computer: [Hermes Agent](https://github.com/NousResearch/hermes-agent) or [Claude Code](https://claude.com/claude-code).
- Information about the circuit, ideally one of:
  - the name of a well-documented circuit (for example "TS808 overdrive" or "Fuzz Face, silicon version"),
  - a link to a schematic or a published circuit analysis,
  - a parts list (every resistor, capacitor, transistor and pot value).

## Step 2: Install the tools

Open **Terminal** (press Cmd+Space, type "Terminal", press Return). Paste each line below, press Return, and wait for it to finish before the next. If it asks for your password, type it (nothing appears as you type) and press Return.

1. Apple's developer tools (a window pops up; click Install):
   ```
   xcode-select --install
   ```
2. Homebrew, a tool installer (skip if you already have it):
   ```
   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
   ```
   When it finishes, it prints two or three lines under "Next steps". Paste and run those too.
3. The tools this project uses:
   ```
   brew install git uv ngspice rustup
   rustup-init -y
   ```
4. Close Terminal and open it again, then check everything is there:
   ```
   for t in git cargo uv ngspice python3; do command -v $t >/dev/null && echo "ok $t" || echo "MISSING $t"; done
   ```
   You should see five lines starting with `ok`. If anything says `MISSING`, repeat step 3.

## Step 3: Give your agent the skill

The "skill" is the instruction sheet the agent follows.

- **Hermes Agent:**
  ```
  hermes skills install somewheresystems/analog-to-vst/skills/analog-to-vst
  ```
- **Claude Code:**
  ```
  git clone --depth 1 https://github.com/somewheresystems/analog-to-vst ~/analog-to-vst
  mkdir -p ~/.claude/skills && cp -R ~/analog-to-vst/skills/analog-to-vst ~/.claude/skills/
  ```

## Step 4: Ask the agent to build the plugin

1. Make a folder for your projects and start the agent in it:
   ```
   mkdir -p ~/Plugins && cd ~/Plugins
   hermes        # or: claude
   ```
2. Paste this message, filling in the brackets:

   > Use the analog-to-vst skill to make a plugin of the **[circuit name]**. Here is what I have about it: **[link to schematic / parts list / "find published values"]**. Call the plugin **[name you want, not a brand name]**. Validate it at every knob extreme, fix anything that fails, then install the plugin and show me the validation summary.

3. Let it work. It will:
   - look up and write down the circuit's component values (it may ask you to confirm values it can't find),
   - write the simulation and the plugin code,
   - run the simulations (each knob setting takes several minutes; this is the slow part),
   - compare, fix and repeat until everything passes,
   - build and install the plugin.

   If it asks for permission to run commands, allow it: everything happens inside your project folder.

## Step 5: Check the results

In your project folder, open `validation/summary.md`. Each knob setting shows **PASS** or **FAIL**. Every setting should be PASS before you trust the plugin.

For detail, open `validation/<setting>/report.md`. Each row is one test:

| Test | What it means | Limit | In percent |
|---|---|---|---|
| frequency response | Bass-to-treble balance at six input levels | within 0.1 dB | about 1.2% |
| static gain | How much it compresses or boosts at each input level | within 0.1 dB | about 1.2% |
| harmonics | The distortion's character (overtone levels) | within 1 dB | about 12% of each overtone |
| attack / release | How fast it reacts to level changes | within 5% | 5% |
| intermodulation | How two notes interact when distorted | within 1 dB | about 12% |
| burst waveform null | Plugin output minus simulation, sample by sample | -40 dB or better | difference under 1% of the signal |

The graphs show the simulation as solid lines and the plugin as dashed lines; on a passing report they sit on top of each other.

**What "matches the simulation" does and doesn't mean.** The simulation is of the circuit as designed, with standard part models. A specific vintage unit can differ from that through part tolerances and aging. The report says exactly what was compared.

## Step 6: Use the plugin

The agent installs the plugin into your Mac's plugin folders. In your DAW, rescan plugins (often in Preferences, under Plug-ins), then find it by the name you chose. The files are also in your project folder under `target/bundled/`.

It uses more CPU than typical plugins (often 15 to 25% of one core per instance), because it computes the whole circuit at a very high internal sample rate. If your computer struggles, freeze or bounce the track.

## Without an agent

If you're comfortable editing text files:

1. `git clone --depth 1 https://github.com/somewheresystems/analog-to-vst && python3 analog-to-vst/scripts/new_project.py my-plugin --name "My Plugin"`
2. In `my-plugin`, edit `reference/circuit.cir` (the SPICE netlist) and `dsp/src/model.rs` (the same circuit for the plugin). The example circuit already in both files shows the format; keep the two identical.
3. List knob settings to test in `settings.txt`.
4. Run `./validate.sh`, read the reports, fix differences (`skills/analog-to-vst/references/diagnosing.md` lists causes) and repeat until all pass.
5. Run `./install.sh`.

## Troubleshooting

| Problem | What to do |
|---|---|
| `command not found` for cargo, uv or ngspice | Close and reopen Terminal; if it persists, repeat step 2.3 |
| A setting keeps failing | Ask the agent to read the report and the skill's `references/diagnosing.md`, and to check every component value against the source |
| The agent says a part is unsupported (transformer, tube, JFET, optical cell) | The circuit needs a solver extension first; the agent can describe what's involved |
| Plugin doesn't appear in the DAW | Rescan plugins; check `~/Library/Audio/Plug-Ins/VST3`; Logic needs Audio Units (not supported) |
| High CPU | Expected; bounce or freeze tracks |

## Before you share or sell a plugin

- Name it yourself. Brand and model names (for example "Tube Screamer") are trademarks: say "modeled after" in the description, never use them as the plugin's name.
- Don't copy schematic images, manuals, impulse responses or faceplate designs into what you distribute. Component values and topologies are facts and fine to model.
- VST3 builds are distributed under GPLv3 (because of the VST3 library this uses); CLAP builds can stay MIT. More in `skills/analog-to-vst/references/legal.md`.
