//! Звук Джарвиса: спектр и детекторы слова «Джарвис». Порт Go-версии один в один —
//! совпадение проверяется тестами на тех же моделях и векторах (Python, «золотые» данные Go).

pub mod audio;
pub mod kws;
pub mod oww;

/// Детектор слова активации: звук 16 кГц моно кусками любой длины.
pub trait Wake: Send {
    /// true — только что прозвучало «Джарвис».
    fn feed(&mut self, pcm: &[i16]) -> bool;
    /// После разговора: не слушать хвост своей же речи.
    fn reset(&mut self);
    /// Последняя вероятность (для панели и журнала).
    fn last_prob(&self) -> f32;
    /// Вероятность в момент срабатывания.
    fn wake_prob(&self) -> f32 {
        self.last_prob()
    }
}

/// Классический радикс-2 FFT на месте (как в Go: те же операции в том же порядке).
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut size = 2;
    while size <= n {
        let ang = -2.0 * std::f64::consts::PI / size as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut start = 0;
        while start < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..size / 2 {
                let (a, b) = (start + k, start + k + size / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            start += size;
        }
        size <<= 1;
    }
}

/// dst[c] += Σ src[i] · w[i·stride + c]  (свёртки: выходные каналы подряд — векторизуется).
#[inline]
pub fn gemv(dst: &mut [f32], src: &[f32], w: &[f32], stride: usize) {
    let n = dst.len();
    for (i, &v) in src.iter().enumerate() {
        let row = &w[i * stride..i * stride + n];
        for (d, &r) in dst.iter_mut().zip(row) {
            *d += v * r;
        }
    }
}

/// Скалярное произведение: четыре независимые суммы (как dotGo).
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let b = &b[..a.len()];
    let (mut s0, mut s1, mut s2, mut s3) = (0f32, 0f32, 0f32, 0f32);
    let mut ca = a.chunks_exact(4);
    let mut cb = b.chunks_exact(4);
    for (x, y) in (&mut ca).zip(&mut cb) {
        s0 += x[0] * y[0];
        s1 += x[1] * y[1];
        s2 += x[2] * y[2];
        s3 += x[3] * y[3];
    }
    for (x, y) in ca.remainder().iter().zip(cb.remainder()) {
        s0 += x * y;
    }
    (s0 + s1) + (s2 + s3)
}

