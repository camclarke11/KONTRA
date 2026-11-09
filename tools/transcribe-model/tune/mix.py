"""Mixes each MAESTRO recording with the Fix You vocals at the song's own
piano-to-voice balance. usage: mix.py <vocals.wav> <piano stem.wav> <out dir> <piece.wav>..."""
import sys, os, numpy as np, soundfile as sf
voc, _ = sf.read(sys.argv[1], dtype='float32', always_2d=True)
pno, _ = sf.read(sys.argv[2], dtype='float32', always_2d=True)
rms = lambda x: float(np.sqrt((x ** 2).mean()))
ratio = rms(voc) / rms(pno)
print(f"voice/piano rms in Fix You: {ratio:.2f}")
for p in sys.argv[4:]:
    x, sr = sf.read(p, dtype='float32', always_2d=True)
    assert sr == 44100
    v = np.resize(voc[30 * sr:], x.shape) if len(voc) - 30 * sr < len(x) else voc[30 * sr:30 * sr + len(x)]
    v = v * (ratio * rms(x) / max(rms(v), 1e-9))
    m = x + v
    m = m / max(1.0, float(np.abs(m).max()) / 0.98)
    out = os.path.join(sys.argv[3], os.path.basename(p).replace('.wav', '-vocals.wav'))
    sf.write(out, m, sr)
    print(out, len(x) / sr)
