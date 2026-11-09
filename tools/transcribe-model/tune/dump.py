"""The transcription model's raw outputs for a recording, segmented and
joined as crates/app/transcribe/src/lib.rs does. usage: dump.py <model.onnx> <in.wav> <out.npz>"""
import sys, numpy as np, librosa, onnxruntime as ort
SEG, HOP = 160000, 80000
sess = ort.InferenceSession(sys.argv[1], providers=['CPUExecutionProvider'])
names = [o.name for o in sess.get_outputs()]
audio, _ = librosa.load(sys.argv[2], sr=16000, mono=True)
n = max(1, -(-len(audio) // SEG)) * 2 - 1
outs = []
for i in range(n):
    piece = np.zeros((1, SEG), np.float32)
    a = audio[i * HOP:i * HOP + SEG]
    piece[0, :len(a)] = a
    outs.append(sess.run(None, {sess.get_inputs()[0].name: piece}))
frames = len(audio) // 160 + 1
joined = {}
for k, name in enumerate(names):
    parts = [o[k].reshape(1001, -1) for o in outs]
    if len(parts) == 1:
        j = parts[0]
    else:
        j = np.concatenate([p[(0 if i == 0 else 250):(1000 if i == len(parts) - 1 else 750)] for i, p in enumerate(parts)])
    joined[name] = j[:frames]
np.savez(sys.argv[3], **joined)
print(sys.argv[2], {k: v.shape for k, v in joined.items()})
