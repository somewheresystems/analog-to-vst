import shutil

import numpy as np
import pytest
from scipy import signal as sps

from analog_harness import analysis, compare, signals, spice

FS = 48_000


@pytest.fixture(scope="module")
def pack():
    return signals.build(FS)


@pytest.fixture(scope="module")
def identity(pack):
    stim, manifest = pack
    return analysis.analyze(stim, stim, manifest)


def test_identity_measures_flat_and_clean(identity):
    assert identity["delay_samples"] == 0 and identity["polarity"] == 1
    for s in identity["sweeps"]:
        assert np.max(np.abs(s["db"])) < 1e-3
    for s in identity["steps"]:
        assert abs(s["gain_db"]) < 1e-3
        assert max(s["harmonics_dbc"].values()) < -100
    assert identity["bursts"][0]["attack_s"] is None


def test_frequency_response_matches_analytic_filter(pack):
    stim, manifest = pack
    sos = sps.butter(2, 2000, fs=FS, output="sos")
    sweep = analysis.analyze(stim, sps.sosfilt(sos, stim), manifest)["sweeps"][0]
    _, h = sps.sosfreqz(sos, worN=sweep["freqs"], fs=FS)
    expected = 20 * np.log10(np.abs(h))
    usable = expected > -40
    assert np.max(np.abs(np.array(sweep["db"])[usable] - expected[usable])) < 0.05


def test_harmonics_match_polynomial_theory(pack):
    stim, manifest = pack
    a2, a3 = 0.05, 0.02
    steps = analysis.analyze(stim, stim + a2 * stim**2 + a3 * stim**3, manifest)["steps"]
    s = next(s for s in steps if s["freq"] == 1000 and s["level_db"] == -4)
    amp = signals.db(-4)
    fundamental = amp + 0.75 * a3 * amp**3
    assert s["harmonics_dbc"][2] == pytest.approx(20 * np.log10(a2 * amp**2 / 2 / fundamental), abs=0.01)
    assert s["harmonics_dbc"][3] == pytest.approx(20 * np.log10(a3 * amp**3 / 4 / fundamental), abs=0.01)


def test_attack_and_release_times(pack):
    stim, manifest = pack
    sec = next(s for s in manifest["sections"] if s["kind"] == "burst")
    t1, t2, _ = (np.cumsum(sec["segments_s"]) * FS).astype(int)
    tau_a, tau_r = 0.005, 0.05
    t = np.arange(sec["length"]) / FS
    g = np.ones(sec["length"])
    g[t1:t2] = 0.5 + 0.5 * np.exp(-t[: t2 - t1] / tau_a)
    g[t2:] = 1 - (1 - g[t2 - 1]) * np.exp(-t[: len(g) - t2] / tau_r)
    gain = np.ones_like(stim)
    gain[sec["start"] : sec["start"] + sec["length"]] = g

    # The 90% point of an exponential in dB, for a 6 dB change, is close to (not exactly) tau*ln(10).
    burst = analysis.analyze(stim, stim * gain, manifest)["bursts"][0]
    db_curve = 20 * np.log10(g)
    target_a = db_curve[t1 - 1] + 0.9 * (db_curve[t2 - 1] - db_curve[t1 - 1])
    expected_a = np.argmax(db_curve[t1:t2] <= target_a) / FS
    target_r = db_curve[t2 - 1] + 0.9 * (db_curve[-1] - db_curve[t2 - 1])
    expected_r = np.argmax(db_curve[t2:] >= target_r) / FS
    assert burst["attack_s"] == pytest.approx(expected_a, rel=0.03)
    assert burst["release_s"] == pytest.approx(expected_r, rel=0.03)


def test_latency_and_polarity_are_recovered(pack):
    stim, manifest = pack
    result = analysis.analyze(stim, -np.concatenate([np.zeros(37), stim]), manifest)
    assert (result["delay_samples"], result["polarity"]) == (37, -1)
    assert np.max(np.abs(result["sweeps"][2]["db"])) < 1e-3


def test_compare_passes_identity_and_flags_small_errors(pack, tmp_path):
    stim, manifest = pack
    assert compare.run(stim, manifest, stim, stim, tmp_path / "same")
    assert (tmp_path / "same" / "frequency_response.png").exists()

    assert not compare.run(stim, manifest, stim, stim * signals.db(0.2), tmp_path / "gain")
    failed = {
        c["name"]
        for c in compare.checks(
            analysis.analyze(stim, stim, manifest), analysis.analyze(stim, np.tanh(3 * stim) / 3, manifest), -100
        )
        if not c["passed"]
    }
    assert {"harmonics", "static gain"} <= failed


@pytest.mark.skipif(not shutil.which("ngspice"), reason="needs ngspice")
def test_spice_reference_matches_analytic_rc(tmp_path):
    """An RC low-pass through the SPICE runner matches its analytic response."""
    netlist = tmp_path / "rc.cir"
    netlist.write_text("* RC\n$SOURCE\nR1 in out $R\nC1 out 0 10n\n$ANALYSIS\n.end\n")
    sweep = signals.sweep(FS, 10, 0.45 * FS, 0.5)
    gap = np.zeros(FS // 2)
    stim = np.concatenate([gap, sweep, gap])
    manifest = {
        "sample_rate": FS,
        "sections": [{"kind": "sweep", "start": len(gap), "length": len(sweep), "f1": 10, "f2": 0.45 * FS}],
    }
    y = spice.render(netlist, stim, manifest, {"R": 1000})
    fr = analysis.frequency_response(stim, y, manifest["sections"][0], FS)
    f = np.array(fr["freqs"])
    expected = -10 * np.log10(1 + (2 * np.pi * f * 1000 * 10e-9) ** 2)
    assert np.max(np.abs(np.array(fr["db"]) - expected)) < 0.01
