"""Measurements of a capture of the test-signal pack."""

import numpy as np
from scipy import signal as sps

MAX_LAG_S = 1.0
HARMONICS = range(2, 6)
FLOOR_DB = -200.0


def to_db(x):
    return 20 * np.log10(np.maximum(np.abs(x), 10 ** (FLOOR_DB / 20)))


def align(stimulus: np.ndarray, capture: np.ndarray, manifest: dict) -> tuple[np.ndarray, int, int]:
    """Finds the capture's delay and polarity from the sync chirp; returns the aligned capture."""
    sync = next(s for s in manifest["sections"] if s["kind"] == "sync")
    a, n = sync["start"], sync["length"]
    search = capture[: a + n + int(MAX_LAG_S * manifest["sample_rate"])]
    xc = sps.correlate(search, stimulus[a : a + n], mode="valid", method="fft")
    peak = int(np.argmax(np.abs(xc)))
    delay = peak - a
    polarity = 1 if xc[peak] >= 0 else -1
    aligned = capture[delay:] if delay >= 0 else np.concatenate([np.zeros(-delay), capture])
    aligned = np.pad(aligned, (0, max(0, len(stimulus) - len(aligned))))[: len(stimulus)]
    return aligned, delay, polarity


def sine_fit(x: np.ndarray, fs: int, freqs) -> np.ndarray:
    """Least-squares amplitudes of sinusoids at `freqs` (plus DC). Exact for steady tones of any length."""
    t = np.arange(len(x)) / fs
    cols = [np.ones_like(t)]
    for f in freqs:
        cols += [np.cos(2 * np.pi * f * t), np.sin(2 * np.pi * f * t)]
    coef, *_ = np.linalg.lstsq(np.stack(cols, axis=1), x, rcond=None)
    return np.hypot(coef[1::2], coef[2::2])


def log_freqs(f2: float) -> np.ndarray:
    """1/12-octave points from 20 Hz to 20 kHz, kept inside the sweep's range."""
    f = 20 * 2 ** (np.arange(120) / 12)
    return f[f <= min(20_000, 0.9 * f2)]


