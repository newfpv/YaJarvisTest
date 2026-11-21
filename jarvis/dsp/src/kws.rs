//! Потоковый детектор слова (KWS). На каждые 10 мс звука:
//!   1) лог-мел спектр (окно 25 мс, FFT 512, 40 полос 20–7600 Гц), нормировка mean/std из модели;
//!   2) причинная свёрточная сеть по времени с растущим шагом (dilation): считаем только новый кадр,
//!      прошлое лежит в кольцевых буферах слоёв;
//!   3) вероятность «слово только что закончилось», сглаженная за 3 кадра, сравнивается с порогом.
//! Формат wake.bin: "JKWS1" + u32 длина JSON-заголовка + JSON + f32 веса (little endian).

use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Header {
    pub sample_rate: usize,
    pub win: usize,
    pub hop: usize,
    pub n_fft: usize,
    pub n_mels: usize,
    pub fmin: f64,
    pub fmax: f64,
    pub channels: usize,
    pub dilations: Vec<usize>,
    pub kernel: usize,
    pub thresholds: Vec<f64>,
}

struct Conv {
    cin: usize,
    cout: usize,
    k: usize,
    dil: usize,
    w: Vec<f32>, // [cout][k][cin] — соседние каналы подряд для векторизации dot()
    b: Vec<f32>,
    span: usize,
    hist: Vec<f32>, // плоское кольцо длиной span * cin
    pos: usize,
}

impl Conv {
    fn step(&mut self, x: &[f32], out: &mut [f32]) {
        let span = self.span;
        let cin = self.cin;
        self.hist[self.pos * cin..(self.pos + 1) * cin].copy_from_slice(x);
        for o in 0..self.cout {
            let mut acc = self.b[o];
            let wo = &self.w[o * self.k * cin..];
            for t in 0..self.k {
                // t=k-1 — текущий кадр, t=0 — самый старый
                let lag = (self.k - 1 - t) * self.dil;
                let idx = (self.pos + span - lag) % span;
                let inp = &self.hist[idx * cin..(idx + 1) * cin];
                acc += crate::dot(&wo[t * cin..(t + 1) * cin], inp);
            }
            out[o] = acc;
        }
        self.pos = (self.pos + 1) % span;
    }

    fn reset(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0;
    }
}

pub struct Kws {
    pub h: Header,
    mean: Vec<f32>,
    std: Vec<f32>,
    mel_sparse: Vec<(usize, Vec<f32>)>,
    window: Vec<f32>,
    proj: Conv,
    blocks: Vec<Conv>,
    head: Conv,
    buf: Vec<f32>,
    feat: Vec<f32>,
    a: Vec<f32>,
    b2: Vec<f32>,
    fft: crate::Rfft,
    frame_buf: Vec<f32>,
    power: Vec<f32>,
    smooth: [f32; 3],
    si: usize,
    sensitivity: Box<dyn Fn() -> i64 + Send>,
    pub cooldown: u32,
    pub last_prob: f32,
}

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// Треугольные фильтры HTK (так же считает kws/features.py).
pub fn mel_bank(sr: usize, nfft: usize, nmels: usize, fmin: f64, fmax: f64) -> Vec<Vec<f32>> {
    let bins = nfft / 2 + 1;
    let (lo, hi) = (hz_to_mel(fmin), hz_to_mel(fmax));
    let pts: Vec<f64> = (0..nmels + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (nmels + 1) as f64)).collect();
    (0..nmels)
        .map(|m| {
            let (l, c, r) = (pts[m], pts[m + 1], pts[m + 2]);
            (0..bins)
                .map(|b| {
                    let f = b as f64 * sr as f64 / nfft as f64;
                    let v = if f > l && f < c {
                        (f - l) / (c - l)
                    } else if f >= c && f < r {
                        (r - f) / (r - c)
                    } else {
                        0.0
                    };
                    v as f32
                })
                .collect()
        })
        .collect()
}

