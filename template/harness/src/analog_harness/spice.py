"""Renders the test-signal pack through a SPICE netlist with ngspice, producing a reference capture.

The pack is upsampled 4x, split into sections (each with its trailing gap) that run in parallel from
the circuit's DC operating point, then stitched and decimated. Sections are separated by silence
longer than the circuit's settling time, so this matches one continuous run.
"""

import multiprocessing
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from string import Template

import numpy as np
from scipy import signal as sps

OVERSAMPLE = 4
SECTION_TIMEOUT_S = 900
NOISE_FLOOR_V = 1e-9
PASSBAND_HZ = 20_000
ATTENUATION_DB = 120


def oversampling_filter(fs: int) -> np.ndarray:
    """Kaiser low-pass, same design as crates/oc-dsp/src/oversample.rs.

    Flat to 20 kHz; attenuated by 120 dB from fs - 20 kHz, so nothing aliases into the audio band.
    Length is rounded up to 4k + 1 taps so the up/down round trip delays by a whole base sample count.
    """
    rate = fs * OVERSAMPLE
    width = 2 * np.pi * (fs - 2 * PASSBAND_HZ) / rate
    taps = int(np.ceil((ATTENUATION_DB - 7.95) / (2.285 * width))) + 1
    taps += (1 - taps) % OVERSAMPLE
    beta = 0.1102 * (ATTENUATION_DB - 8.7)
    return sps.firwin(taps, fs / 2, window=("kaiser", beta), fs=rate)


def upsample(x: np.ndarray, fs: int) -> np.ndarray:
    h = oversampling_filter(fs)
    delay = (len(h) - 1) // 2
    return sps.upfirdn(h * OVERSAMPLE, x, up=OVERSAMPLE)[delay : delay + len(x) * OVERSAMPLE]


