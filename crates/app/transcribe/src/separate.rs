//! The piano out of a mix: BS-RoFormer (W.-T. Lu, J.-C. Wang, Q. Kong,
//! Y.-N. Hung, ByteDance 2023; lucidrains' implementation, MIT) as trained
//! for six stems (BS-RoFormer-SW, a community model), its piano stem alone,
//! run on the GPU through candle (Metal on a Mac), so a song's voice, drums
//! and the rest do not reach the transcription as notes.
//!
//! A chunk of stereo 44.1 kHz audio becomes its short-time spectrum (here,
//! on the CPU); the model hears it as 62 bands of frequency over time,
//! through twelve pairs of transformers (across time within each band,
//! then across bands at each moment), and answers with a complex mask that
//! keeps the piano; the masked spectrum is turned back into audio. Chunks
//! overlap by half and cross-fade, as the model's authors join them.
//! `tools/separate-model` makes the weights file.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use candle_core::{D, DType, Device, Tensor};
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// The settings the weights file carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub sample_rate: u32,
    /// Samples a chunk.
    pub chunk: usize,
    pub n_fft: usize,
    pub hop: usize,
    pub dim: usize,
    pub depth: usize,
    pub heads: usize,
    pub dim_head: usize,
    /// Each band's width in model inputs: 4 per frequency bin (two
    /// channels, real and imaginary).
    pub bands: Vec<usize>,
}

/// One transformer layer's weights, each linear map's transposed. The
/// query's and key's channels are reordered within each head, evens then
/// odds, so the model's interleaved rotary positions become the halved
/// kind on the layout the products come in (their dot products are the
/// same).
struct Layer {
    norm: Tensor,
    q: Tensor,
    k: Tensor,
    v: Tensor,
    gates_w: Tensor,
    gates_b: Tensor,
    out: Tensor,
    ff_norm: Tensor,
    ff_w1: Tensor,
    ff_b1: Tensor,
    ff_w2: Tensor,
    ff_b2: Tensor,
}

/// A band's input projection, or its mask estimator.
struct Band {
    gamma: Tensor,
    w: Tensor,
    b: Tensor,
}

struct Mask {
    w1: Tensor,
    b1: Tensor,
    w2: Tensor,
    b2: Tensor,
}

/// The separator, loaded once.
pub struct Separator {
    device: Device,
    pub settings: Settings,
    bands: Vec<Band>,
    /// Time then frequency, for each depth.
    layers: Vec<(Layer, Layer)>,
    final_norm: Tensor,
    masks: Vec<Mask>,
    /// The rotary embeddings' frequencies, across time and across bands.
    rope_time: Tensor,
    rope_freq: Tensor,
}

type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The settings in a safetensors file's metadata.
fn settings(path: &Path) -> Result<Settings> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(err)?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).map_err(err)?;
    let len = usize::try_from(u64::from_le_bytes(len)).map_err(err)?;
    if len > 100 << 20 {
        return Err("its header is too large".into());
    }
    let mut header = vec![0u8; len];
    f.read_exact(&mut header).map_err(err)?;
    let header: serde_json::Value = serde_json::from_slice(&header).map_err(err)?;
    let raw = header
        .get("__metadata__")
        .and_then(|m| m.get("settings"))
        .and_then(serde_json::Value::as_str)
        .ok_or("it has no settings")?;
    let s: serde_json::Value = serde_json::from_str(raw).map_err(err)?;
    let n = |k: &str| -> Result<usize> {
        s.get(k)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| format!("its settings lack {k}"))
    };
    let bands = s
        .get("bands")
        .and_then(serde_json::Value::as_array)
        .ok_or("its settings lack bands")?
        .iter()
        .map(|b| b.as_u64().and_then(|v| usize::try_from(v).ok()))
        .collect::<Option<Vec<_>>>()
        .ok_or("its bands are not numbers")?;
    Ok(Settings {
        sample_rate: u32::try_from(n("sample_rate")?).map_err(err)?,
        chunk: n("chunk")?,
        n_fft: n("n_fft")?,
        hop: n("hop")?,
        dim: n("dim")?,
        depth: n("depth")?,
        heads: n("heads")?,
        dim_head: n("dim_head")?,
        bands,
    })
}

/// The GPU where there is one (Metal on a Mac), else the CPU.
fn device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(d) = Device::new_metal(0) {
        return d;
    }
    Device::Cpu
}

