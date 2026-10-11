"""Compares a model capture against a reference capture and writes a validation report."""

import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

from . import analysis

TOLERANCES = {
    "frequency_response_db": 0.1,
    "static_gain_db": 0.1,
    "harmonics_db": 1.0,
    "timing_relative": 0.05,
    "imd_db": 1.0,
    "burst_null_db": -40.0,
}
TIMING_FLOOR_S = 1e-3
# Harmonic and IMD products below this are treated as noise floor on both sides.
FLOOR_DBC = -90.0


def check(name: str, value: float, limit: float, detail: str = "") -> dict:
    return {"name": name, "value": value, "limit": limit, "passed": bool(value <= limit), "detail": detail}


def worst(pairs) -> tuple[float, str]:
    pairs = list(pairs)
    if not pairs:
        return 0.0, ""
    return max(pairs, key=lambda p: p[0])


def checks(ref: dict, dut: dict, burst_null_db: float) -> list[dict]:
    tol = TOLERANCES
    out = []

    fr = worst(
        (float(np.max(np.abs(np.subtract(r["db"], d["db"])))), f"sweep at {r['level_db']} dBFS")
        for r, d in zip(ref["sweeps"], dut["sweeps"])
    )
    out.append(check("frequency response", fr[0], tol["frequency_response_db"], fr[1]))

    gain = worst(
        (abs(r["gain_db"] - d["gain_db"]), f"{r['freq']} Hz at {r['level_db']} dBFS")
        for r, d in zip(ref["steps"], dut["steps"])
    )
    out.append(check("static gain", gain[0], tol["static_gain_db"], gain[1]))

    def harmonic_errors():
        for r, d in zip(ref["steps"], dut["steps"]):
            for k, rv in r["harmonics_dbc"].items():
                dv = d["harmonics_dbc"][k]
                if max(rv, dv) > FLOOR_DBC:
                    err = abs(max(rv, FLOOR_DBC) - max(dv, FLOOR_DBC))
                    yield err, f"H{k} of {r['freq']} Hz at {r['level_db']} dBFS"

    h = worst(harmonic_errors())
    out.append(check("harmonics", h[0], tol["harmonics_db"], h[1]))

    for key in ("attack_s", "release_s"):
        for r, d in zip(ref["bursts"], dut["bursts"]):
            rv, dv = r[key], d[key]
            if rv is None and dv is None:
                err, detail = 0.0, "no gain change in either"
            elif rv is None or dv is None:
                err, detail = float("inf"), f"reference {rv}, model {dv}"
            else:
                # Relative to the reference time, floored at 1 ms so instant responses compare
                # against 50 us rather than dividing by zero.
                err = abs(dv - rv) / max(rv, TIMING_FLOOR_S)
                detail = f"reference {rv * 1e3:.2f} ms, model {dv * 1e3:.2f} ms"
            out.append(check(key.removesuffix("_s"), err, tol["timing_relative"], detail))

    imd = worst(
        (abs(max(r["imd_db"], FLOOR_DBC) - max(d["imd_db"], FLOOR_DBC)), f"{r['level_db']} dBFS")
        for r, d in zip(ref["imd"], dut["imd"])
        if max(r["imd_db"], d["imd_db"]) > FLOOR_DBC
    )
    out.append(check("intermodulation", imd[0], tol["imd_db"], imd[1]))
    out.append(check("burst waveform null", burst_null_db, tol["burst_null_db"], "dB relative to reference"))
    return out


def burst_null(ref: np.ndarray, dut: np.ndarray, manifest: dict) -> float:
    sec = next(s for s in manifest["sections"] if s["kind"] == "burst")
    a, n = sec["start"], sec["length"]
    err = np.sqrt(np.mean((dut[a : a + n] - ref[a : a + n]) ** 2))
    return float(analysis.to_db(err) - analysis.to_db(np.sqrt(np.mean(ref[a : a + n] ** 2))))


