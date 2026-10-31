#!/usr/bin/env python3
"""Soundtrack of the ONE CUT teaser (stdlib only). Usage: pv_audio.py OUT.wav
120 BPM: a beat is 0.5 s, a bar 2 s. Cuts, impacts and titles match src/pv.rs."""
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


B = 0.5  # beat
def hit(t0, amp=0.5):
    """Cut accent: short noise burst over a low thump."""
    tone(t0, 0.18, 120, amp, a=0.001, r=0.17, glide=50)
    noise(t0, 0.08, amp * 0.5, a=0.001, r=0.07, lp=0.6)

def clang(t0, amp=0.3):
    """Blades meeting: bright inharmonic ring."""
    for ratio, w, d in ((1, 1.0, 0.5), (2.32, 0.7, 0.35), (4.17, 0.5, 0.25), (6.3, 0.3, 0.15)):
        tone(t0, d, 1180 * ratio, amp * w * 0.35, a=0.001, r=d * 0.9)
    noise(t0, 0.06, amp * 0.6, a=0.001, r=0.05, lp=0.9)

def shing(t0, amp=0.25, dur=0.5):
    noise(t0, dur, amp, a=0.01, r=dur * 0.7, lp=0.35, lp_end=0.95)
    tone(t0, dur, 2200, amp * 0.25, a=0.01, r=dur * 0.8, glide=5200)

def hat(t0, amp=0.05):
    noise(t0, 0.04, amp, a=0.001, r=0.035, lp=0.95)

def snare(t0, amp=0.18):
    noise(t0, 0.16, amp, a=0.001, r=0.15, lp=0.5)
    tone(t0, 0.1, 190, amp * 0.6, a=0.001, r=0.09)

# 0-2: black. Sub drone, the line grows, type ticks on TWO BLADES.
for f, a in ((55.0, 0.18), (82.4, 0.08)):
    tone(0, 30, f, a, a=1.5, r=3.0, harm=(1.0, 0.3))
tone(0, 2.0, 220, 0.05, a=1.5, r=0.02, glide=880)
noise(0, 2.0, 0.05, a=1.8, r=0.1, lp=0.02, lp_end=0.3)
for k in range(11):
    tone(0.5 + k * 0.045, 0.03, 1800, 0.05, a=0.001, r=0.025)
hit(1.5, 0.35)
# 2-6: eyes. Heartbeat on beats 1 and 3, glint chime on the beat.
for bar in range(1, 3):
    for k in range(4):
        kick(2.0 + (bar - 1) * 2 + k * B, 0.25 if k % 2 == 0 else 0.12)
tone(2.0, 4.0, note("A3"), 0.04, a=1.5, r=1.5)
tone(2.0, 4.0, note("E4"), 0.03, a=1.5, r=1.5)
noise(3.0, 0.15, 0.1, a=0.001, r=0.14, lp=0.7)   # blink
bell(3.5, note("A5"), 0.07, dur=2.0)
bell(5.5, note("E5"), 0.07, dur=2.0)
tone(4.0, 2.0, 110, 0.05, a=1.0, r=0.05, glide=220, harm=(1.0, 0.4))
# 6-8: wide. Tron arp wakes up.
pat = ["A3", "E4", "A4", "C5", "E5", "C5", "A4", "E4"]
t, k = 6.0, 0
while t < 14.0 - 0.01:
    f = note(pat[k % 8])
    vol = 0.03 + 0.035 * min(1.0, (t - 6.0) / 6.0)
    tone(t, B * 0.5, f, vol, a=0.003, r=B * 0.4, harm=(1.0, 0.5, 0.33, 0.2))
    t += B / 2
    k += 1
for k in range(8):
    kick(6.0 + k * B, 0.28)
hit(6.0, 0.4)
# 8-10: four cuts, one hit each
for t0 in (8.0, 8.5, 9.0, 9.5):
    hit(t0, 0.45)
    kick(t0, 0.3)
    hat(t0 + B / 2, 0.05)