impl Kws {
    pub fn load(data: &[u8], sensitivity: Box<dyn Fn() -> i64 + Send>) -> Result<Kws, String> {
        if data.len() < 9 || &data[..5] != b"JKWS1" {
            return Err("wake.bin: не тот формат".into());
        }
        let n = u32::from_le_bytes(data[5..9].try_into().unwrap()) as usize;
        if data.len()>4*1024*1024 || n>64*1024 {return Err("wake.bin: модель слишком большая".into());}
        if 9 + n > data.len() {
            return Err("wake.bin: обрезан".into());
        }
        let h: Header = serde_json::from_slice(&data[9..9 + n]).map_err(|e| e.to_string())?;
        if data.len()>4*1024*1024 || n>64*1024 || h.sample_rate!=16000
            || !(16..=2048).contains(&h.n_fft) || !h.n_fft.is_power_of_two()
            || h.win==0 || h.win>h.n_fft || h.hop==0 || h.hop>h.win
            || !(2..=128).contains(&h.n_mels) || !(1..=128).contains(&h.channels)
            || !(1..=9).contains(&h.kernel) || h.dilations.len()>16
            || h.dilations.iter().any(|d|!(1..=256).contains(d))
            || !h.fmin.is_finite() || !h.fmax.is_finite() || h.fmin<0.0 || h.fmax<=h.fmin || h.fmax>8000.0
            || h.thresholds.len()!=10 || h.thresholds.iter().any(|n|!n.is_finite() || !(0.0..=1.0).contains(n)) {
            return Err("wake.bin: неподдерживаемые параметры модели".into());
        }
        let history=h.dilations.iter().map(|d|((h.kernel-1)*d+1)*h.channels).sum::<usize>();
        if history>1024*1024{return Err("wake.bin: модель требует слишком много памяти".into());}
        let raw = &data[9 + n..];
        if raw.len() % 4 != 0 {
            return Err("wake.bin: веса повреждены".into());
        }
        let floats: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        if floats.iter().any(|v|!v.is_finite()){return Err("wake.bin: веса содержат неверные числа".into());}
        let mut off = 0;
        let mut take = |n: usize| -> Result<Vec<f32>, String> {
            if floats.len() - off < n {
                return Err("wake.bin: весов меньше, чем нужно".into());
            }
            off += n;
            Ok(floats[off - n..off].to_vec())
        };
        let mean = take(h.n_mels)?;
        let std = take(h.n_mels)?;
        if std.iter().any(|v|*v<=0.0){return Err("wake.bin: неверная нормализация".into());}
        let mut mk = |cin: usize, cout: usize, k: usize, dil: usize| -> Result<Conv, String> {
            let raw_w = take(cout * cin * k)?;
            let mut w = vec![0.0; raw_w.len()];
            for o in 0..cout {
                for t in 0..k {
                    for c in 0..cin {
                        w[(o * k + t) * cin + c] = raw_w[(o * cin + c) * k + t];
                    }
                }
            }
            let b = take(cout)?;
            let span = (k - 1) * dil + 1;
            Ok(Conv { cin, cout, k, dil, w, b, span, hist: vec![0.0; span * cin], pos: 0 })
        };
        let proj = mk(h.n_mels, h.channels, 1, 1)?;
        let mut blocks = vec![];
        for &d in &h.dilations {
            blocks.push(mk(h.channels, h.channels, h.kernel, d)?);
        }
        let head = mk(h.channels, 1, 1, 1)?;
        if off != floats.len() {
            return Err(format!("wake.bin: лишние веса ({})", floats.len() - off));
        }
        if h.thresholds.len() != 10 {
            return Err("wake.bin: нужно 10 порогов".into());
        }
        // окно Ханна, как torch.hann_window(periodic=True)
        let window = (0..h.win)
            .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / h.win as f64).cos()) as f32)
            .collect();
        let bins = h.n_fft / 2 + 1;
        let mel_sparse = mel_bank(h.sample_rate, h.n_fft, h.n_mels, h.fmin, h.fmax)
            .into_iter()
            .map(|row| {
                let first = row.iter().position(|&v| v != 0.0).unwrap_or(0);
                let last = row.iter().rposition(|&v| v != 0.0).map_or(first, |i| i + 1);
                (first, row[first..last].to_vec())
            })
            .collect();
        Ok(Kws {
            mel_sparse,
            mean,
            std,
            window,
            proj,
            blocks,
            head,
            buf: Vec::with_capacity(4096),
            feat: vec![0.0; h.n_mels],
            a: vec![0.0; h.channels],
            b2: vec![0.0; h.channels],
            fft: crate::Rfft::new(h.n_fft),
            frame_buf: vec![0.0; h.n_fft],
            power: vec![0.0; bins],
            smooth: [0.0; 3],
            si: 0,
            sensitivity,
            cooldown: 0,
            last_prob: 0.0,
            h,
        })
    }

    /// Один кадр (окно win): обновить сеть; вернуть сглаженную вероятность.
    pub fn frame(&mut self, x: &[f64]) -> f32 {
        for (dst, (&v, &w)) in self.frame_buf[..self.h.win].iter_mut().zip(x.iter().zip(&self.window)) {
            *dst = v as f32 * w;
        }
        self.frame_loaded()
    }

    fn frame_loaded(&mut self) -> f32 {
        self.fft.power(&self.frame_buf, &mut self.power);
        for (m, (start, weights)) in self.mel_sparse.iter().enumerate() {
            let acc = crate::dot(weights, &self.power[*start..*start + weights.len()]);
            self.feat[m] = (((acc as f64) + 1e-6).ln() as f32 - self.mean[m]) / self.std[m];
        }
        self.proj.step(&self.feat, &mut self.a);
        self.a.iter_mut().for_each(|v| *v = v.max(0.0));
        for l in &mut self.blocks {
            l.step(&self.a, &mut self.b2);
            for (a, &v) in self.a.iter_mut().zip(&self.b2) {
                *a += v.max(0.0); // остаточная связь
            }
        }
        let mut out = [0f32];
        self.head.step(&self.a, &mut out);
        let p = (1.0 / (1.0 + (-(out[0] as f64)).exp())) as f32;
        self.smooth[self.si % 3] = p;
        self.si += 1;
        let avg = (self.smooth[0] + self.smooth[1] + self.smooth[2]) / 3.0;
        self.last_prob = avg;
        avg
    }

    /// Последняя несглаженная вероятность кадра (для сверки с Python).
    pub fn last_raw(&self) -> f32 {
        self.smooth[(self.si + 2) % 3]
    }
}

