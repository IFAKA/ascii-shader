#!/usr/bin/env python3
"""Sound for the 10 s ONE CUT short (stdlib only). Usage: pv_short_audio.py EVENTS.json OUT.wav
EVENTS.json comes from `ascii-shader --story pvshort --film-events EVENTS.json`. Every sound is placed
at the sample of the visual event (t * 44100), so picture and sound cannot drift. Stereo, 16 bit, 44.1 kHz.
Buses: amb (drone, wind, pad; ducked under hits), fx (hits and whooshes), rev (reverb send), post (after the
flash silence gate). Master: reverb mix, gate, fade, peak limiter."""
import json, math, random, struct, sys, wave
from itertools import accumulate

SR = 44100
TAU = 2 * math.pi
ev = json.load(open(sys.argv[1]))
EVENTS = ev["events"]
DUR = ev["duration"]
N = int(DUR * SR)
rng = random.Random(11)
sin, exp = math.sin, math.exp


def evs(kind):
    return [e for e in EVENTS if e["type"] == kind]


def T(kind, i=0):
    return evs(kind)[i]["t"]


def bus():
    return ([0.0] * N, [0.0] * N)


amb, fx, rev, post = bus(), bus(), bus(), bus()


def pg(p):
    a = (p + 1) * math.pi / 4
    return math.cos(a), math.sin(a)


def mix(b, t0, sig, amp=1.0, pan=0.0, pan2=None):
    i0 = int(round(t0 * SR))
    if i0 < 0:
        sig, i0 = sig[-i0:], 0
    n = min(len(sig), N - i0)
    if n <= 0:
        return
    L, R = b
    if pan2 is None:
        gl, gr = pg(pan)
        gl, gr = gl * amp, gr * amp
        for k in range(n):
            v = sig[k]
            L[i0 + k] += v * gl
            R[i0 + k] += v * gr
    else:
        for k in range(n):
            gl, gr = pg(pan + (pan2 - pan) * k / n)
            v = sig[k] * amp
            L[i0 + k] += v * gl
            R[i0 + k] += v * gr


def fxs(t0, sig, amp=1.0, pan=0.0, send=0.0, pan2=None):
    mix(fx, t0, sig, amp, pan, pan2)
    if send > 0:
        mix(rev, t0, sig, amp * send, pan, pan2)


# ---- generators (return lists of samples) ----
def decay(n, tau, atk=0.0015):
    a = max(1, int(atk * SR))
    return [min(1.0, k / a) * exp(-k / SR / tau) for k in range(n)]


def sine(f, dur, tau=None, atk=0.0015):
    n = int(dur * SR)
    e = decay(n, tau if tau else 1e9, atk)
    return [sin(TAU * f * k / SR) * e[k] for k in range(n)]


def drop(f_hi, f_lo, tau_p, dur, tau_a, atk=0.0015):
    """Sine whose pitch falls from f_hi to f_lo (time constant tau_p), amplitude decays with tau_a."""
    n = int(dur * SR)
    ph = accumulate(TAU * (f_lo + (f_hi - f_lo) * exp(-k / SR / tau_p)) / SR for k in range(n))
    e = decay(n, tau_a, atk)
    return [sin(p) * e[k] for k, p in enumerate(ph)]


def sweep(f0, f1, dur, harm=(1.0,)):
    n = int(dur * SR)
    ph = list(accumulate(TAU * f0 * (f1 / f0) ** (k / n) / SR for k in range(n)))
    return [sum(w * sin((j + 1) * p) for j, w in enumerate(harm)) for p in ph]


def lp_noise(dur, c0, c1, passes=2):
    n = int(dur * SR)
    out = [0.0] * n
    ys = [0.0] * passes
    for k in range(n):
        c = c0 * (c1 / c0) ** (k / n)
        x = rng.uniform(-1, 1)
        for j in range(passes):
            ys[j] += c * (x - ys[j])
            x = ys[j]
        out[k] = x
    return out


def hp(sig, c=0.08):
    y, out = 0.0, []
    for x in sig:
        y += c * (x - y)
        out.append(x - y)
    return out