impl Separator {
    /// The weights at `path` (tools/separate-model makes them).
    ///
    /// # Errors
    /// When they cannot be read or are not that model.
    pub fn load(path: &Path) -> Result<Self> {
        let fail = |e: String| format!("{}: {e}", path.display());
        let settings = settings(path).map_err(fail)?;
        let device = device();
        let mut t = candle_core::safetensors::load(path, &device).map_err(|e| fail(err(e)))?;
        let mut take = |k: &str| t.remove(k).ok_or_else(|| fail(format!("it lacks {k}")));
        let mut bands = Vec::with_capacity(settings.bands.len());
        let mut masks = Vec::with_capacity(settings.bands.len());
        for i in 0..settings.bands.len() {
            bands.push(Band {
                gamma: take(&format!("band.{i}.gamma"))?,
                w: take(&format!("band.{i}.w"))?.t().map_err(err)?,
                b: take(&format!("band.{i}.b"))?,
            });
            masks.push(Mask {
                w1: take(&format!("mask.{i}.w1"))?.t().map_err(err)?,
                b1: take(&format!("mask.{i}.b1"))?,
                w2: take(&format!("mask.{i}.w2"))?.t().map_err(err)?,
                b2: take(&format!("mask.{i}.b2"))?,
            });
        }
        let (heads, dh) = (settings.heads, settings.dim_head);
        let inner = heads * dh;
        // Within each head, the even channels then the odd ones.
        let order: Vec<u32> = (0..heads)
            .flat_map(|h| {
                (0..dh).map(move |p| {
                    (h * dh
                        + if p < dh / 2 {
                            2 * p
                        } else {
                            2 * (p - dh / 2) + 1
                        }) as u32
                })
            })
            .collect();
        let order = Tensor::new(order.as_slice(), &device).map_err(err)?;
        let mut layer = |l: usize, axis: &str| -> Result<Layer> {
            let mut w = |k: &str| take(&format!("layer.{l}.{axis}.{k}"));
            let t = |x: Tensor| x.t().and_then(|x| x.contiguous()).map_err(err);
            let qkv = w("qkv")?;
            let part = |n: usize| qkv.narrow(0, n * inner, inner).map_err(err);
            let reorder = |x: Tensor| x.index_select(&order, 0).map_err(err);
            Ok(Layer {
                norm: w("norm")?,
                q: t(reorder(part(0)?)?)?,
                k: t(reorder(part(1)?)?)?,
                v: t(part(2)?)?,
                gates_w: t(w("gates.w")?)?,
                gates_b: w("gates.b")?,
                out: t(w("out")?)?,
                ff_norm: w("ff.norm")?,
                ff_w1: t(w("ff.w1")?)?,
                ff_b1: w("ff.b1")?,
                ff_w2: t(w("ff.w2")?)?,
                ff_b2: w("ff.b2")?,
            })
        };
        let mut layers = Vec::with_capacity(settings.depth);
        for l in 0..settings.depth {
            layers.push((layer(l, "time")?, layer(l, "freq")?));
        }
        Ok(Self {
            final_norm: take("final_norm")?,
            rope_time: take("rope.time")?,
            rope_freq: take("rope.freq")?,
            device,
            settings,
            bands,
            layers,
            masks,
        })
    }

    /// Whether it runs on a GPU.
    #[must_use]
    pub fn on_gpu(&self) -> bool {
        !self.device.is_cpu()
    }

    /// `x` normalised to unit length over its last axis, scaled by the
    /// square root of its width and by `gamma` (lucidrains' RMSNorm).
    fn norm(x: &Tensor, gamma: &Tensor) -> candle_core::Result<Tensor> {
        candle_nn::ops::rms_norm(&x.contiguous()?, gamma, 1e-12)
    }

