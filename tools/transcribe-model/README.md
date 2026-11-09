# The transcription model

"Transcribe a recording" (the Songs tab, or a recording dropped on the
editor) runs ByteDance's high-resolution piano transcription with pedals
(Q. Kong, B. Li, X. Song, Y. Wan, Y. Wang, 2020;
[paper](https://arxiv.org/abs/2010.01815),
[code](https://github.com/bytedance/piano_transcription), Apache-2.0) in
`crates/app/transcribe`, with tract. Its trained model is not in this
repository: it is 136 MB, and it is the authors' (CC BY 4.0,
[zenodo.org/record/4034264](https://zenodo.org/record/4034264)). This folder
makes it once, as one ONNX graph:

```sh
uv venv --python 3.12 venv && . venv/bin/activate
uv pip install torch torchlibrosa librosa resampy onnx onnxruntime numpy mido soundfile audioread piano_transcription_inference
curl -L -o checkpoint.pth \
  "https://zenodo.org/records/4034264/files/CRNN_note_F1%3D0.9677_pedal_F1%3D0.9186.pth?download=1"
md5 checkpoint.pth   # 22b961b77c1878239fec963362097045
python export.py checkpoint.pth piano-transcription.onnx <a piano recording.wav>
```

`export.py` writes the graph (a 10 s segment of 16 kHz mono audio in; note
onset, offset, frame and velocity regressions and the pedal's offset and
frame out, at 100 frames a second) and checks onnxruntime's outputs against
PyTorch's on the recording's first ten seconds.

KONTRA looks for `piano-transcription.onnx`, in order: the path in
`KONTRA_TRANSCRIBE_MODEL`; the app bundle's `Contents/Resources` (or beside
the executable); `~/Library/Application Support/kontra-v3/models/` on macOS
(the data folder elsewhere). Without it the Songs tab says the model is
missing.

Checked on 2026-11-09 against the authors' PyTorch pipeline, on 60 s of a
Chopin nocturne rendered from MIDI by macOS's General MIDI piano: the Rust
pipeline found the same notes (F1 0.993 against PyTorch's, velocities
within 1 on average) and scored as PyTorch does against the MIDI itself
(onset within 50 ms: precision 0.89, recall 0.99). On an M5 Pro it
transcribes a minute of audio in about 4.5 s on four threads.
