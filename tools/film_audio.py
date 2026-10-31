#!/usr/bin/env python3
"""Synthesize the soundtrack of the 30 s film (stdlib only). Usage: film_audio.py OUT.wav
Cues match src/film.rs: impacts at 17.0 s and 21.5 s, whiteout 25.6-26.5 s, title at 27.6 s."""
import math, random, struct, sys, wave

SR = 44100
N = SR * 30
buf = [0.0] * N
rng = random.Random(7)
TAU = 2 * math.pi


def env(t, dur, a, r):
    return min(1.0, t / a if a > 0 else 1.0) * min(1.0, (dur - t) / r if r > 0 else 1.0)


def tone(t0, dur, f, amp, a=0.05, r=0.3, harm=(1.0,), glide=None):
    """glide = (f_end): exponential sweep from f to f_end."""
    i0, n = int(t0 * SR), int(dur * SR)
    ph = 0.0
    for k in range(n):
        i = i0 + k
        if i >= N:
            break
        t = k / SR
        ff = f if glide is None else f * (glide / f) ** (t / dur)
        ph += TAU * ff / SR
        v = sum(w * math.sin(ph * (j + 1)) for j, w in enumerate(harm))
        buf[i] += v * amp * env(t, dur, a, r)


def noise(t0, dur, amp, a=0.05, r=0.3, lp=0.2, lp_end=None):
    i0, n = int(t0 * SR), int(dur * SR)
    y = 0.0
    for k in range(n):
        i = i0 + k
        if i >= N:
            break
        t = k / SR
        c = lp if lp_end is None else lp + (lp_end - lp) * (t / dur)
        y += c * (rng.uniform(-1, 1) - y)
        buf[i] += y * amp * env(t, dur, a, r)


def bell(t0, f, amp, dur=3.0):
    for ratio, w, d in ((1, 1.0, 1.0), (2.76, 0.5, 0.6), (5.4, 0.25, 0.35), (8.9, 0.1, 0.2)):
        tone(t0, dur * d, f * ratio, amp * w, a=0.002, r=dur * d * 0.95)


def boom(t0, amp=0.9):
    tone(t0, 1.6, 90, amp, a=0.002, r=1.5, glide=34)
    noise(t0, 0.9, amp * 0.6, a=0.001, r=0.85, lp=0.5, lp_end=0.03)


def kick(t0, amp=0.5):
    tone(t0, 0.28, 140, amp, a=0.001, r=0.27, glide=45)


def note(name):
    names = {"C": -9, "D": -7, "E": -5, "F": -4, "G": -2, "A": 0, "B": 2}
    n = names[name[0]] + (12 if name[1] == "#" else 0)
    off = 1 if name[1] == "#" else 0
    octv = int(name[1 + off])
    return 440.0 * 2 ** ((names[name[0]] + off + 12 * (octv - 4)) / 12)


# 0-13 s: night drone (A1/E2/A2), swelling, with wind
for f, a in ((55.0, 0.16), (82.4, 0.10), (110.0, 0.07)):
    tone(0, 30, f, a, a=3.0, r=3.0, harm=(1.0, 0.3))
noise(0.0, 10.0, 0.05, a=2.5, r=2.0, lp=0.02, lp_end=0.06)
# tilt-up shot: high airy fifths
tone(5.2, 4.0, note("E5"), 0.025, a=1.5, r=1.5)
tone(5.2, 4.0, note("A5"), 0.02, a=1.5, r=1.5)
# 9-13 s: heartbeat that speeds up, then the rings chime at 11.7 s
t, gap = 9.0, 0.95
while t < 13.0:
    kick(t, 0.35)
    kick(t + 0.18, 0.22)
    t += gap
    gap = max(0.4, gap * 0.9)
bell(11.7, note("A4"), 0.12)
bell(11.95, note("E5"), 0.09)
bell(12.2, note("A5"), 0.08)
# 13-17 s: riser
tone(13.0, 4.0, 180, 0.07, a=2.0, r=0.05, glide=2400, harm=(1.0, 0.5, 0.25))
tone(13.0, 4.0, 90, 0.10, a=2.0, r=0.05, glide=1200)
noise(13.0, 4.0, 0.12, a=3.0, r=0.05, lp=0.03, lp_end=0.6)
for k in range(24):  # rising pulse
    kick(13.0 + k * (4.0 / 24) * (1 - 0.0), 0.12 + 0.01 * k)
# 17.0 s impact, then 17-21.5 s driving arpeggio in A minor
boom(17.0, 1.0)
beat = 60 / 128 / 2
pat = ["A3", "E4", "A4", "C5", "E5", "C5", "A4", "E4"]
chords = [["A3", "A4"], ["F3", "F4"], ["C4", "C5"], ["G3", "G4"]]
t, k = 17.0, 0
while t < 21.5:
    ch = chords[int((t - 17.0) / (beat * 8)) % 4]
    base = note(ch[0]) / note("A3")
    f = note(pat[k % 8]) * base
    tone(t, beat * 1.4, f, 0.055, a=0.003, r=beat * 1.2, harm=(1.0, 0.5, 0.33, 0.2))
    if k % 4 == 0:
        kick(t, 0.55)
    if k % 2 == 1:
        noise(t, 0.05, 0.05, a=0.001, r=0.04, lp=0.7)
    t += beat
    k += 1
for ch, t0 in zip(chords, (17.0, 17.0 + beat * 8, 17.0 + beat * 16, 17.0 + beat * 24)):
    if t0 < 21.5:
        tone(t0, beat * 8, note(ch[0]) / 2, 0.13, a=0.05, r=0.3, harm=(1.0, 0.4))
# 21.5 s second hit, 21.5-26.5 s ascent: rising sweep, shimmering to the whiteout
boom(21.5, 1.0)
tone(21.5, 4.8, 220, 0.07, a=1.0, r=0.8, glide=3500, harm=(1.0, 0.4, 0.2))
tone(21.5, 4.8, 330, 0.05, a=1.0, r=0.8, glide=5200)
noise(21.5, 4.8, 0.14, a=1.5, r=0.9, lp=0.05, lp_end=0.8)
for k in range(10):
    bell(22.0 + k * 0.4, note(["A5", "C6", "E6", "A6", "E6"][k % 5]), 0.04, dur=1.5)
# 26.5 s: sunrise. A major pad, slow bells, last note rings out
noise(25.3, 1.4, 0.2, a=1.2, r=0.2, lp=0.02, lp_end=0.9)
for f in ("A3", "E4", "A4", "C#5", "E5"):
    tone(26.5, 3.5, note(f), 0.06, a=0.8, r=1.6, harm=(1.0, 0.3))
for t0, f in ((26.9, "A5"), (27.6, "E5"), (28.3, "C#6"), (28.9, "A5")):
    bell(t0, note(f), 0.1, dur=2.5)
bell(27.6, note("A3"), 0.15, dur=4.0)

peak = max(abs(v) for v in buf) or 1.0
scale = 0.9 / peak
fade = SR  # 1 s out
w = wave.open(sys.argv[1], "wb")
w.setnchannels(1)
w.setsampwidth(2)
w.setframerate(SR)
frames = bytearray()
for i, v in enumerate(buf):
    g = min(1.0, (N - i) / fade, i / (SR * 0.5))
    frames += struct.pack("<h", int(max(-1, min(1, v * scale * g)) * 32767))
w.writeframes(bytes(frames))
w.close()
