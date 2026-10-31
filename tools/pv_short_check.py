#!/usr/bin/env python3
"""Sync check (stdlib only). Usage: pv_short_check.py EVENTS.json SOUND.wav
For each key event, finds the first fast rise of 2 ms energy within +-120 ms of the event time
and prints the offset in ms. Pass = within one frame (1/fps). The flash is checked as silence + ring onset."""
import json, math, struct, sys, wave

ev = json.load(open(sys.argv[1]))
fps = ev["fps"]
w = wave.open(sys.argv[2], "rb")
sr, ch, n = w.getframerate(), w.getnchannels(), w.getnframes()
raw = struct.unpack("<%dh" % (n * ch), w.readframes(n))
mono = [sum(raw[i * ch:(i + 1) * ch]) / ch / 32768.0 for i in range(n)]
win = int(0.002 * sr)
cum = [0.0]
for v in mono:
    cum.append(cum[-1] + v * v)


def energy(a, b):
    a, b = max(0, a), min(n, b)
    return (cum[b] - cum[a]) / max(1, b - a)


def onset(t, span=0.12, before=None):
    """Time (s) of the first fast rise of 2 ms energy near t: energy above 25% of the window peak and 6x the
    energy just before it."""
    idx = [i for i in range(int((t - (before if before is not None else span)) * sr), int((t + span) * sr), 4) if 6 * win < i and i + win < n]
    e = {i: energy(i, i + win) for i in idx}
    peak = max(e.values())
    for i in idx:
        if e[i] > 0.25 * peak and e[i] > 6 * (energy(i - 6 * win, i - win) + 1e-9):
            return i / sr
    return t + span


def db(x):
    return 10 * math.log10(max(x, 1e-12))


frame_ms = 1000.0 / fps
bad = 0
print(f"frame = {frame_ms:.1f} ms")
for e in ev["events"]:
    k = e["type"]
    if k in ("clash", "slash", "split", "title_word", "title_settle", "pass", "eye_open", "hook_tick", "aftermath", "dash_start"):
        off = (onset(e["t"], 0.12, 0.03 if k == "pass" else None) - e["t"]) * 1000
        ok = abs(off) <= frame_ms
        bad += not ok
        print(f"{k:13s} t={e['t']:.3f}  onset offset {off:+6.1f} ms  {'ok' if ok else 'FAIL'}")
    elif k == "flash":
        s0, s1 = e["t"], e["t"] + e["silence"]
        quiet = db(energy(int((s0 + 0.004) * sr), int((s1 - 0.004) * sr)))
        before = db(energy(int((s0 - 0.1) * sr), int(s0 * sr)))
        off = (onset(s1, 0.12, 0.01) - s1) * 1000
        ok = abs(off) <= frame_ms and quiet < before - 40
        bad += not ok
        print(f"flash         t={s0:.3f}  silence {quiet:6.1f} dB vs {before:6.1f} dB before; ring onset offset {off:+6.1f} ms  {'ok' if ok else 'FAIL'}")
print("ALL OK" if not bad else f"{bad} FAIL")
sys.exit(1 if bad else 0)
