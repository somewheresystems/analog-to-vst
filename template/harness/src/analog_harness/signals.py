"""The test-signal pack: one mono file of stimulus sections, plus a JSON manifest locating each."""

import json
from pathlib import Path

import numpy as np
import soundfile as sf

GAP_S = 0.5
SWEEP_LEVELS_DB = [-40, -30, -20, -12, -6, 0]
SWEEP_S = 4.0
STEP_FREQS = [100, 1000, 5000]
STEP_LEVELS_DB = list(range(-40, 1, 4))
STEP_S = 0.5
BURST_FREQ = 1000
BURST_LEVELS_DB = (-30, -6)
BURST_S = (1.0, 1.0, 2.0)
IMD_FREQS = (60, 7000)
IMD_LEVELS_DB = [-20, -6]
IMD_S = 1.0


def db(x: float) -> float:
    return 10 ** (x / 20)


def sweep(fs: int, f1: float, f2: float, seconds: float) -> np.ndarray:
    """Exponential sine sweep (Farina)."""
    t = np.arange(int(seconds * fs)) / fs
    r = np.log(f2 / f1)
    return np.sin(2 * np.pi * f1 * seconds / r * (np.exp(t * r / seconds) - 1))


def tone(fs: int, freq: float, seconds: float, level_db: float) -> np.ndarray:
    return db(level_db) * np.sin(2 * np.pi * freq * np.arange(int(seconds * fs)) / fs)


def pink(fs: int, seconds: float, rms_db: float, seed: int = 0) -> np.ndarray:
    n = int(seconds * fs)
    spectrum = np.fft.rfft(np.random.default_rng(seed).standard_normal(n))
    f = np.fft.rfftfreq(n, 1 / fs)
    spectrum[1:] /= np.sqrt(f[1:])
    spectrum[0] = 0
    x = np.fft.irfft(spectrum, n)
    return x * db(rms_db) / np.sqrt(np.mean(x**2))


def build(fs: int = 96_000, quick: bool = False) -> tuple[np.ndarray, dict]:
    """The full pack, or with `quick` a shorter one (fewer levels, same kinds of checks) for CI."""
    sweep_levels = SWEEP_LEVELS_DB[::2] if quick else SWEEP_LEVELS_DB
    step_levels = STEP_LEVELS_DB[::3] if quick else STEP_LEVELS_DB
    sections: list[dict] = []
    parts: list[np.ndarray] = [np.zeros(int(GAP_S * fs))]
    pos = len(parts[0])

    def add(kind: str, signal: np.ndarray, **params):
        nonlocal pos
        gap = np.zeros(int(GAP_S * fs))
        parts.extend([signal, gap])
        sections.append({"kind": kind, "start": pos, "length": len(signal), **params})
        pos += len(signal) + len(gap)

    add("sync", db(-12) * sweep(fs, 100, 10_000, 0.1))
    add("cal", tone(fs, 1000, 2.0, -18), freq=1000, level_db=-18)

    f1, f2 = 10.0, 0.45 * fs
    for level in sweep_levels:
        add("sweep", db(level) * sweep(fs, f1, f2, SWEEP_S), level_db=level, f1=f1, f2=f2)

    for freq in STEP_FREQS:
        for level in step_levels:
            add("step", tone(fs, freq, STEP_S, level), freq=freq, level_db=level)

    lo, hi = BURST_LEVELS_DB
    bursts = np.concatenate([tone(fs, BURST_FREQ, s, lv) for s, lv in zip(BURST_S, (lo, hi, lo))])
    # Phase-continuous: each segment is a whole number of cycles at 1 kHz.
    add("burst", bursts, freq=BURST_FREQ, levels_db=[lo, hi, lo], segments_s=list(BURST_S))

    for level in IMD_LEVELS_DB:
        lf, hf = IMD_FREQS
        x = 0.8 * tone(fs, lf, IMD_S, 0) + 0.2 * tone(fs, hf, IMD_S, 0)
        add("imd", db(level) * x / np.max(np.abs(x)), freqs=list(IMD_FREQS), level_db=level)

    if not quick:
        add("noise", pink(fs, 5.0, -20), rms_db=-20)

    signal = np.concatenate(parts).astype(np.float32)
    return signal, {"version": 1, "sample_rate": fs, "length": len(signal), "sections": sections}


def write(out_dir: Path, fs: int = 96_000, quick: bool = False) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    signal, manifest = build(fs, quick)
    sf.write(out_dir / "pack.wav", signal, fs, subtype="FLOAT")
    path = out_dir / "pack.json"
    path.write_text(json.dumps(manifest, indent=2))
    return path


def load(manifest_path: Path) -> tuple[np.ndarray, dict]:
    manifest = json.loads(manifest_path.read_text())
    signal, fs = sf.read(manifest_path.with_name("pack.wav"), dtype="float64")
    assert fs == manifest["sample_rate"]
    return signal, manifest