def downsample(x: np.ndarray, fs: int) -> np.ndarray:
    h = oversampling_filter(fs)
    delay = (len(h) - 1) // 2 // OVERSAMPLE
    return sps.upfirdn(h, x, down=OVERSAMPLE)[delay : delay + len(x) // OVERSAMPLE]


def interpolation_compensation(taps: int = 9) -> np.ndarray:
    """FIR that cancels the sinc^2 response of filesource's linear interpolation below base Nyquist."""
    m = taps // 2
    w = np.linspace(0, 2 * np.pi * 0.5 / OVERSAMPLE, 512)
    target = 1 / np.sinc(w / (2 * np.pi)) ** 2
    basis = np.column_stack([np.ones_like(w)] + [2 * np.cos(k * w) for k in range(1, m + 1)])
    c = np.linalg.lstsq(basis, target, rcond=None)[0]
    return np.concatenate([c[:0:-1], c])


def read_raw(path: Path, vector: str) -> np.ndarray:
    """Reads one real vector from an ngspice binary rawfile."""
    data = path.read_bytes()
    marker = b"Binary:\n"
    header = data[: data.index(marker)].decode()
    lines = header.splitlines()
    nvars = int(next(ln for ln in lines if ln.startswith("No. Variables:")).split(":")[1])
    npoints = int(next(ln for ln in lines if ln.startswith("No. Points:")).split(":")[1])
    start = lines.index("Variables:") + 1
    names = [lines[start + i].split()[1].lower() for i in range(nvars)]
    values = np.frombuffer(data, dtype="<f8", offset=data.index(marker) + len(marker), count=nvars * npoints)
    return values.reshape(npoints, nvars)[:, names.index(vector.lower())].copy()


def run_section(template: Template, params: dict, x: np.ndarray, rate: int, work: Path, name: str) -> np.ndarray:
    src = work / f"{name}.txt"
    # filesource interpolates linearly between points (pre-compensated here) and reads past its
    # last point at the final timestep (padded beyond the stop time).
    # A -180 dB noise floor keeps decaying currents out of the denormal range in silent gaps, which
    # x86 CPUs process up to ~100x slower (measured: a 0.6 s section took 12.7 s with it, >120 s without).
    x = x + NOISE_FLOOR_V * np.random.default_rng(0).standard_normal(len(x))
    padded = np.concatenate([np.convolve(x, interpolation_compensation(), mode="same"), np.zeros(16)])
    np.savetxt(src, np.column_stack([np.arange(len(padded)) / rate, padded]), fmt="%.10e")
    step = 1 / rate
    cir, raw = work / f"{name}.cir", work / f"{name}.raw"
    netlist = template.substitute(
        params,
        SOURCE=f'A_IN %v([in]) insrc\n.model insrc filesource (file="{src}" amploffset=[0] amplscale=[1] '
        "timeoffset=0 timescale=1 timerelative=false amplstep=false)",
        # Tight tolerances and a quarter-sample maximum step keep output interpolation error
        # below 0.002 dB at 20 kHz.
        ANALYSIS=".options interp reltol=1e-6 abstol=1e-15 vntol=1e-9\n.save v(out)\n"
        f".tran {step:.12e} {(len(x) - 1) * step:.12e} 0 {step / 4:.12e}\n"
        f".control\nset noaskquit\nset filetype=binary\nrun\nwrite {raw} v(out)\nquit\n.endc",
    )
    cir.write_text(netlist)
    try:
        result = subprocess.run(
            ["ngspice", "-b", str(cir)],
            check=False,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=SECTION_TIMEOUT_S,
        )
    except subprocess.TimeoutExpired as e:
        tail = (e.stdout or b"")[-2000:]
        raise RuntimeError(f"ngspice timed out on {name} after {SECTION_TIMEOUT_S} s:\n{tail!r}") from e
    y = read_raw(raw, "v(out)") if raw.exists() else np.empty(0)
    if result.returncode != 0 or len(y) != len(x):
        raise RuntimeError(f"ngspice failed on {name} (exit {result.returncode}):\n{result.stdout[-2000:]}")
    return y


def operating_point(netlist: Path, params: dict) -> str:
    """ngspice's DC operating point for the netlist with the input at 0 V."""
    text = Template(netlist.read_text()).substitute(
        params, SOURCE="VIN in 0 DC 0", ANALYSIS=".control\nset noaskquit\nop\nprint all\nquit\n.endc"
    )
    with tempfile.TemporaryDirectory() as tmp:
        cir = Path(tmp) / "op.cir"
        cir.write_text(text)
        out = subprocess.run(
            ["ngspice", "-b", str(cir)], check=False, stdin=subprocess.DEVNULL, capture_output=True, text=True
        ).stdout
    return "\n".join(line for line in out.splitlines() if " = " in line)


def render(netlist: Path, stimulus: np.ndarray, manifest: dict, params: dict, jobs: int | None = None) -> np.ndarray:
    fs = manifest["sample_rate"]
    rate = fs * OVERSAMPLE
    up = upsample(stimulus, fs)
    template = Template(netlist.read_text())
    gap = int(0.5 * fs)
    out = np.zeros_like(up)
    jobs = jobs or int(os.environ.get("HARNESS_JOBS", "0")) or os.cpu_count() or 1
    spans = [
        (sec["start"] * OVERSAMPLE, min((sec["start"] + sec["length"] + gap) * OVERSAMPLE, len(up)))
        for sec in manifest["sections"]
    ]
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        args = [(template, params, up[a:b], rate, work, f"s{i:03d}") for i, (a, b) in enumerate(spans)]
        if jobs == 1:
            results = (run_section(*x) for x in args)
            pool = None
        else:
            # Spawned, not forked: forking after NumPy starts its thread pool can deadlock on Linux.
            pool = ProcessPoolExecutor(jobs, mp_context=multiprocessing.get_context("spawn"))
            results = (f.result() for f in [pool.submit(run_section, *x) for x in args])
        for k, ((a, b), y) in enumerate(zip(spans, results), 1):
            out[a:b] = y
            print(f"  section {k}/{len(spans)} done", file=sys.stderr, flush=True)
        if pool:
            pool.shutdown()
    return downsample(out, fs)