# 10-11: eyes, snare roll accelerating
hit(10.0, 0.5)
hit(10.5, 0.5)
t, gap = 10.0, 0.25
while t < 11.0:
    snare(t, 0.10 + 0.1 * (t - 10.0))
    t += gap
    gap = max(0.05, gap * 0.8)
# 11.55 the draw
shing(11.5, 0.3, 0.5)
shing(11.6, 0.2, 0.4)
for k in range(8):
    kick(11.0 + k * 0.125, 0.12)
bell(11.0, note("A3"), 0.1, dur=2.0)
# 12-14: the charge. Four on the floor, bass, riser, then silence before the hit.
tone(12.0, 1.95, 90, 0.10, a=1.0, r=0.02, glide=1200)
noise(12.0, 1.95, 0.12, a=1.5, r=0.02, lp=0.03, lp_end=0.7)
for k in range(8):
    kick(12.0 + k * 0.25, 0.3 + 0.03 * k)
boom(13.96 + 0.04, 1.0)
clang(14.0, 0.5)
# 14-16: clash. Every cut rings.
for t0 in (14.5, 15.0):
    clang(t0, 0.4)
    hit(t0, 0.5)
boom(15.0, 0.9)
tone(15.0, 1.0, note("A2"), 0.12, a=0.01, r=0.9, harm=(1.0, 0.5))
hit(15.5, 0.45)
hit(16.0, 0.5)
# 16-20: drive. Kick on every beat, hat off beats, bass on A minor, clang on cuts.
chords = ["A2", "F2", "C3", "G2"]
t, k = 16.0, 0
while t < 20.0 - 0.01:
    kick(t, 0.5)
    hat(t + B / 2, 0.06)
    if k % 2 == 1:
        snare(t, 0.14)
    tone(t, B * 0.9, note(chords[(k // 4) % 4]), 0.14, a=0.005, r=B * 0.5, harm=(1.0, 0.6, 0.3))
    clang(t, 0.18 if k % 2 == 0 else 0.3)
    t += B
    k += 1
# 20-21.5: slow motion. Lowpassed, heartbeat, rising shimmer; 21.5-22 riser to the white-out.
for k in range(4):
    kick(20.0 + k * 0.5, 0.35 - 0.05 * k)
tone(20.0, 1.8, 55, 0.2, a=0.3, r=0.1, harm=(1.0, 0.3))
noise(20.0, 1.5, 0.06, a=0.5, r=0.2, lp=0.02, lp_end=0.1)
tone(21.0, 1.0, 220, 0.06, a=0.5, r=0.02, glide=4000, harm=(1.0, 0.5))
noise(21.4, 0.6, 0.2, a=0.5, r=0.02, lp=0.05, lp_end=0.95)
# 22: the cut. Everything stops but one long slice and the wind.
shing(22.0, 0.55, 0.45)
boom(22.0, 0.6)
tone(22.0, 2.0, note("A2"), 0.06, a=0.3, r=1.5)
noise(22.4, 1.6, 0.05, a=0.5, r=0.5, lp=0.03, lp_end=0.15)
bell(22.8, note("E5"), 0.08, dur=2.5)
shing(23.0, 0.45, 0.35)
bell(23.0, note("A3"), 0.16, dur=3.0)
# 24: title. Two booms, one per word, then the final chord.
boom(24.0, 1.0)
tone(24.0, 6.0, note("A2"), 0.12, a=0.01, r=2.5, harm=(1.0, 0.4, 0.2))
boom(24.5, 0.9)
noise(23.7, 0.3, 0.2, a=0.2, r=0.01, lp=0.05, lp_end=0.9)
for f in ("A3", "E4", "A4", "C5", "E5"):
    tone(24.0, 6.0, note(f), 0.05, a=0.1, r=3.0, harm=(1.0, 0.3))
for t0, f in ((25.0, "E6"), (25.5, "C6"), (26.0, "A5"), (26.5, "E5"), (27.0, "A5"), (28.0, "C6")):
    bell(t0, note(f), 0.05, dur=2.0)
for t0 in (28.0, 28.25, 28.5, 28.75):
    tone(t0, 0.06, 1500, 0.04, a=0.001, r=0.05)  # COMING SOON blink ticks

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