def burst(dur, c0, c1, tau, atk=0.001, hpc=None):
    s = lp_noise(dur, c0, c1, passes=1)
    if hpc:
        s = hp(s, hpc)
    e = decay(len(s), tau, atk)
    return [a * b for a, b in zip(s, e)]


def shaped(sig, f):
    n = len(sig)
    return [v * f(k / n) for k, v in enumerate(sig)]


def metal(f0, ratios, dur, amp=1.0, tau=0.5):
    """Inharmonic partials; higher partials die sooner."""
    n = int(dur * SR)
    out = [0.0] * n
    for i, r in enumerate(ratios):
        e = decay(n, tau / (1 + 0.7 * i), 0.001)
        w = amp / (1 + 0.5 * i)
        f = f0 * r
        for k in range(n):
            out[k] += w * sin(TAU * f * k / SR) * e[k]
    return out


CLANG = (1.0, 1.59, 2.76, 4.17, 5.4, 8.93)


def crush(sig, bits=4):
    q = 2 ** bits
    return [round(v * q) / q for v in sig]


# ---- ducking and gate arrays ----
duck = [1.0] * N


def add_duck(t, depth, rel):
    i0 = int(t * SR)
    la = int(0.005 * SR)
    for k in range(la):
        j = i0 - la + k
        if 0 <= j < N:
            duck[j] = min(duck[j], 1 - depth * (k + 1) / la)
    for k in range(min(int(rel * 5 * SR), N - i0)):
        duck[i0 + k] = min(duck[i0 + k], 1 - depth * exp(-k / SR / rel))