def plot(ref: dict, dut: dict, out_dir: Path):
    fig, (ax, dx) = plt.subplots(2, 1, figsize=(10, 7), sharex=True, height_ratios=[3, 1])
    for i, (r, d) in enumerate(zip(ref["sweeps"], dut["sweeps"])):
        c = f"C{i}"
        ax.semilogx(r["freqs"], r["db"], color=c, label=f"{r['level_db']} dBFS")
        ax.semilogx(d["freqs"], d["db"], color=c, linestyle="--")
        dx.semilogx(r["freqs"], np.subtract(d["db"], r["db"]), color=c)
    ax.set(ylabel="dB", title="Frequency response (solid: reference, dashed: model)")
    ax.legend(fontsize=8)
    lim = TOLERANCES["frequency_response_db"]
    dx.axhspan(-lim, lim, color="0.9")
    dx.set(xlabel="Hz", ylabel="model - ref (dB)")
    fig.tight_layout()
    fig.savefig(out_dir / "frequency_response.png", dpi=120)
    plt.close(fig)

    freqs = sorted({s["freq"] for s in ref["steps"]})
    fig, axes = plt.subplots(2, len(freqs), figsize=(4 * len(freqs), 7), squeeze=False)
    for j, f in enumerate(freqs):
        r = [s for s in ref["steps"] if s["freq"] == f]
        d = [s for s in dut["steps"] if s["freq"] == f]
        levels = [s["level_db"] for s in r]
        axes[0, j].plot(levels, [s["gain_db"] for s in r], "k-", levels, [s["gain_db"] for s in d], "k--")
        axes[0, j].set(title=f"{f} Hz gain", xlabel="input dBFS", ylabel="dB")
        for k in r[0]["harmonics_dbc"]:
            c = f"C{int(k)}"
            axes[1, j].plot(levels, [s["harmonics_dbc"][k] for s in r], color=c, label=f"H{k}")
            axes[1, j].plot(levels, [s["harmonics_dbc"][k] for s in d], color=c, linestyle="--")
        axes[1, j].set(title=f"{f} Hz harmonics", xlabel="input dBFS", ylabel="dBc", ylim=(-130, 0))
        axes[1, j].legend(fontsize=8)
    fig.tight_layout()
    fig.savefig(out_dir / "harmonics.png", dpi=120)
    plt.close(fig)

    fig, ax = plt.subplots(figsize=(10, 4))
    for r, d in zip(ref["bursts"], dut["bursts"]):
        ax.plot(r["times"], r["gain_db"], "k-", label="reference")
        ax.plot(d["times"], d["gain_db"], "C1--", label="model")
    ax.set(title="Gain across level steps", xlabel="s", ylabel="dB")
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_dir / "dynamics.png", dpi=120)
    plt.close(fig)


def run(stimulus: np.ndarray, manifest: dict, ref_capture: np.ndarray, dut_capture: np.ndarray, out_dir: Path) -> bool:
    out_dir.mkdir(parents=True, exist_ok=True)
    ref = analysis.analyze(stimulus, ref_capture, manifest)
    dut = analysis.analyze(stimulus, dut_capture, manifest)
    ref_aligned = analysis.align(stimulus, ref_capture, manifest)[0] * ref["polarity"]
    dut_aligned = analysis.align(stimulus, dut_capture, manifest)[0] * dut["polarity"]
    results = checks(ref, dut, burst_null(ref_aligned, dut_aligned, manifest))
    passed = all(c["passed"] for c in results)

    (out_dir / "report.json").write_text(
        json.dumps({"passed": passed, "checks": results, "reference": ref, "model": dut}, indent=1)
    )
    plot(ref, dut, out_dir)
    lines = ["| Check | Result | Value | Limit | Worst case |", "|---|---|---|---|---|"]
    for c in results:
        lines.append(
            f"| {c['name']} | {'pass' if c['passed'] else 'FAIL'} | {c['value']:.4g} | {c['limit']:g} | {c['detail']} |"
        )
    summary = "\n".join(lines)
    (out_dir / "report.md").write_text(
        f"# Validation report: {'PASS' if passed else 'FAIL'}\n\n{summary}\n\n"
        "![](frequency_response.png)\n\n![](harmonics.png)\n\n![](dynamics.png)\n"
    )
    print(summary)
    return passed