impl crate::Wake for Kws {
    fn feed(&mut self, pcm: &[i16]) -> bool {
        self.buf.extend(pcm.iter().map(|&s| s as f32 / 32768.0));
        let mut hit = false;
        let mut off = 0;
        let (win, hop) = (self.h.win, self.h.hop);
        while self.buf.len() - off >= win {
            for (dst, (&x, &w)) in self.frame_buf[..win]
                .iter_mut()
                .zip(self.buf[off..off + win].iter().zip(&self.window))
            {
                *dst = x * w;
            }
            let avg = self.frame_loaded();
            off += hop;
            if self.cooldown > 0 {
                self.cooldown -= 1;
                continue;
            }
            let s = (self.sensitivity)().clamp(1, 10) as usize;
            if avg >= self.h.thresholds[s - 1] as f32 {
                self.cooldown = 150; // 1,5 с
                hit = true;
            }
        }
        let rem = self.buf.len() - off;
        if off > 0 {
            self.buf.copy_within(off.., 0);
            self.buf.truncate(rem);
        }
        hit
    }

    fn reset(&mut self) {
        self.proj.reset();
        for l in &mut self.blocks {
            l.reset();
        }
        self.head.reset();
        self.smooth = [0.0; 3];
        self.cooldown = 100; // 1 с после разговора не слушаем — хвост своей же речи
    }

    fn last_prob(&self) -> f32 {
        self.last_prob
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn rejects_invalid_geometry_before_allocating_or_running(){
        let original=include_bytes!("../../../models/wake.bin");let n=u32::from_le_bytes(original[5..9].try_into().unwrap()) as usize;
        for (field,value) in [("hop",serde_json::json!(0)),("n_fft",serde_json::json!(513)),("channels",serde_json::json!(usize::MAX)),("dilations",serde_json::json!([0]))]{
            let mut header:serde_json::Value=serde_json::from_slice(&original[9..9+n]).unwrap();header[field]=value;let header=serde_json::to_vec(&header).unwrap();
            let mut bytes=b"JKWS1".to_vec();bytes.extend((header.len() as u32).to_le_bytes());bytes.extend(header);bytes.extend(&original[9+n..]);assert!(Kws::load(&bytes,Box::new(||5)).is_err());
        }
        assert!(Kws::load(original,Box::new(||5)).is_ok());
    }

    /// Совпадение с PyTorch: JARVIS_KWS_DIR=…/kws/out cargo test -p jarvis-dsp kws
    #[test]
    fn matches_python() {
        let Ok(dir) = std::env::var("JARVIS_KWS_DIR") else {
            eprintln!("нет JARVIS_KWS_DIR — пропускаю");
            return;
        };
        let dir = std::path::Path::new(&dir);
        let bin = std::fs::read(dir.join("wake.bin")).unwrap();
        #[derive(Deserialize)]
        struct Tv {
            pcm: Vec<i16>,
            probs: Vec<f64>,
        }
        let tv: Tv = serde_json::from_slice(&std::fs::read(dir.join("testvec.json")).unwrap()).unwrap();
        let mut k = Kws::load(&bin, Box::new(|| 5)).unwrap();
        let mut got = vec![];
        let mut buf: Vec<f64> = vec![];
        let t0 = std::time::Instant::now();
        for chunk in tv.pcm.chunks(320) {
            buf.extend(chunk.iter().map(|&s| s as f64 / 32768.0));
            while buf.len() >= k.h.win {
                let f: Vec<f64> = buf[..k.h.win].to_vec();
                k.frame(&f);
                got.push(k.last_raw() as f64);
                buf.drain(..k.h.hop);
            }
        }
        let el = t0.elapsed();
        assert_eq!(got.len(), tv.probs.len(), "кадров");
        let maxd = got.iter().zip(&tv.probs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        let peak = got.iter().cloned().fold(0.0, f64::max);
        let peak_py = tv.probs.iter().cloned().fold(0.0, f64::max);
        eprintln!("кадров {}, макс. расхождение {maxd:.5}, пик Rust {peak:.3} / Python {peak_py:.3}, {:?} на {:.1} с звука", got.len(), el, tv.pcm.len() as f64 / 16000.0);
        assert!(maxd < 0.01, "Rust и Python расходятся: {maxd}");
    }
}