    /// `x` (batch, n, d) through one transformer layer, with rotary
    /// positions from `freqs`. Its linear maps run as one large matrix
    /// product over every token, not one per sequence.
    fn layer(&self, x: &Tensor, w: &Layer, freqs: &Tensor) -> candle_core::Result<Tensor> {
        let (b, n, d) = x.dims3()?;
        let (heads, dh) = (self.settings.heads, self.settings.dim_head);
        let x = x.reshape((b * n, d))?;
        let h = Self::norm(&x, &w.norm)?;
        let pos = Tensor::arange(0u32, n as u32, &self.device)?.to_dtype(DType::F32)?;
        let angles = pos.unsqueeze(1)?.broadcast_mul(&freqs.unsqueeze(0)?)?;
        let (cos, sin) = (angles.cos()?, angles.sin()?);
        // (batch, n, heads, dh), as the products come, then heads first.
        let heads_first = |t: Tensor| t.transpose(1, 2);
        let split = |w: &Tensor| h.matmul(w)?.reshape((b, n, heads, dh));
        let q = heads_first(candle_nn::rotary_emb::rope_thd(&split(&w.q)?, &cos, &sin)?)?;
        let k = heads_first(candle_nn::rotary_emb::rope_thd(&split(&w.k)?, &cos, &sin)?)?;
        let v = heads_first(split(&w.v)?)?;
        let scale = 1. / (dh as f32).sqrt();
        let out = self.attend(&q, &k, &v, scale)?;
        let gates = candle_nn::ops::sigmoid(&h.matmul(&w.gates_w)?.broadcast_add(&w.gates_b)?)?;
        let gates = gates
            .reshape((b, n, heads))?
            .transpose(1, 2)?
            .unsqueeze(3)?;
        let out = out.broadcast_mul(&gates)?;
        let out = out.transpose(1, 2)?.reshape((b * n, heads * dh))?;
        let x = (&x + out.matmul(&w.out)?)?;
        let f = Self::norm(&x, &w.ff_norm)?
            .matmul(&w.ff_w1)?
            .broadcast_add(&w.ff_b1)?
            .gelu_erf()?
            .matmul(&w.ff_w2)?
            .broadcast_add(&w.ff_b2)?;
        (x + f)?.reshape((b, n, d))
    }

    /// softmax(q kᵀ · scale) v: fused on the GPU, in slices on the CPU so
    /// a long sequence's scores stay small.
    fn attend(
        &self,
        q: &Tensor,
        k: &Tensor,
        v: &Tensor,
        scale: f32,
    ) -> candle_core::Result<Tensor> {
        if !self.device.is_cpu() {
            return candle_nn::ops::sdpa(q, k, v, None, false, scale, 1.);
        }
        let b = q.dim(0)?;
        let step = 8;
        let mut outs = Vec::new();
        for s in (0..b).step_by(step) {
            let len = step.min(b - s);
            let part = |t: &Tensor| t.narrow(0, s, len)?.contiguous();
            let (q, k, v) = (part(q)?, part(k)?, part(v)?);
            let att = (q.matmul(&k.t()?)? * f64::from(scale))?;
            let att = candle_nn::ops::softmax_last_dim(&att)?;
            outs.push(att.matmul(&v)?);
        }
        Tensor::cat(&outs, 0)
    }

    /// The piano's complex mask for `x` (frames, inputs), as the model
    /// lays its inputs out: bins, then channels, then real and imaginary.
    fn mask(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let mut feats = Vec::with_capacity(self.bands.len());
        let mut from = 0;
        for (band, &width) in self.bands.iter().zip(&self.settings.bands) {
            let xi = x.narrow(1, from, width)?;
            from += width;
            let h = Self::norm(&xi, &band.gamma)?
                .matmul(&band.w)?
                .broadcast_add(&band.b)?;
            feats.push(h);
        }
        // (frames, bands, dim)
        let mut h = Tensor::stack(&feats, 1)?;
        for (time, freq) in &self.layers {
            let t = h.transpose(0, 1)?.contiguous()?;
            let t = self.layer(&t, time, &self.rope_time)?;
            h = t.transpose(0, 1)?.contiguous()?;
            h = self.layer(&h, freq, &self.rope_freq)?;
        }
        let h = Self::norm(&h, &self.final_norm)?;
        let mut outs = Vec::with_capacity(self.masks.len());
        for (i, (m, &width)) in self.masks.iter().zip(&self.settings.bands).enumerate() {
            let hb = h.narrow(1, i, 1)?.squeeze(1)?.contiguous()?;
            let y = hb
                .matmul(&m.w1)?
                .broadcast_add(&m.b1)?
                .tanh()?
                .matmul(&m.w2)?
                .broadcast_add(&m.b2)?;
            // GLU: the first half, gated by the second.
            let gate = candle_nn::ops::sigmoid(&y.narrow(D::Minus1, width, width)?)?;
            outs.push(y.narrow(D::Minus1, 0, width)?.mul(&gate)?);
        }
        Tensor::cat(&outs, 1)
    }

