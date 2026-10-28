//! Small, allocation-free DSP building blocks used on the audio threads.

use std::f32::consts::PI;

pub const SR: f32 = 48_000.0;

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

// ------------------------------------------------------------------ biquad (RBJ cookbook)

#[derive(Clone, Copy)]
pub struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    pub fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }

    fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    pub fn peaking(f0: f32, q: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / SR;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        Self::norm(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        )
    }

    pub fn low_shelf(f0: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / SR;
        let (s, c) = w0.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt(); // shelf slope S = 1
        let t = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) - (a - 1.0) * c + t),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - t),
            (a + 1.0) + (a - 1.0) * c + t,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - t,
        )
    }

    pub fn high_shelf(f0: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / SR;
        let (s, c) = w0.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let t = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * c + t),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - t),
            (a + 1.0) - (a - 1.0) * c + t,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - t,
        )
    }
}

/// 5-band EQ: low shelf, three peaking bands, high shelf. Handles up to two channels.
pub struct Eq5 {
    coeffs: [Coeffs; 5],
    z: [[[f32; 2]; 5]; 2],
    cached: [f32; 5],
    bypass: bool,
}

impl Eq5 {
    pub fn new() -> Self {
        Self {
            coeffs: [Coeffs::identity(); 5],
            z: [[[0.0; 2]; 5]; 2],
            cached: [0.0; 5],
            bypass: true,
        }
    }

    /// Recomputes coefficients only when the gains changed.
    pub fn update(&mut self, gains: [f32; 5]) {
        if gains == self.cached {
            return;
        }
        self.cached = gains;
        self.bypass = gains.iter().all(|g| g.abs() < 0.05);
        self.coeffs[0] = Coeffs::low_shelf(60.0, gains[0]);
        self.coeffs[1] = Coeffs::peaking(250.0, 0.9, gains[1]);
        self.coeffs[2] = Coeffs::peaking(1000.0, 0.9, gains[2]);
        self.coeffs[3] = Coeffs::peaking(4000.0, 0.9, gains[3]);
        self.coeffs[4] = Coeffs::high_shelf(12000.0, gains[4]);
    }

    #[inline]
    fn run(&mut self, ch: usize, mut x: f32) -> f32 {
        for b in 0..5 {
            let c = &self.coeffs[b];
            let z = &mut self.z[ch][b];
            let y = c.b0 * x + z[0];
            z[0] = c.b1 * x - c.a1 * y + z[1];
            z[1] = c.b2 * x - c.a2 * y;
            x = y;
        }
        x
    }

    #[inline]
    pub fn mono(&mut self, x: f32) -> f32 {
        if self.bypass {
            x
        } else {
            self.run(0, x)
        }
    }

    #[inline]
    pub fn stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.bypass {
            (l, r)
        } else {
            (self.run(0, l), self.run(1, r))
        }
    }
}

// ------------------------------------------------------------------ dynamics

/// Feed-forward compressor gain computer. Feed it a level, get a linear gain back.
pub struct Compressor {
    env: f32,
    atk: f32,
    rel: f32,
}

impl Compressor {
    pub fn new(attack_ms: f32, release_ms: f32) -> Self {
        Self {
            env: 0.0,
            atk: (-1.0 / (SR * attack_ms / 1000.0)).exp(),
            rel: (-1.0 / (SR * release_ms / 1000.0)).exp(),
        }
    }

    #[inline]
    pub fn gain(&mut self, level: f32, threshold_db: f32, ratio: f32) -> f32 {
        let coef = if level > self.env { self.atk } else { self.rel };
        self.env = coef * self.env + (1.0 - coef) * level;
        let db = lin_to_db(self.env);
        if db > threshold_db {
            db_to_lin((threshold_db - db) * (1.0 - 1.0 / ratio))
        } else {
            1.0
        }
    }
}

/// Simple stereo-linked peak limiter that protects the output from clipping.
pub struct Limiter {
    gain: f32,
    rel: f32,
}

const CEILING: f32 = 0.97;

impl Limiter {
    pub fn new() -> Self {
        Self { gain: 1.0, rel: 1.0 - (-1.0 / (SR * 0.15)).exp() }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let target = if peak > CEILING { CEILING / peak } else { 1.0 };
        if target < self.gain {
            self.gain = target;
        } else {
            self.gain += (target - self.gain) * self.rel;
        }
        ((l * self.gain).clamp(-1.0, 1.0), (r * self.gain).clamp(-1.0, 1.0))
    }
}
