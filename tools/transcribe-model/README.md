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

## Cleaning up what a voice leaves

With the piano isolated from a song, what is left of the voice still makes
the model hear an onset now and then: mostly a key "struck" that then never
sounds (a note of 10 ms on the falling notes), or a key already ringing
"struck again" weakly, cutting a held note in two. `post.rs` takes neither
for a note (`GHOST`, `RESTRIKE`). `tune/` measured them, on 2026-11-09:
three recordings from MAESTRO v3's test split (Chopin's Nocturne Op. 15
No. 2, Debussy's "Des pas sur la neige", Rachmaninoff's Prelude Op. 32
No. 12; CC BY-NC-SA 4.0, not in this repository), each with a singer's
voice mixed over it as loud as the piano, isolated, transcribed, and scored
against their true notes (onset within 50 ms, same key):

| | false notes | true notes missed (of 3127) |
| --- | --- | --- |
| the authors' post-processing | 147 | 152 |
| with the clean-up | 99 | 179 |

On the solo recordings, isolated: 33 → 25 false, 119 → 140 missed. On a
seven-minute song with a voice (Jacob Collier's "Fix You", live), notes
under 60 ms went from 155 to 14, and 2399 notes to 2134.

```sh
python tune/mix.py <vocals.wav> <the song's piano.wav> <out dir> <piece.wav>...   # voice over each
# isolate each (tools/separate-model), then:
python tune/dump.py piano-transcription.onnx <isolated.wav> <out.npz>              # the raw outputs
python tune/score.py a.npz=a.midi,b.npz=b.midi 'ghost=0,restrike=0' 'ghost=0.3,restrike=0.4'
```
