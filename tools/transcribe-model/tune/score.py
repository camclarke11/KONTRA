"""Scores clean-up thresholds against true notes: onset within 50 ms and the
same key (mir_eval), each test as <raw outputs.npz>=<true notes.midi>.
usage: score.py a.npz=a.midi[,b.npz=b.midi...] ['ghost=0,restrike=0'] ['ghost=0.3,restrike=0.4'] ..."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np, mido, mir_eval, post


def truth(path):
    out, held, t = [], {}, 0.0
    for m in mido.MidiFile(path):
        t += m.time
        if m.type == 'note_on' and m.velocity > 0:
            held[m.note] = t
        elif m.type in ('note_off', 'note_on') and m.note in held:
            s = held.pop(m.note); out.append((s, max(t, s + 0.01), m.note))
    return out


def score(ref, est):
    hz = lambda ks: mir_eval.util.midi_to_hz(np.array(ks, float))
    ri = np.array([[s, e] for s, e, _ in ref]); ei = np.array([[n['start'], n['end']] for n in est])
    p, r, f, _ = mir_eval.transcription.precision_recall_f1_overlap(
        ri, hz([k for _, _, k in ref]), ei, hz([n['key'] for n in est]), offset_ratio=None)
    return p, r, f, len(est) - round(p * len(est)), len(ref) - round(r * len(ref))


tests = [t.split('=') for t in sys.argv[1].split(',')]
data = [(os.path.basename(a), dict(np.load(a)), truth(m)) for a, m in tests]
for c in sys.argv[2:] or ['ghost=0,restrike=0', '']:
    kw = eval(f'dict({c})')
    fake = missed = 0
    for name, d, ref in data:
        p, r, f, fp, fn = score(ref, post.notes(d, **kw)); fake += fp; missed += fn
        print(f"  {name:28s} P {p:.3f} R {r:.3f} F1 {f:.3f}")
    print(f"{c or 'as KONTRA does'}: {fake} false notes, {missed} true ones missed\n")
