//! Слово «Джарвис» на признаках openWakeWord. Звук → мел-спектр (32 полосы, 10 мс) → сеть speech
//! embedding от Google (Apache 2.0, веса из openWakeWord) → «отпечаток» из 96 чисел каждые 80 мс → наша
//! маленькая сеть по 16 отпечаткам подряд → вероятность «Джарвис».
//!
//! Сеть отпечатков считается потоково: свёртки по времени без отступов, шаг по времени ровно 8 кадров,
//! поэтому каждые 80 мс досчитываются только новые строки каждого слоя.

use crate::{Bin, dot, gemv};

const STEP: usize = 1280; // 80 мс
const CTX: usize = 480;
const NFFT: usize = 512;
const HOP: usize = 160;
const MELS: usize = 32;
const BINS: usize = NFFT / 2 + 1;
const DROP: usize = 4; // окна сети начинаются с кадра 4 (как у openWakeWord при шаге 8)

/// После каких свёрток (1-based) пулинг (время×мелы).
fn pool_after(i: usize) -> Option<(usize, usize)> {
    match i {
        3 | 11 | 19 => Some((2, 2)),
        7 | 15 => Some((1, 2)),
        _ => None,
    }
}

struct Conv {
    co: usize,
    ci: usize,
    kh: usize,
    kw: usize,
    w: Vec<f32>, // [kh][kw][ci][co]
    b: Vec<f32>,
    win: usize,
    wout: usize,
    row_len: usize,
    rows: Vec<f32>, // последние kh строк входа подряд, кольцевой буфер без аллокаций
    filled: usize,
    head: usize,
    out: Vec<f32>,
    act: bool,
}

impl Conv {
    fn reset(&mut self) {
        self.filled = 0;
        self.head = 0;
    }

    fn push_row(&mut self, row: &[f32]) {
        debug_assert_eq!(row.len(), self.row_len);
        let slot = if self.filled < self.kh {
            let slot = self.filled;
            self.filled += 1;
            slot
        } else {
            let slot = self.head;
            self.head = (self.head + 1) % self.kh;
            slot
        };
        let dst = &mut self.rows[slot * self.row_len..(slot + 1) * self.row_len];
        dst.copy_from_slice(row);
    }

    fn forward(&mut self) {
        let (co, ci, kw) = (self.co, self.ci, self.kw);
        let pad = kw / 2;
        let span = kw * ci;
        for x in 0..self.wout {
            self.out[x * co..(x + 1) * co].copy_from_slice(&self.b);
        }
        for x in 0..self.wout {
            let lo = x as isize - pad as isize;
            let skip = if lo < 0 { (-lo) as usize } else { 0 };
            let hi = ((lo + kw as isize) as usize).min(self.win);
            let start = (lo + skip as isize) as usize;
            let n = (hi - start) * ci;
            let dst = &mut self.out[x * co..(x + 1) * co];
            for dh in 0..self.kh {
                let row_idx = (self.head + dh) % self.kh;
                let off = row_idx * self.row_len + start * ci;
                let src = &self.rows[off..off + n];
                let w = &self.w[(dh * span + skip * ci) * co..];
                gemv(dst, src, w, co);
            }
        }
        if self.act {
            for v in &mut self.out {
                if *v < 0.0 {
                    *v = (*v * 0.2).max(-0.4);
                }
            }
        }
    }
}

