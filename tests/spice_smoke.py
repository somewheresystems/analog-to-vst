"""Runs one short section of the example circuit through ngspice and prints timing, for diagnosing CI."""

import sys
import tempfile
import time
from pathlib import Path
from string import Template

import numpy as np

from analog_harness import signals, spice

root = Path(sys.argv[1])
fs = 96_000
x = spice.upsample(0.2 * signals.tone(fs, 1000, 0.05, -6), fs)
start = time.time()
with tempfile.TemporaryDirectory() as tmp:
    y = spice.run_section(
        Template((root / "reference/circuit.cir").read_text()),
        {"GAIN": "50000", "LEVEL": "0.1"},
        x,
        fs * spice.OVERSAMPLE,
        Path(tmp),
        "smoke",
    )
print(f"50 ms section: {time.time() - start:.1f} s, peak {np.max(np.abs(y)):.4f} V")
