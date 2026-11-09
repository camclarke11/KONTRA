"""Exports ByteDance's piano transcription (Kong et al. 2020; code Apache-2.0,
checkpoint CC-BY-4.0, zenodo.org/record/4034264) to one ONNX graph: a 10 s
segment of 16 kHz mono audio in, the note and pedal regressions out. Then
checks onnxruntime against PyTorch on a segment of real audio.

usage: python export.py <checkpoint.pth> <out.onnx> <test.wav>
"""
import sys

import librosa
import numpy as np
import onnxruntime as ort
import torch
from piano_transcription_inference.models import Note_pedal

SEGMENT = 160_000  # 10 s at 16 kHz
NAMES = [
    "onset",
    "offset",
    "frame",
    "velocity",
    "pedal_offset",
    "pedal_frame",
]


class Graph(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, audio):
        n = self.model.note_model(audio)
        p = self.model.pedal_model(audio)
        return (
            n["reg_onset_output"],
            n["reg_offset_output"],
            n["frame_output"],
            n["velocity_output"],
            p["reg_pedal_offset_output"],
            p["pedal_frame_output"],
        )


def main(checkpoint, out, wav):
    model = Note_pedal(frames_per_second=100, classes_num=88)
    state = torch.load(checkpoint, map_location="cpu", weights_only=False)
    model.load_state_dict(state["model"], strict=False)
    model.eval()
    graph = Graph(model).eval()
    dummy = torch.zeros(1, SEGMENT)
    torch.onnx.export(
        graph,
        (dummy,),
        out,
        input_names=["audio"],
        output_names=NAMES,
        opset_version=17,
        dynamo=False,
    )
    print("exported", out)
    audio, _ = librosa.load(wav, sr=16000, mono=True, res_type="kaiser_best")
    seg = np.zeros((1, SEGMENT), dtype=np.float32)
    seg[0, : min(SEGMENT, len(audio))] = audio[:SEGMENT]
    with torch.no_grad():
        want = [t.numpy() for t in graph(torch.from_numpy(seg))]
    sess = ort.InferenceSession(out)
    got = sess.run(None, {"audio": seg})
    for name, w, g in zip(NAMES, want, got):
        print(f"{name:13s} {w.shape} max |diff| {np.abs(w - g).max():.2e}")
    np.save(out + ".input.npy", seg)
    for name, w in zip(NAMES, want):
        np.save(f"{out}.{name}.npy", w)


if __name__ == "__main__":
    main(*sys.argv[1:4])
