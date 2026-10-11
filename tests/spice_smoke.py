"""Times the pack's first section (chirp, then silence) through ngspice. Fails if it is slow, which
catches regressions such as denormal slowdowns on x86."""

import sys
import tempfile
import time
from pathlib import Path
from string import Template

from analog_harness import signals, spice

root = Path(sys.argv[1])
fs = 96_000
stim, manifest = signals.build(fs, quick=True)
sec = manifest["sections"][0]
a, b = sec["start"], sec["start"] + sec["length"] + fs // 2
x = spice.upsample(0.2 * stim, fs)[a * 4 : b * 4]
spice.SECTION_TIMEOUT_S = 120
start = time.time()
with tempfile.TemporaryDirectory() as tmp:
    template = Template((root / "reference/circuit.cir").read_text())
    spice.run_section(template, {"GAIN": "10000", "LEVEL": "0.1"}, x, fs * 4, Path(tmp), "smoke")
print(f"first section: {time.time() - start:.1f} s for {len(x) / fs / 4:.2f} s of signal")
