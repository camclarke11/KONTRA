# The piano separation model

Before a recording is transcribed (the Songs tab's "Transcribe a
recording…" with "Piano only" on, the default), KONTRA isolates its piano:
voices, drums, bass, guitars and the rest are separated out, so only the
piano becomes notes. `crates/app/transcribe/src/separate.rs` runs
**BS-RoFormer** (Band-Split RoPE Transformer: W.-T. Lu, J.-C. Wang,
Q. Kong, Y.-N. Hung, ByteDance, [arXiv:2309.02612](https://arxiv.org/abs/2309.02612);
the implementation follows [lucidrains/BS-RoFormer](https://github.com/lucidrains/BS-RoFormer), MIT)
on the GPU with candle (Metal on a Mac).

The weights are **BS-RoFormer-SW**, a community-trained six-stem model
(vocals, drums, bass, guitar, piano, other), the best openly downloadable
piano stem found on 2026-11-09: piano SDR 7.83 dB and vocals 11.30 dB on
[MVSep's multisong benchmark](https://mvsep.com/quality_checker/leaderboard/piano/?sort=piano),
within 0.1 dB of the leaderboard's best. Who trained it is not documented
and **no licence is stated**, so the weights are not in this repository and
must not be redistributed; download them yourself for your own use. This
folder keeps only the piano stem (51 M parameters, 204 MB, from 175 M):

```sh
# The same Python environment as tools/transcribe-model, plus:
uv pip install "git+https://github.com/openmirlab/bs-roformer-infer" ml_collections safetensors
curl -L -o BS-Rofo-SW-Fixed.ckpt \
  https://huggingface.co/enerjazzer/BS-ROFO-SW-Fixed/resolve/main/BS-Rofo-SW-Fixed.ckpt
shasum -a 256 BS-Rofo-SW-Fixed.ckpt
# 24e7d35ee9c64415673d3fd33e06a67cac2c103c5df6267ba1576459c775916e
python export.py BS-Rofo-SW-Fixed.ckpt piano-separation.safetensors <a 44.1 kHz recording.wav>
```

`export.py` writes the backbone and the piano's mask estimator, renamed
plainly, with the settings in the file's metadata; given a recording it
also writes `piano-separation.safetensors.check`: one chunk of it and the
piano PyTorch hears there. `cargo run --release -p kontra-transcribe
--example separate -- piano-separation.safetensors` checks the Rust model
against it (and, given the recording and PyTorch's demixed piano WAV, a
whole song).

KONTRA looks for `piano-separation.safetensors` in `KONTRA_SEPARATE_MODEL`,
the app bundle's `Contents/Resources`, then
`~/Library/Application Support/kontra-v3/models/`. Without it, recordings
are transcribed whole and the Songs tab says the model is missing.

Checked on 2026-11-09:

- Against PyTorch: one chunk agrees to 3.6e-7 (on a signal of rms 0.03); a
  7-minute song, demixed, agrees but for about 20 of its 19 million samples
  (PyTorch ran on the GPU there).
- On a Chopin nocturne rendered by macOS's General MIDI piano with a sung
  vocal mixed in at the same loudness, the transcription's note F1 (onset
  within 50 ms) is 0.799 without isolating the piano and 0.939 with it; on
  the piano alone it is 0.937 either way.
- On an M5 Pro a 13.35 s chunk takes 2.6 s on the GPU (PyTorch on the GPU:
  2.8 s): a 7-minute song is isolated in about 3 minutes.