struct Pool {
    ph: usize,
    pw: usize,
    win: usize,
    c: usize,
    has_pending: bool,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl Pool {
    fn reset(&mut self) {
        self.has_pending = false;
    }

    fn forward(&mut self, row: &[f32]) -> bool {
        if self.ph == 2 && !self.has_pending {
            self.pending.copy_from_slice(row);
            self.has_pending = true;
            return false;
        }
        let wout = self.win / self.pw;
        let has_pending = self.has_pending;
        for x in 0..wout {
            for ch in 0..self.c {
                let mut m = f32::NEG_INFINITY;
                for dw in 0..self.pw {
                    let i = (x * self.pw + dw) * self.c + ch;
                    m = m.max(row[i]);
                    if has_pending {
                        m = m.max(self.pending[i]);
                    }
                }
                self.out[x * self.c + ch] = m;
            }
        }
        self.has_pending = false;
        true
    }
}

pub struct Oww {
    window: Vec<f32>,
    mel_sparse: Vec<(usize, Vec<f32>)>, // разрежённые полосы мел-фильтра [m] -> (start_bin, weights)
    floor: f64,
    convs: Vec<Conv>,
    pools: Vec<Option<Pool>>, // pools[i] — после свёртки i (0-based)
    seq: usize,
    emb: usize,
    h: usize,
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    w3: Vec<f32>,
    b3: f32,
    thresholds: [f32; 10],
    sensitivity: Box<dyn Fn() -> i64 + Send>,
    buf: Vec<f32>,
    dropped: usize,
    pub ring: Vec<Vec<f32>>,
    cooldown: u32,
    pub last_prob: f32,
    pending: u32,
    peak: f32,
    pub last_wake_prob: f32,
    fft: crate::Rfft,
    frame_buf: Vec<f32>,
    power: Vec<f32>,
    mel_buf: Vec<f32>,
    h1: Vec<f32>,
}

impl Oww {
    /// feat: oww.bin (мел и сеть отпечатков), head: wake_oww.bin (наша сеть «Джарвис»).
    pub fn load(feat: &[u8], head: &[u8], sensitivity: Box<dyn Fn() -> i64 + Send>) -> Result<Oww, String> {
        if feat.len() < 8 || &feat[..4] != b"OWW1" || head.len() < 8 || &head[..4] != b"OWWH" {
            return Err("не те файлы модели openWakeWord".into());
        }
        let mut r = Bin::new(&feat[4..]);
        let floor = r.f32s(1)[0] as f64;
        let window = r.f32s(NFFT);
        let melmat = r.f32s(BINS * MELS);
        let mel_sparse = (0..MELS)
            .map(|m| {
                let mut first = BINS;
                let mut last = 0;
                for b in 0..BINS {
                    if melmat[b * MELS + m] != 0.0 {
                        first = first.min(b);
                        last = b + 1;
                    }
                }
                if first >= last {
                    (0, Vec::new())
                } else {
                    let w: Vec<f32> = (first..last).map(|b| melmat[b * MELS + m]).collect();
                    (first, w)
                }
            })
            .collect();
        let n = r.u32();
        let mut width = MELS;
        let mut convs = vec![];
        let mut pools = vec![];
        for i in 1..=n {
            if r.err {
                break;
            }
            let (co, ci, kh, kw) = (r.u32(), r.u32(), r.u32(), r.u32());
            let raw = r.f32s(co * ci * kh * kw); // [co][ci][kh][kw]
            let b = r.f32s(co);
            let mut w = vec![0f32; raw.len()];
            for o in 0..co {
                for c in 0..ci {
                    for dh in 0..kh {
                        for dw in 0..kw {
                            w[((dh * kw + dw) * ci + c) * co + o] = raw[((o * ci + c) * kh + dh) * kw + dw];
                        }
                    }
                }
            }
            let row_len = width * ci;
            convs.push(Conv {
                co,
                ci,
                kh,
                kw,
                w,
                b,
                win: width,
                wout: width,
                row_len,
                rows: vec![0.0; kh * row_len],
                filled: 0,
                head: 0,
                out: vec![0.0; width * co],
                act: true,
            });
            match pool_after(i) {
                Some((ph, pw)) => {
                    pools.push(Some(Pool {
                        ph,
                        pw,
                        win: width,
                        c: co,
                        has_pending: false,
                        pending: if ph == 2 { vec![0.0; width * co] } else { Vec::new() },
                        out: vec![0.0; (width / pw) * co],
                    }));
                    width /= pw;
                }
                None => pools.push(None),
            }
        }
        if r.err || convs.len() != 20 || width != 1 {
            return Err("модель признаков повреждена".into());
        }
        convs.last_mut().unwrap().act = false;

        let mut hr = Bin::new(&head[4..]);
        let (seq, emb, h) = (hr.u32(), hr.u32(), hr.u32());
        if !(1..=40).contains(&seq) || emb != 96 || !(1..=256).contains(&h)
            || head.len() != 16 + 4 * (10 + h * seq * emb + h * h + 3 * h + 1)
        {
            return Err("Неверные размеры классификатора openWakeWord".into());
        }
        let th = hr.f32s(10);
        let w1 = hr.f32s(h * seq * emb);
        let b1 = hr.f32s(h);
        let w2 = hr.f32s(h * h);
        let b2 = hr.f32s(h);
        let w3 = hr.f32s(h);
        let b3 = hr.f32s(1)[0];
        if hr.err || !b3.is_finite() || th.iter().any(|p| !(0.0..=1.0).contains(p))
            || w1.iter().chain(&w2).chain(&w3).chain(&b1).chain(&b2).any(|v| !v.is_finite()) {

            return Err("модель «Джарвис» повреждена".into());
        }
        let mut thresholds = [0f32; 10];
        thresholds.copy_from_slice(&th);
        let frames_per_step = STEP / HOP;
        Ok(Oww {
            window,
            mel_sparse,
            floor,
            convs,
            pools,
            seq,
            emb,
            h,
            w1,
            b1,
            w2,
            b2,
            w3,
            b3,
            thresholds,
            sensitivity,
            buf: vec![0.0; CTX],
            dropped: 0,
            ring: Vec::with_capacity(seq),
            cooldown: 0,
            last_prob: 0.0,
            pending: 0,
            peak: 0.0,
            last_wake_prob: 0.0,
            fft: crate::Rfft::new(NFFT),
            frame_buf: vec![0.0; NFFT],
            power: vec![0.0; BINS],
            mel_buf: vec![0.0; frames_per_step * MELS],
            h1: vec![0.0; h],
        })
    }