def frequency_response(stim: np.ndarray, cap: np.ndarray, sec: dict, fs: int) -> dict:
    """Linear magnitude response via exponential-sweep deconvolution, harmonics windowed out."""
    a, n = sec["start"], sec["length"]
    f1, f2 = sec["f1"], sec["f2"]
    t = np.arange(n) / fs
    rate = np.log(f2 / f1)
    sweep = stim[a : a + n]
    inverse = sweep[::-1] * np.exp(-t * rate / (n / fs))

    tail = int(0.5 * fs)
    pre = int(min(0.5 * (n / fs) / rate * np.log(2), 0.02) * fs)
    window = np.ones(pre + tail)
    fade_in, fade_out = max(pre // 4, 1), tail // 10
    window[:fade_in] = np.hanning(2 * fade_in)[:fade_in]
    window[-fade_out:] = np.hanning(2 * fade_out)[fade_out:]

    def linear_ir(y):
        h = sps.fftconvolve(y[a : a + n + tail], inverse)
        peak = n - 1
        return h[peak - pre : peak + tail] * window

    nfft = 1 << 18
    H = np.fft.rfft(linear_ir(cap), nfft) / np.fft.rfft(linear_ir(stim), nfft)
    bins = np.fft.rfftfreq(nfft, 1 / fs)
    power = np.abs(H) ** 2
    freqs = log_freqs(f2)
    # 1/24-octave power smoothing; falls back to interpolation where a band is narrower than a bin.
    smoothed = []
    for fc in freqs:
        band = (bins >= fc * 2 ** (-1 / 48)) & (bins <= fc * 2 ** (1 / 48))
        smoothed.append(power[band].mean() if band.sum() >= 2 else np.interp(fc, bins, power))
    return {"freqs": freqs.tolist(), "db": (10 * np.log10(np.maximum(smoothed, 1e-12))).tolist()}


def step(cap: np.ndarray, sec: dict, fs: int) -> dict:
    """Gain at the fundamental and harmonics 2-5 (dB relative to the fundamental)."""
    a, n, f = sec["start"], sec["length"], sec["freq"]
    seg = cap[a + int(0.2 * fs) : a + n - int(0.05 * fs)]
    ks = [k for k in [1, *HARMONICS] if k * f < 0.45 * fs]
    amps = sine_fit(seg, fs, [k * f for k in ks])
    gain = to_db(amps[0]) - sec["level_db"]
    return {
        "freq": f,
        "level_db": sec["level_db"],
        "gain_db": float(gain),
        "harmonics_dbc": {k: float(to_db(amp) - to_db(amps[0])) for k, amp in zip(ks[1:], amps[1:])},
    }


def burst(stim: np.ndarray, cap: np.ndarray, sec: dict, fs: int) -> dict:
    """Gain over time across a level step, with 90% attack and release times.

    Envelopes come from the analytic signal of a zero-phase band-pass around the tone, so timing
    resolution is a fraction of a millisecond. Faster dynamics are checked by the waveform null in compare.
    """
    a, n, f = sec["start"], sec["length"], sec["freq"]
    sos = sps.butter(2, [f / 2, f * 2], btype="band", fs=fs, output="sos")

    def envelope(x):
        return np.abs(sps.hilbert(sps.sosfiltfilt(sos, x[a : a + n])))

    gains = to_db(envelope(cap)) - to_db(envelope(stim))
    times = np.arange(n) / fs

    def settle_time(start_s: float, end_s: float, settle_s: float) -> float | None:
        span = (times >= start_s) & (times < end_s)
        g, t = gains[span], times[span]
        before = gains[(times >= start_s - 0.1) & (times < start_s - 0.01)].mean()
        after = g[(t >= end_s - settle_s) & (t < end_s - 0.02)].mean()
        change = after - before
        if abs(change) < 0.5:
            return None
        reached = np.nonzero((g - before) / change >= 0.9)[0]
        return float(t[reached[0]] - start_s) if len(reached) else None

    t1, t2, t3 = np.cumsum(sec["segments_s"])
    hop = int(fs / 2000)
    return {
        "times": times[::hop].tolist(),
        "gain_db": gains[::hop].tolist(),
        "attack_s": settle_time(t1, t2, 0.1),
        "release_s": settle_time(t2, t3, 0.2),
    }


def imd(cap: np.ndarray, sec: dict, fs: int) -> dict:
    """SMPTE intermodulation: sidebands around the high tone relative to it, in dB."""
    a, n = sec["start"], sec["length"]
    lf, hf = sec["freqs"]
    seg = cap[a + int(0.2 * fs) : a + n - int(0.05 * fs)]
    sidebands = [hf + s * k * lf for k in (1, 2, 3) for s in (-1, 1)]
    amps = sine_fit(seg, fs, [lf, 2 * lf, 3 * lf, hf, *sidebands])
    return {"level_db": sec["level_db"], "imd_db": float(to_db(np.sqrt(np.sum(amps[4:] ** 2))) - to_db(amps[3]))}


def analyze(stimulus: np.ndarray, capture: np.ndarray, manifest: dict) -> dict:
    fs = manifest["sample_rate"]
    cap, delay, polarity = align(stimulus, capture, manifest)
    cap = cap * polarity
    out = {"delay_samples": delay, "polarity": polarity, "sweeps": [], "steps": [], "bursts": [], "imd": []}
    for sec in manifest["sections"]:
        match sec["kind"]:
            case "sweep":
                out["sweeps"].append({"level_db": sec["level_db"], **frequency_response(stimulus, cap, sec, fs)})
            case "step":
                out["steps"].append(step(cap, sec, fs))
            case "burst":
                out["bursts"].append(burst(stimulus, cap, sec, fs))
            case "imd":
                out["imd"].append(imd(cap, sec, fs))
    return out