    /// One chunk (each channel [`Settings::chunk`] samples) of stereo
    /// audio: its piano, stereo.
    ///
    /// # Errors
    /// When the model fails.
    pub fn chunk(&self, stft: &mut Stft, l: &[f32], r: &[f32]) -> Result<[Vec<f32>; 2]> {
        let (sl, sr) = (stft.forward(l), stft.forward(r));
        let frames = sl.len();
        let bins = self.settings.n_fft / 2 + 1;
        let width = bins * 4;
        // Bins, then channels, then real and imaginary, each frame.
        let mut x = vec![0f32; frames * width];
        for (t, row) in x.chunks_exact_mut(width).enumerate() {
            let (a, b) = (sl.get(t), sr.get(t));
            for (f, cell) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (cl, cr) = (
                    a.and_then(|a| a.get(f)).copied().unwrap_or_default(),
                    b.and_then(|b| b.get(f)).copied().unwrap_or_default(),
                );
                *cell = [cl.re, cl.im, cr.re, cr.im];
            }
        }
        let x = Tensor::from_vec(x, (frames, width), &self.device).map_err(err)?;
        let m = self.mask(&x).map_err(err)?;
        let m: Vec<f32> = m.flatten_all().map_err(err)?.to_vec1().map_err(err)?;
        let mut out = [Vec::new(), Vec::new()];
        for (c, spec) in [sl, sr].into_iter().enumerate() {
            let masked: Vec<Vec<Complex<f32>>> = spec
                .iter()
                .enumerate()
                .map(|(t, frame)| {
                    frame
                        .iter()
                        .enumerate()
                        .map(|(f, z)| {
                            let at = t * width + f * 4 + c * 2;
                            let mask = Complex::new(
                                m.get(at).copied().unwrap_or(0.),
                                m.get(at + 1).copied().unwrap_or(0.),
                            );
                            z * mask
                        })
                        .collect()
                })
                .collect();
            if let Some(o) = out.get_mut(c) {
                *o = stft.inverse(&masked, l.len());
            }
        }
        Ok(out)
    }

    /// The piano in a stereo recording at [`Settings::sample_rate`]: chunks
    /// half a chunk apart, cross-faded over a tenth of one, as the model's
    /// authors run it. `progress` hears the share done; `cancel` stops it.
    ///
    /// # Errors
    /// When cancelled, or the model fails.
    pub fn piano(
        &self,
        left: &[f32],
        right: &[f32],
        progress: &dyn Fn(f64),
        cancel: &AtomicBool,
    ) -> Result<[Vec<f32>; 2]> {
        let c = self.settings.chunk;
        let step = c / 2;
        let fade = c / 10;
        let border = c - step;
        let len = left.len().min(right.len());
        // Reflected at both ends so the ends are heard mid-chunk too.
        let padded = len > 2 * border && border > 0;
        let pad = |x: &[f32]| -> Vec<f32> {
            let x = x.get(..len).unwrap_or(x);
            if !padded {
                return x.to_vec();
            }
            let mut out = Vec::with_capacity(len + 2 * border);
            out.extend((1..=border).rev().map(|i| x.get(i).copied().unwrap_or(0.)));
            out.extend_from_slice(x);
            out.extend((1..=border).map(|i| x.get(len - 1 - i).copied().unwrap_or(0.)));
            out
        };
        let (l, r) = (pad(left), pad(right));
        let total = l.len();
        let window: Vec<f32> = (0..c)
            .map(|i| {
                let ramp = |k: usize| k as f32 / (fade - 1).max(1) as f32;
                if i < fade {
                    ramp(i)
                } else if i >= c - fade {
                    ramp(c - 1 - i)
                } else {
                    1.
                }
            })
            .collect();
        let mut sum = [vec![0f32; total], vec![0f32; total]];
        let mut weight = vec![0f32; total];
        let mut stft = Stft::new(self.settings.n_fft, self.settings.hop);
        let chunks = total.div_ceil(step).max(1);
        let mut i = 0;
        let mut done = 0;
        while i < total {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let length = c.min(total - i);
            let part = |x: &[f32]| -> Vec<f32> {
                let mut p = x.get(i..i + length).unwrap_or_default().to_vec();
                if length < c {
                    if length > c / 2 + 1 {
                        // Reflected out to a whole chunk.
                        let n = p.len();
                        for k in 0..(c - n) {
                            p.push(
                                p.get(n.saturating_sub(2 + k % (n - 1)))
                                    .copied()
                                    .unwrap_or(0.),
                            );
                        }
                    } else {
                        p.resize(c, 0.);
                    }
                }
                p
            };
            let [pl, pr] = self.chunk(&mut stft, &part(&l), &part(&r))?;
            for k in 0..length {
                let mut w = window.get(k).copied().unwrap_or(1.);
                if (i == 0 && k < fade) || (i + c >= total && k >= c - fade) {
                    w = 1.;
                }
                let at = i + k;
                for (s, p) in sum.iter_mut().zip([&pl, &pr]) {
                    if let (Some(o), Some(v)) = (s.get_mut(at), p.get(k)) {
                        *o += v * w;
                    }
                }
                if let Some(o) = weight.get_mut(at) {
                    *o += w;
                }
            }
            i += step;
            done += 1;
            progress(done as f64 / chunks as f64);
        }
        let from = if padded { border } else { 0 };
        let out = sum.map(|s| {
            s.iter()
                .zip(&weight)
                .skip(from)
                .take(len)
                .map(|(v, w)| if *w > 0. { v / w } else { 0. })
                .collect()
        });
        Ok(out)
    }
}