# =============== amb: drone, wind, pad ===============
def smooth_step(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


T_SLOW0, T_SLOW1, T_FLASH = T("slowmo_start"), 5.0 + 0.4, T("flash")
T_AFTER, T_TITLE, T_SETTLE = T("aftermath"), T("title_word"), T("title_settle")


def pitch(t):
    if t < T_SLOW0:
        return 1.0
    if t < T_FLASH:
        return 1.0 - 0.18 * smooth_step((t - T_SLOW0) / 0.7)
    return 1.0


def cutoff(t):
    slow = smooth_step((t - T_SLOW0 - 0.1) / 0.3) - smooth_step((t - T_SLOW1) / 0.1)
    return 0.30 - 0.265 * max(0.0, slow)


def level(t):
    if t < 1.5:
        return smooth_step(t / 1.5) * 0.7
    if t < T_AFTER:
        return 0.7 + 0.3 * smooth_step((t - 2.0) / 1.0)  # (everything but the ring is gated from the flash to T_AFTER)
    if t < T_TITLE:
        return 0.5 * smooth_step((t - T_AFTER) / 1.0)
    return 0.5 + 0.7 * smooth_step((t - T_TITLE) / 1.0)


def drone():
    for ch, det in ((0, 1.0), (1, 1.004)):
        out = [0.0] * N
        ph1 = ph2 = ph3 = 0.0
        y = 0.0
        for k in range(N):
            t = k / SR
            p = pitch(t) * det
            ph1 += TAU * 55.0 * p / SR
            ph2 += TAU * 82.41 * p / SR
            ph3 += TAU * 110.0 * p / SR
            v = 0.5 * sin(ph1) + 0.17 * sin(2 * ph1) + 0.2 * sin(ph2) + 0.07 * sin(3 * ph1) + 0.08 * sin(ph3)
            y += cutoff(t) * (v - y)
            out[k] = y * level(t) * 0.55
        for k in range(N):
            amb[ch][k] += out[k]


drone()
# wind in the aftermath, rising to the slash
wind = lp_noise(T("slash") - T_AFTER + 0.2, 0.02, 0.12)
mix(amb, T_AFTER, shaped(wind, lambda u: 0.07 * smooth_step(u * 2.0)), 1.0, -0.5, 0.5)
# title pad: A2 E3 A3 under the settle, fading with the fade
for f, a in ((110.0, 0.09), (164.8, 0.06), (220.0, 0.05), (329.6, 0.025)):
    mix(amb, T_SETTLE, sine(f, DUR - T_SETTLE, atk=0.4), a, -0.2 if f < 200 else 0.2)

# =============== fx ===============
# --- hook line: thin rising sweep, widening, tick at the end; drone is already fading in ---
hl = evs("hook_line")[0]
for p0, p1, s in ((0.0, -1.0, 1.0), (0.0, 1.0, 1.02)):
    sw = sweep(300 * s, 4200 * s, hl["dur"], (1.0, 0.25))
    fxs(hl["t"], shaped(sw, lambda u: 0.12 * u ** 1.5), 1.0, p0, 0.15, p1)
fxs(hl["t"], shaped(hp(lp_noise(hl["dur"], 0.15, 0.9), 0.2), lambda u: 0.10 * u ** 2), 1.0, 0, 0.1)
tk = T("hook_tick")
fxs(tk, burst(0.02, 0.9, 0.9, 0.006, hpc=0.2), 0.35, 0, 0.2)
fxs(tk, sine(2600, 0.06, 0.012), 0.12, 0, 0.3)
fxs(tk, drop(90, 48, 0.05, 0.4, 0.12), 0.35)

# --- eyes: sub heartbeat per eye, second one higher; glass shimmer ---
for i, e in enumerate(evs("eye_open")):
    hi, lo = (95, 52) if i == 0 else (112, 62)
    fxs(e["t"], drop(hi, lo, 0.05, 0.5, 0.14), 0.85)
    air = shaped(hp(lp_noise(0.28, 0.1, 0.5), 0.12), lambda u: 0.07 * math.sin(math.pi * u))
    fxs(e["t"], air, 1.0, -0.4 if i == 0 else 0.4, 0.2)
    fxs(e["t"] + 0.01, metal(3150 * (1 if i == 0 else 0.84), (1.0, 1.34, 1.93), 0.7, 0.05, 0.3), 1.0, -0.5 if i == 0 else 0.5, 0.5)

# --- cut ticks on every shot cut (the big hits below cover the main cuts) ---
for e in evs("shot_cut"):
    if e["t"] > 0:
        fxs(e["t"], burst(0.014, 0.9, 0.9, 0.004, hpc=0.25), 0.16, 0.0, 0.1)


def kick(t, amp=0.5, pan=0.0):
    fxs(t, drop(150, 45, 0.045, 0.32, 0.1), amp, pan)


# --- dash: two whooshes converge on the middle, a riser peaks on the clash frame ---
d = evs("dash_start")[0]
t_clash = T("clash")
dd = t_clash - d["t"] - 0.015  # everything stops 15 ms before the hit, so the hit lands in a gap
for p0, p1 in ((-1.0, -0.12), (1.0, 0.12)):
    w = lp_noise(dd, 0.012, 0.55)
    fxs(d["t"], shaped(w, lambda u: 0.55 * u ** 2.2), 1.0, p0, 0.12, p1)
rs = sweep(110, 1900, dd, (1.0, 0.4, 0.2))
fxs(d["t"], shaped(rs, lambda u: 0.14 * u ** 2.0), 1.0, -0.15, 0.05, 0.15)
fxs(d["t"], shaped(sweep(220, 3800, dd, (1.0, 0.3)), lambda u: 0.07 * u ** 3.0), 1.0, 0.2, 0.05, -0.2)
kick(d["t"], 0.55)
fxs(d["t"], burst(0.08, 0.7, 0.7, 0.03, hpc=0.1), 0.25, 0, 0.2)
for kt, ka in ((2.0, 0.42), (2.5, 0.5), (2.75, 0.42), (2.875, 0.4)):
    kick(kt, ka)
sp = T("speed_peak")
for j in range(3):
    fxs(sp + j * 0.035, sine(3900 + 500 * j, 0.03, 0.01), 0.08, rng.uniform(-0.7, 0.7))

# --- clash: boom + clang + crack + reverb, sparks ---
fxs(t_clash, drop(120, 36, 0.11, 1.4, 0.45), 1.0, 0, 0.18)
fxs(t_clash, burst(0.5, 0.6, 0.04, 0.08, atk=0.0005), 0.45, 0, 0.2)
fxs(t_clash, burst(0.03, 0.95, 0.95, 0.01, atk=0.0003, hpc=0.1), 0.9, 0, 0.1)  # crack
cl = metal(640, CLANG, 1.6, 0.30, 0.7)
fxs(t_clash, cl, 1.0, -0.15, 0.55)
fxs(t_clash + 0.004, metal(642, CLANG, 1.2, 0.2, 0.5), 1.0, 0.25, 0.55)
add_duck(t_clash, 0.9, 0.6)


def sparks(t0, dur, n):
    for _ in range(n):
        ts = t0 + 0.02 + rng.random() * (dur - 0.03)
        f = rng.uniform(3000, 9000)
        fxs(ts, sine(f, 0.02, rng.uniform(0.003, 0.009)), rng.uniform(0.04, 0.12), rng.uniform(-0.9, 0.9), 0.3)


s0 = evs("sparks")
sparks(s0[0]["t"], s0[0]["dur"], 14)

# --- slow motion: tape-stop sweep, slow heartbeat, reversed clang into the pass, reverse cymbal into the flash ---
t0s = T("slowmo_start")
fxs(t0s, shaped(sweep(900, 55, 0.5, (1.0, 0.4)), lambda u: 0.16 * (1 - u)), 1.0, 0, 0.3)
fxs(t0s, shaped(lp_noise(0.5, 0.5, 0.02), lambda u: 0.25 * (1 - u) ** 1.5), 1.0, 0, 0.3)
for hb, a in ((3.5, 0.0), (4.0, 0.7), (4.6, 0.8), (5.25, 0.9)):
    if hb > t0s:
        fxs(hb, drop(70, 38, 0.06, 0.6, 0.22), a * 0.8, 0, 0.15)
        fxs(hb + 0.17, drop(62, 34, 0.06, 0.5, 0.16), a * 0.45, 0, 0.1)
    add_duck(hb, 0.35, 0.3)
t_pass = T("pass")
rv_clang = metal(420, CLANG, 0.8, 0.3, 0.5)[::-1]
fxs(t_pass - 0.8, shaped(rv_clang, lambda u: (u ** 1.5 if u < 0.97 else 0.0) * 0.5), 1.0, -0.4, 0.4, 0.2)
# pass: low thud, soft clang, whoosh sweeping left to right across the pass frame
fxs(t_pass, drop(95, 40, 0.07, 0.6, 0.2), 1.0)
fxs(t_pass, metal(420, CLANG, 1.4, 0.2, 0.6), 1.0, 0.0, 0.5)
fxs(t_pass, burst(0.02, 0.9, 0.9, 0.008, hpc=0.1), 0.5, 0, 0.1)
pw = lp_noise(0.8, 0.02, 0.2)
fxs(t_pass - 0.4, shaped(pw, lambda u: 0.16 * math.sin(math.pi * u) ** 2), 1.0, -1.0, 0.2, 1.0)
add_duck(t_pass, 0.6, 0.35)
s1 = evs("sparks")[1]
sparks(s1["t"], s1["dur"], 7)
# reverse cymbal pulling into the flash (cut by the silence gate at t_flash)
rc_n = 1.15
rc = hp(lp_noise(rc_n, 0.3, 0.95, passes=1), 0.15)
t_flash = T("flash")
fxs(t_flash - rc_n, shaped(rc, lambda u: 0.55 * u ** 3.0), 1.0, -0.3, 0.1, 0.3)
fxs(t_flash - rc_n, shaped(sweep(300, 5000, rc_n, (1.0, 0.3)), lambda u: 0.12 * u ** 3.0), 1.0, 0, 0.1)

# --- slash and split ---
t_sl = T("slash")
add_duck(t_sl - 0.15, 0.85, 0.1)  # tension: the world drops away just before the slash
sh = lp_noise(0.28, 0.25, 0.97, passes=1)
fxs(t_sl, shaped(hp(sh, 0.05), lambda u: 0.7 * min(1, u * 30) * (1 - u) ** 1.3), 1.0, -0.8, 0.35, 0.8)
fxs(t_sl, shaped(sweep(2400, 6800, 0.3, (1.0, 0.3)), lambda u: 0.17 * min(1, u * 40) * (1 - u)), 1.0, -0.8, 0.3, 0.8)
fxs(t_sl, metal(3100, (1.0, 1.53, 2.07, 2.91), 1.2, 0.07, 0.5), 1.0, 0.0, 0.6)
fxs(t_sl, burst(0.015, 0.95, 0.95, 0.005, atk=0.0003, hpc=0.1), 0.45, 0, 0.2)
add_duck(t_sl, 0.5, 0.3)
t_sp = T("split")
fxs(t_sp, drop(170, 42, 0.07, 0.6, 0.18), 0.9, 0, 0.3)
fxs(t_sp, burst(0.05, 0.4, 0.15, 0.02, atk=0.0005), 0.7, 0, 0.3)
fxs(t_sp, burst(0.012, 0.95, 0.95, 0.004, atk=0.0003, hpc=0.1), 0.55, 0, 0.1)
for pan, f, dly in ((-0.7, 1760.0, 0.02), (0.7, 2093.0, 0.06)):
    fxs(t_sp + dly, metal(f, (1.0, 2.01, 3.02), 1.4, 0.10, 0.7), 1.0, pan, 0.6)
for p0, p1 in ((0.0, -1.0), (0.0, 1.0)):
    sc = hp(lp_noise(0.3, 0.1, 0.4), 0.1)
    fxs(t_sp, shaped(sc, lambda u: 0.22 * (1 - u) ** 2), 1.0, p0, 0.2, p1)
add_duck(t_sp, 0.6, 0.35)
# swell into the title (reverse noise), peaks on the title frame
tt = T("title_word")
fxs(tt - 0.3, shaped(hp(lp_noise(0.3, 0.05, 0.8, passes=1), 0.1), lambda u: 0.3 * u ** 2.5), 1.0, 0, 0.2)

# --- title: each word lands with a sub hit and digital glitch ticks; settle = deep boom + long tail ---
for e in evs("title_word"):
    base = 90 if e["n"] == 0 else 105
    fxs(e["t"], drop(base * 1.2, base * 0.45, 0.07, 0.8, 0.28), 0.95, 0, 0.2)
    fxs(e["t"], burst(0.04, 0.9, 0.9, 0.012, atk=0.0004, hpc=0.1), 0.45, 0, 0.1)
    fxs(e["t"], metal(900 + 120 * e["n"], CLANG, 0.8, 0.1, 0.3), 1.0, -0.3 + 0.6 * e["n"], 0.5)
    for i in range(e["letters"]):
        tg = e["t"] + i * e["letter_dt"]
        f = rng.uniform(1500, 6000)
        gl = crush(sine(f, 0.012, 0.01), 3)
        fxs(tg, gl, 0.16, rng.uniform(-0.8, 0.8), 0.15)
        fxs(tg + 0.011, crush(burst(0.01, 0.95, 0.95, 0.008, hpc=0.3), 2), 0.12, rng.uniform(-0.8, 0.8), 0.1)
    add_duck(e["t"], 0.85, 0.5)
fxs(T_SETTLE, drop(80, 30, 0.12, 2.2, 0.8), 1.0, 0, 0.25)
fxs(T_SETTLE, sine(41.2, 1.5, 0.7), 0.35)
fxs(T_SETTLE, burst(0.9, 0.5, 0.02, 0.25, atk=0.0005), 0.4, 0, 0.3)
fxs(T_SETTLE, metal(440, CLANG, 2.0, 0.22, 1.0), 1.0, -0.2, 0.7)
fxs(T_SETTLE + 0.003, metal(443, CLANG, 1.8, 0.16, 0.9), 1.0, 0.25, 0.7)
add_duck(T_SETTLE, 0.8, 0.9)
fxs(T("tagline") + 0.06, metal(2637, (1.0, 2.0, 3.0), 0.9, 0.05, 0.3), 1.0, 0.2, 0.5)
# coming soon: three blink ticks
for bt in (9.0 + 2 / 24, 9.0 + 6 / 24, 9.0 + 8 / 24):
    fxs(bt, crush(sine(1500, 0.04, 0.02), 4), 0.12, 0.0, 0.2)

# --- aftermath: the world returns after the silence ---
fxs(T_AFTER, drop(60, 38, 0.06, 0.6, 0.25), 0.35)

# 25 ms gap right before the pass hit (same trick as before the clash), so the hit stands out of the build-up
g0, g1 = int((t_pass - 0.025) * SR), int(t_pass * SR)
for b in (fx, rev, amb):
    for c in range(2):
        for i in range(g0, g1):
            b[c][i] = 0.0

# =============== reverb ===============
def comb(x, d, fb, damp):
    y = [0.0] * N
    s = 0.0
    for i in range(d, N):
        s = y[i - d] * (1 - damp) + s * damp
        y[i] = x[i] + fb * s
    return y


def allpass(x, d, g):
    y = [0.0] * N
    for i in range(d, N):
        y[i] = -g * x[i] + x[i - d] + g * y[i - d]
    return y


def reverb(src, delays, fb=0.87, damp=0.35):
    cs = [comb(src, d, fb, damp) for d in delays]
    out = [sum(v) * 0.25 for v in zip(*cs)]
    out = allpass(out, 556, 0.5)
    return allpass(out, 441, 0.5)


wet_l = reverb(rev[0], (1557, 1617, 1491, 1422))
wet_r = reverb(rev[1], (1580, 1640, 1514, 1445))

# =============== master ===============
fl, fr = T_FLASH, T_FLASH + evs("flash")[0]["silence"]
gate = [1.0] * N
i0, i1 = int(round(fl * SR)), int(round(T_AFTER * SR))  # silence for the flash, then only the ring until the aftermath cut
ramp = int(0.0007 * SR)
for i in range(i0, min(i1, N)):
    gate[i] = min(1.0, max(0.0, 1 - (i - i0 + 1) / ramp))
# ringing high tone after the silence
for p, f in ((-0.5, 3520.0), (0.5, 3524.0)):
    mix(post, fr, sine(f, 2.2, 0.8, atk=0.001), 0.16, p)
mix(post, fr, sine(5274.0, 1.5, 0.35, atk=0.001), 0.05, 0.0)
mix(post, fr, sine(880.0, 2.0, 0.9, atk=0.001), 0.05, 0.0)

fo = int((DUR - 0.5) * SR)
outL, outR = [0.0] * N, [0.0] * N
for i in range(N):
    g = gate[i]
    d_ = duck[i]
    fade = 1.0
    if i >= fo:
        fade = math.cos((i - fo) / (N - fo) * math.pi / 2) ** 2
    if i < 200:
        fade *= i / 200
    outL[i] = ((amb[0][i] * d_ + fx[0][i] + wet_l[i] * 0.9) * g + post[0][i]) * fade
    outR[i] = ((amb[1][i] * d_ + fx[1][i] + wet_r[i] * 0.9) * g + post[1][i]) * fade

# peak limiter: instant drop, 2 ms look-ahead ramp, 80 ms release
thr = 0.9
gn = [thr / m if m > thr else 1.0 for m in (max(abs(a), abs(b)) for a, b in zip(outL, outR))]
la = int(0.002 * SR)
for i in range(N - 2, -1, -1):
    gn[i] = min(gn[i], gn[i + 1] + (1.0 - gn[i + 1]) / la)
rel = 1.0 / (0.08 * SR)
for i in range(1, N):
    gn[i] = min(gn[i], gn[i - 1] + rel)
w = wave.open(sys.argv[2], "wb")
w.setnchannels(2)
w.setsampwidth(2)
w.setframerate(SR)
data = bytearray()
peak = 0.0
for i in range(N):
    a, b = outL[i] * gn[i], outR[i] * gn[i]
    peak = max(peak, abs(a), abs(b))
    data += struct.pack("<hh", int(max(-1, min(1, a)) * 32767), int(max(-1, min(1, b)) * 32767))
w.writeframes(bytes(data))
w.close()
print(f"wrote {sys.argv[2]}: {N / SR:.3f} s, peak {peak:.3f}")