    fn stream(&mut self, pcm: &[i16], mut emit: impl FnMut(&mut Self, usize)) {
        for (index, &sample) in pcm.iter().enumerate() {
            self.buf.push(sample as f32);
            if self.buf.len() != CTX + STEP { continue; }
            let n = self.mel();
            for t in 0..n {
                if self.dropped < DROP {
                    self.dropped += 1;
                } else if self.push_mel(t) {
                    emit(self, index + 1);
                }
            }
            self.buf.copy_within(STEP.., 0);
            self.buf.truncate(CTX);
        }
    }

    /// Feature extraction for enrollment. Each recording starts with independent DSP state.
    pub fn embeddings(&mut self, pcm: &[i16]) -> Vec<(usize, Vec<f32>)> {
        self.buf.resize(CTX, 0.0);
        self.buf.fill(0.0);
        self.dropped = 0;
        for convolution in &mut self.convs { convolution.reset(); }
        for pool in self.pools.iter_mut().flatten() { pool.reset(); }
        self.ring.clear();
        self.cooldown = 0;
        self.pending = 0;
        let mut features = Vec::with_capacity(pcm.len() / STEP);
        self.stream(pcm, |me, end| {
            features.push((end, me.convs.last().unwrap().out.clone()));
        });
        features
    }

    fn mel(&mut self) -> usize {
        let n = (self.buf.len() - NFFT) / HOP + 1;
        if self.mel_buf.len() < n * MELS {
            self.mel_buf.resize(n * MELS, 0.0);
        }
        let mut top = f64::NEG_INFINITY;
        for t in 0..n {
            let src = &self.buf[t * HOP..t * HOP + NFFT];
            for (dst, (&x, &w)) in self.frame_buf.iter_mut().zip(src.iter().zip(&self.window)) {
                *dst = x * w;
            }
            self.fft.power(&self.frame_buf, &mut self.power);
            let row = &mut self.mel_buf[t * MELS..(t + 1) * MELS];
            for (cell, (start, weights)) in row.iter_mut().zip(&self.mel_sparse) {
                let mut acc = 0f64;
                let p = &self.power[*start..*start + weights.len()];
                for (&pwr, &w) in p.iter().zip(weights) {
                    acc += pwr as f64 * w as f64;
                }
                let db = 10.0 * acc.max(self.floor).log10();
                *cell = db as f32;
                top = top.max(db);
            }
        }
        let norm_floor = top - 80.0;
        for v in &mut self.mel_buf[..n * MELS] {
            *v = ((*v as f64).max(norm_floor) / 10.0 + 2.0) as f32;
        }
        n
    }

    /// Новая строка на вход сети; при возврате true отпечаток (96 чисел) лежит в последнем слое `convs`.
    fn push_mel(&mut self, t: usize) -> bool {
        let row = &self.mel_buf[t * MELS..(t + 1) * MELS];
        self.convs[0].push_row(row);
        let len = self.convs.len();
        for i in 0..len {
            if i + 1 < len {
                let (left, right) = self.convs.split_at_mut(i + 1);
                let c = &mut left[i];
                if c.filled < c.kh {
                    return false;
                }
                c.forward();
                let next_row: &[f32] = if let Some(p) = self.pools[i].as_mut() {
                    if !p.forward(&c.out) {
                        return false;
                    }
                    &p.out
                } else {
                    &c.out
                };
                right[0].push_row(next_row);
            } else {
                let c = &mut self.convs[i];
                if c.filled < c.kh {
                    return false;
                }
                c.forward();
            }
        }
        true
    }

