"""Times the pack's first section (chirp then silence) through ngspice, with and without a
-180 dB noise floor on the input, for diagnosing slow SPICE on CI."""

import sys
import tempfile
import time
from pathlib import Path
from string import Template

import numpy as np

from analog_harness import signals, spice

root = Path(sys.argv[1])
fs = 96_000
stim, manifest = signals.build(fs, quick=True)
sec = manifest["sections"][0]
a, b = sec["start"], sec["start"] + sec["length"] + fs // 2
x = spice.upsample(0.2 * stim, fs)[a * 4 : b * 4]
template = Template((root / "reference/circuit.cir").read_text())
for label, floor in [("noise floor", 1e-9), ("plain", 0.0)]:
    xi = x + floor * np.random.default_rng(0).standard_normal(len(x))
    start = time.time()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            spice.SECTION_TIMEOUT_S = 120
            spice.run_section(template, {"GAIN": "10000", "LEVEL": "0.1"}, xi, fs * 4, Path(tmp), "smoke")
        print(f"{label}: {time.time() - start:.1f} s for {len(x) / fs / 4:.2f} s of signal", flush=True)
    except RuntimeError:
        print(f"{label}: over 120 s", flush=True)