/// The short-time Fourier transform as torch.stft and torch.istft do it:
/// a periodic Hann window, frames centred on their hop by reflecting the
/// ends, one-sided.
pub struct Stft {
    n_fft: usize,
    hop: usize,
    window: Vec<f32>,
    forward: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    inverse: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
}

impl Stft {
    #[must_use]
    pub fn new(n_fft: usize, hop: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let window = (0..n_fft)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n_fft as f64).cos()) as f32)
            .collect();
        Self {
            n_fft,
            hop,
            window,
            forward: planner.plan_fft_forward(n_fft),
            inverse: planner.plan_fft_inverse(n_fft),
        }
    }

    /// `x`'s frames, each `n_fft / 2 + 1` bins.
    #[must_use]
    pub fn forward(&mut self, x: &[f32]) -> Vec<Vec<Complex<f32>>> {
        let pad = self.n_fft / 2;
        let n = x.len();
        let at = |j: usize| -> f32 {
            // Reflected about the ends, as torch pads a centred frame.
            let i = j as i64 - pad as i64;
            let i = if i < 0 { -i } else { i };
            let last = n as i64 - 1;
            let i = if i > last { 2 * last - i } else { i };
            usize::try_from(i)
                .ok()
                .and_then(|i| x.get(i))
                .copied()
                .unwrap_or(0.)
        };
        let frames = 1 + n / self.hop;
        let mut buf = vec![0f32; self.n_fft];
        let mut out = Vec::with_capacity(frames);
        for f in 0..frames {
            for (k, b) in buf.iter_mut().enumerate() {
                *b = at(f * self.hop + k) * self.window.get(k).copied().unwrap_or(0.);
            }
            let mut spec = self.forward.make_output_vec();
            if self.forward.process(&mut buf, &mut spec).is_err() {
                spec.fill(Complex::default());
            }
            out.push(spec);
        }
        out
    }

    /// `frames` back to `len` samples: each frame windowed and added where
    /// it falls, divided by the windows' overlapping power.
    #[must_use]
    pub fn inverse(&mut self, frames: &[Vec<Complex<f32>>], len: usize) -> Vec<f32> {
        let pad = self.n_fft / 2;
        let total = self.n_fft + self.hop * frames.len().saturating_sub(1);
        let mut sum = vec![0f32; total];
        let mut env = vec![0f32; total];
        let mut buf = vec![0f32; self.n_fft];
        let scale = 1. / self.n_fft as f32;
        for (f, frame) in frames.iter().enumerate() {
            let mut spec = frame.clone();
            // A real signal's DC and Nyquist bins are real.
            if let Some(z) = spec.first_mut() {
                z.im = 0.;
            }
            if let Some(z) = spec.last_mut() {
                z.im = 0.;
            }
            if self.inverse.process(&mut spec, &mut buf).is_err() {
                buf.fill(0.);
            }
            for (k, v) in buf.iter().enumerate() {
                let w = self.window.get(k).copied().unwrap_or(0.);
                let at = f * self.hop + k;
                if let (Some(s), Some(e)) = (sum.get_mut(at), env.get_mut(at)) {
                    *s += v * scale * w;
                    *e += w * w;
                }
            }
        }
        (0..len)
            .map(|i| {
                let at = i + pad;
                match (sum.get(at), env.get(at)) {
                    (Some(s), Some(e)) if *e > 1e-11 => s / e,
                    _ => 0.,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stft_round_trips() {
        let n = 588_800 / 8;
        let x: Vec<f32> = (0..n)
            .map(|i| ((i as f32 * 0.013).sin() + (i as f32 * 0.21).cos() * 0.3) * 0.5)
            .collect();
        let mut s = Stft::new(2048, 512);
        let spec = s.forward(&x);
        assert_eq!(spec.len(), 1 + n / 512);
        assert_eq!(spec[0].len(), 1025);
        let back = s.inverse(&spec, n);
        let err = x
            .iter()
            .zip(&back)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(err < 1e-4, "{err}");
    }
}