    /// Новый отпечаток в `convs.last().unwrap().out`: по последним seq считаем вероятность «Джарвис».
    fn head_current(&mut self) -> bool {
        let e = &self.convs.last().unwrap().out;
        if self.ring.len() < self.seq {
            self.ring.push(e.clone());
        } else {
            self.ring.rotate_left(1);
            self.ring.last_mut().unwrap().copy_from_slice(e);
        }
        if self.ring.len() < self.seq {
            return false;
        }
        let inp = self.emb * self.seq;
        for (j, out) in self.h1.iter_mut().enumerate() {
            let mut acc = self.b1[j];
            let wj = &self.w1[j * inp..(j + 1) * inp];
            for (t, v) in self.ring.iter().enumerate() {
                acc += dot(v, &wj[t * self.emb..(t + 1) * self.emb]);
            }
            *out = acc.max(0.0);
        }
        let mut acc = self.b3;
        for j in 0..self.h {
            let a = (self.b2[j] + dot(&self.h1, &self.w2[j * self.h..(j + 1) * self.h])).max(0.0);
            acc += a * self.w3[j];
        }
        let p = (1.0 / (1.0 + (-(acc as f64)).exp())) as f32;
        self.detect(p)
    }

    fn detect(&mut self, p: f32) -> bool {
        self.last_prob = p;
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        if self.pending > 0 {
            self.peak = self.peak.max(p);
            self.pending -= 1;
            if self.pending == 0 {
                self.last_wake_prob = self.peak;
                self.cooldown = 19;
                return true;
            }
            return false;
        }
        let s = (self.sensitivity)().clamp(1, 10) as usize;
        if p >= self.thresholds[s - 1] {
            // первое пересечение часто 0,4, а через 80–240 мс тот же оклик достигает 0,99: ждём пик
            self.pending = 3;
            self.peak = p;
        }
        false
    }
}

impl crate::Wake for Oww {
    fn feed(&mut self, pcm: &[i16]) -> bool {
        let mut hit = false;
        self.stream(pcm, |model, _| hit |= model.head_current());
        hit
    }

    fn reset(&mut self) {
        // сеть потоковая: сброс контекста давал ~2,5 с «глухоты»; подавляем лишь повторный оклик
        self.cooldown = 12;
        self.pending = 0;
    }

    fn last_prob(&self) -> f32 {
        self.last_prob
    }

    fn wake_prob(&self) -> f32 {
        self.last_wake_prob
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Wake;

    /// Те же входы и «золотые» отпечатки, что у Go (jarvisd/testdata/oww-stream.bin).
    #[test]
    fn stream_matches_go_golden() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (Ok(feat), Ok(head)) = (std::fs::read(root.join("models/oww.bin")), std::fs::read(root.join("models/wake_oww.bin"))) else {
            eprintln!("нет моделей — пропускаю");
            return;
        };
        let gold = std::env::var("JARVIS_OWW_GOLDEN").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("jarvisd/testdata/oww-stream.bin"));
        let Ok(gold) = std::fs::read(gold) else {
            eprintln!("нет oww-stream.bin — пропускаю");
            return;
        };
        let want: Vec<f32> = gold.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let mut o = Oww::load(&feat, &head, Box::new(|| 5)).unwrap();
        let mut values = vec![];
        let mut seed: u32 = 17;
        let mut pcm = vec![0i16; STEP];
        let t0 = std::time::Instant::now();
        for _ in 0..80 {
            for s in pcm.iter_mut() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *s = (seed >> 16) as u16 as i16;
            }
            o.feed(&pcm);
            if let Some(last) = o.ring.last() {
                values.extend_from_slice(last);
                values.push(o.last_prob);
            }
        }
        eprintln!("80 шагов (6,4 с звука) за {:?}", t0.elapsed());
        assert_eq!(values.len(), want.len());
        for (i, (a, b)) in values.iter().zip(&want).enumerate() {
            assert!(!a.is_nan() && (a - b).abs() <= 0.001, "отпечаток/вероятность {i}: {a} против {b}");
        }
    }
}
