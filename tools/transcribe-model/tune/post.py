"""crates/app/transcribe/src/post.rs's notes in numpy, its clean-up's
thresholds as arguments, to try other ones against true notes (score.py).
The pedal is left out."""
import numpy as np
ONSET, OFFSET, FRAME, LONGEST, FPS = 0.3, 0.3, 0.1, 600, 100.


def peaks(x, threshold, nb):
    out = [None] * len(x)
    at = lambda n: x[n] if 0 <= n < len(x) else 0.
    for n in range(nb, len(x) - nb):
        v = x[n]
        if v <= threshold: continue
        if not all(at(n - i) >= at(n - i - 1) for i in range(nb)): continue
        if not all(at(n + i) >= at(n + i + 1) for i in range(nb)): continue
        b, a = at(n - 1), at(n + 1)
        s = (a - b) / (v - a) / 2 if b > a else (a - b) / (v - b) / 2 if v != b else 0.
        out[n] = s if np.isfinite(s) else 0.
    return out


def key_notes(frame, on, off, vel):
    out = []; begun = faded = ended = None
    sh = lambda s, n: s[n] if s[n] is not None else 0.
    last = len(on) - 1
    for i in range(len(on)):
        if on[i] is not None:
            if begun is not None:
                out.append((begun, max(i - 1, 0), sh(on, begun), 0., vel[begun]))
                faded = ended = None
            begun = i
        if begun is None or i <= begun: continue
        b = begun
        if faded is None and frame[i] <= FRAME: faded = i
        if ended is None and off[i] is not None: ended = i
        if faded is not None:
            fin = ended if (ended is not None and ended - b > max(faded - ended, 0)) else faded
            out.append((b, fin, sh(on, b), sh(off, fin), vel[b])); begun = faded = ended = None
        elif i - b >= LONGEST or i == last:
            out.append((b, i, sh(on, b), sh(off, i), vel[b])); begun = faded = ended = None
    return out


def notes(d, onset=ONSET, ghost=0.3, look=10, restrike=0.4, sounding=0.5):
    """The notes in a recording's raw outputs `d` (dump.py). An onset is no
    note when: on a silent key whose frame output never reaches `ghost` in
    the `look` frames after; or on a key sounding (frame over `sounding`
    just before) and under `restrike`. ghost=0, restrike=0 is the authors'
    own post-processing."""
    O, F, V, Off = d['onset'], d['frame'], d['velocity'], d['offset']
    res = []
    for k in range(88):
        on = peaks(O[:, k], onset, 2); off = peaks(Off[:, k], OFFSET, 4)
        f = F[:, k]
        for b in range(len(on)):
            if on[b] is None: continue
            pre = f[max(b - 3, 0):b].max() if b > 0 else 0.
            if (O[b, k] < restrike) if pre >= sounding else (f[b:b + look].max() < ghost):
                on[b] = None
        for b, fi, s, e, v in key_notes(f, on, off, V[:, k]):
            start = max((b + s) / FPS, 0.); end = max((fi + e) / FPS, start + 1 / FPS)
            res.append(dict(start=start, end=end, key=21 + k, velocity=int(np.clip(int(v * 128), 1, 127))))
    res.sort(key=lambda n: (n['start'], n['key']))
    return res
