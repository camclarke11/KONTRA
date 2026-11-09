"""Exports BS-RoFormer-SW's piano stem for KONTRA's separator
(crates/app/transcribe/src/separate.rs): its band split, its 12 time and
frequency transformer layers and the piano's mask estimator alone, renamed
plainly, as one safetensors file with the settings in its metadata. Also
writes a check: one chunk of a recording and the piano PyTorch hears in it.

usage: python export.py <BS-Rofo-SW-Fixed.ckpt> <out.safetensors> [recording.wav]
"""
import json
import os
import sys

import numpy as np
import torch
import yaml
from ml_collections import ConfigDict
from safetensors.torch import save_file

import bs_roformer
from bs_roformer.inference import SafeLoaderWithTuple
from bs_roformer.utils import get_model_from_config, load_checkpoint_state


def main(ckpt, out, wav=None):
    cfg_path = os.path.join(os.path.dirname(bs_roformer.__file__), "configs", "BS-Rofo-SW-Fixed.yaml")
    cfg = ConfigDict(yaml.load(open(cfg_path), Loader=SafeLoaderWithTuple))
    sd = load_checkpoint_state(ckpt, map_location="cpu")
    stems = list(cfg.training.instruments)
    piano = stems.index("piano")
    model = get_model_from_config("bs_roformer", cfg)
    model.load_state_dict(sd)
    model.eval()
    bands = [int(f) for f in model.band_split.dim_inputs]
    t = {}
    for i in range(len(bands)):
        p = f"band_split.to_features.{i}"
        t[f"band.{i}.gamma"] = sd[f"{p}.0.gamma"]
        t[f"band.{i}.w"] = sd[f"{p}.1.weight"]
        t[f"band.{i}.b"] = sd[f"{p}.1.bias"]
    for l in range(cfg.model.depth):
        for j, axis in enumerate(["time", "freq"]):
            p = f"layers.{l}.{j}.layers.0"
            q = f"layer.{l}.{axis}"
            # One rotary embedding per axis, shared by its layers.
            t[f"rope.{axis}"] = sd[f"{p}.0.rotary_embed.freqs"]
            t[f"{q}.norm"] = sd[f"{p}.0.norm.gamma"]
            t[f"{q}.qkv"] = sd[f"{p}.0.to_qkv.weight"]
            t[f"{q}.gates.w"] = sd[f"{p}.0.to_gates.weight"]
            t[f"{q}.gates.b"] = sd[f"{p}.0.to_gates.bias"]
            t[f"{q}.out"] = sd[f"{p}.0.to_out.0.weight"]
            t[f"{q}.ff.norm"] = sd[f"{p}.1.net.0.gamma"]
            t[f"{q}.ff.w1"] = sd[f"{p}.1.net.1.weight"]
            t[f"{q}.ff.b1"] = sd[f"{p}.1.net.1.bias"]
            t[f"{q}.ff.w2"] = sd[f"{p}.1.net.4.weight"]
            t[f"{q}.ff.b2"] = sd[f"{p}.1.net.4.bias"]
    t["final_norm"] = sd["final_norm.gamma"]
    for i in range(len(bands)):
        p = f"mask_estimators.{piano}.to_freqs.{i}.0"
        t[f"mask.{i}.w1"] = sd[f"{p}.0.weight"]
        t[f"mask.{i}.b1"] = sd[f"{p}.0.bias"]
        t[f"mask.{i}.w2"] = sd[f"{p}.2.weight"]
        t[f"mask.{i}.b2"] = sd[f"{p}.2.bias"]
    t = {k: v.float().contiguous().clone() for k, v in t.items()}
    meta = {
        "model": "BS-RoFormer-SW (6 stems), piano stem only",
        "settings": json.dumps({
            "sample_rate": 44100,
            "chunk": int(cfg.inference.chunk_size),
            "n_fft": int(cfg.model.stft_n_fft),
            "hop": int(cfg.model.stft_hop_length),
            "dim": int(cfg.model.dim),
            "depth": int(cfg.model.depth),
            "heads": int(cfg.model.heads),
            "dim_head": int(cfg.model.dim_head),
            "bands": bands,
        }),
    }
    save_file(t, out, metadata=meta)
    print(f"wrote {out}: {sum(v.numel() for v in t.values()) / 1e6:.1f} M parameters")
    if wav:
        import soundfile as sf
        mix, sr = sf.read(wav, dtype="float32", always_2d=True)
        assert sr == 44100, sr
        chunk = cfg.inference.chunk_size
        start = min(60 * sr, max(0, len(mix) - chunk))
        x = torch.from_numpy(mix[start:start + chunk].T.copy()).unsqueeze(0)
        with torch.no_grad():
            y = model(x)[0, piano]
        save_file({"input": x[0].contiguous(), "piano": y.contiguous()}, out + ".check")
        print(f"wrote {out}.check: {chunk / sr:.2f} s from {start / sr:.0f} s")


if __name__ == "__main__":
    main(*sys.argv[1:4])