/// Предвычисленный БПФ вещественного сигнала в `f32` без тригонометрии на кадрах (NEON-дружественный).
pub(crate) struct Rfft {
    m: usize,
    bitrev: Vec<usize>,
    stage_re: Vec<Vec<f32>>,
    stage_im: Vec<Vec<f32>>,
    unp_re: Vec<f32>,
    unp_im: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Rfft {
    pub(crate) fn new(n: usize) -> Self {
        assert!(n >= 4 && n.is_power_of_two());
        let m = n / 2;
        let bits = m.trailing_zeros() as usize;
        let bitrev = (0..m).map(|i| i.reverse_bits() >> (usize::BITS as usize - bits)).collect();
        let mut stage_re = Vec::with_capacity(bits);
        let mut stage_im = Vec::with_capacity(bits);
        let mut size = 2;
        while size <= m {
            let half = size / 2;
            let mut sre = Vec::with_capacity(half);
            let mut sim = Vec::with_capacity(half);
            for k in 0..half {
                let ang = -2.0 * std::f64::consts::PI * k as f64 / size as f64;
                sre.push(ang.cos() as f32);
                sim.push(ang.sin() as f32);
            }
            stage_re.push(sre);
            stage_im.push(sim);
            size <<= 1;
        }
        let mut unp_re = Vec::with_capacity(m / 2);
        let mut unp_im = Vec::with_capacity(m / 2);
        for k in 0..m / 2 {
            let ang = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
            unp_re.push(ang.cos() as f32);
            unp_im.push(ang.sin() as f32);
        }
        Rfft {
            m,
            bitrev,
            stage_re,
            stage_im,
            unp_re,
            unp_im,
            re: vec![0.0; m],
            im: vec![0.0; m],
        }
    }

    /// Квадрат модуля спектра `input[0..n]` в `out[0..=n/2]`.
    pub(crate) fn power(&mut self, input: &[f32], out: &mut [f32]) {
        let m = self.m;
        debug_assert_eq!(input.len(), m * 2);
        debug_assert_eq!(out.len(), m + 1);
        for (k, &rev) in self.bitrev.iter().enumerate() {
            self.re[rev] = input[k * 2];
            self.im[rev] = input[k * 2 + 1];
        }
        let mut half = 1;
        for (sre, sim) in self.stage_re.iter().zip(&self.stage_im) {
            let size = half * 2;
            for start in (0..m).step_by(size) {
                let (re_lo, re_hi) = self.re[start..start + size].split_at_mut(half);
                let (im_lo, im_hi) = self.im[start..start + size].split_at_mut(half);
                for k in 0..half {
                    let wr = sre[k];
                    let wi = sim[k];
                    let br = re_hi[k];
                    let bi = im_hi[k];
                    let tr = br * wr - bi * wi;
                    let ti = br * wi + bi * wr;
                    re_hi[k] = re_lo[k] - tr;
                    im_hi[k] = im_lo[k] - ti;
                    re_lo[k] += tr;
                    im_lo[k] += ti;
                }
            }
            half = size;
        }
        let r0 = self.re[0];
        let i0 = self.im[0];
        out[0] = (r0 + i0) * (r0 + i0);
        out[m] = (r0 - i0) * (r0 - i0);
        let q = m / 2;
        if m >= 2 {
            out[q] = self.re[q] * self.re[q] + self.im[q] * self.im[q];
        }
        for k in 1..q {
            let mk = m - k;
            let er = 0.5 * (self.re[k] + self.re[mk]);
            let ei = 0.5 * (self.im[k] - self.im[mk]);
            let or = 0.5 * (self.im[k] + self.im[mk]);
            let oi = -0.5 * (self.re[k] - self.re[mk]);
            let wr = self.unp_re[k];
            let wi = self.unp_im[k];
            let tr = or * wr - oi * wi;
            let ti = or * wi + oi * wr;
            let pr = er + tr;
            let pi = ei + ti;
            let nr = er - tr;
            let ni = ei - ti;
            out[k] = pr * pr + pi * pi;
            out[mk] = nr * nr + ni * ni;
        }
    }
}

/// Чтение моделей: little-endian u32 и f32 подряд.
pub(crate) struct Bin<'a> {
    b: &'a [u8],
    pub err: bool,
}

impl<'a> Bin<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Bin { b, err: false }
    }
    pub fn u32(&mut self) -> usize {
        if self.b.len() < 4 {
            self.err = true;
            return 0;
        }
        let v = u32::from_le_bytes(self.b[..4].try_into().unwrap());
        self.b = &self.b[4..];
        v as usize
    }
    pub fn f32s(&mut self, n: usize) -> Vec<f32> {
        if self.err || self.b.len() < n * 4 {
            self.err = true;
            return vec![0.0; n];
        }
        let v = self.b[..n * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        self.b = &self.b[n * 4..];
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_of_impulse_is_flat() {
        let mut re = vec![0.0; 8];
        let mut im = vec![0.0; 8];
        re[0] = 1.0;
        fft(&mut re, &mut im);
        assert!(re.iter().all(|v| (v - 1.0).abs() < 1e-12) && im.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn dot_and_gemv() {
        let a: Vec<f32> = (0..10).map(|i| i as f32).collect();
        assert_eq!(dot(&a, &a), 285.0);
        let mut d = vec![1.0, 1.0];
        gemv(&mut d, &[1.0, 2.0], &[1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(d, vec![8.0, 11.0]);
    }

    #[test]
    fn rfft_matches_reference_fft() {
        let mut x = [0f32; 512];
        let mut re = [0f64; 512];
        let mut im = [0f64; 512];
        let mut seed = 42u32;
        for i in 0..512 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            x[i] = ((seed >> 16) as f32 / 32768.0) - 1.0;
            re[i] = x[i] as f64;
        }
        fft(&mut re, &mut im);
        let mut rfft = Rfft::new(512);
        let mut got = [0f32; 257];
        rfft.power(&x, &mut got);
        for b in 0..257 {
            let want = (re[b] * re[b] + im[b] * im[b]) as f32;
            let err = (got[b] - want).abs();
            assert!(err <= want.abs() * 1e-4 + 1e-4, "bin {b}: got {got_b} want {want}", got_b = got[b]);
        }
    }
}
