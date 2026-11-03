//! Analyse acoustique (spec §2.5.3, FR-AU-02/03) : trames de 4096
//! echantillons a 16 kHz (256 ms), fenetre de Hann, FFT reelle.
//! Les echantillons arrivent deja ramenes a 24 bits signes (INMP441,
//! mot I2S de 32 bits decale de 8).
use crate::config::Settings;
use serde::Serialize;

pub const SAMPLE_RATE: f32 = 16_000.0;
pub const FRAME: usize = 4096;
/// Pleine echelle d'un echantillon 24 bits.
const FULL_SCALE: f32 = 8_388_608.0;
const BIN_HZ: f32 = SAMPLE_RATE / FRAME as f32; // 3,906 Hz
/// -110 dBFS en amplitude relative.
const SILENCE_FLOOR: f32 = 3.162e-6;
pub const BANDS_HZ: [(f32, f32); 4] = [
    (100.0, 200.0),
    (200.0, 300.0),
    (300.0, 500.0),
    (500.0, 1000.0),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMetrics {
    pub db_spl: f32,
    pub f_dom_hz: f32,
    /// Part de l'energie 100-1000 Hz dans chaque bande (somme = 1)
    pub bands: [f32; 4],
    pub silent: bool,
}

impl FrameMetrics {
    pub fn ratio_b3_b2(&self) -> f32 {
        if self.bands[1] > 0.0 {
            self.bands[2] / self.bands[1]
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    LvlMax,
    LvlMin,
    FreqMax,
    FreqMin,
    Ratio,
}

pub fn analyze_frame(samples: &[i32; FRAME], spl_offset_db: f32) -> FrameMetrics {
    // RMS de la seule composante alternative : un micro debranche ou une
    // ligne DATA figee (0, -1, ou n'importe quelle constante) n'a aucune
    // energie AC. Le plancher de bruit de l'INMP441 est vers -87 dBFS ;
    // sous -110 dBFS, ce n'est pas un son, c'est un micro en panne.
    let mean = samples.iter().map(|&s| s as f64).sum::<f64>() / FRAME as f64;
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64 - mean).powi(2)).sum();
    let ac_rms = (sum_sq / FRAME as f64).sqrt() as f32;
    if ac_rms < FULL_SCALE * SILENCE_FLOOR {
        return FrameMetrics {
            db_spl: 0.0,
            f_dom_hz: 0.0,
            bands: [0.0; 4],
            silent: true,
        };
    }
    let db_spl = 20.0 * (ac_rms / FULL_SCALE).log10() + spl_offset_db;

    let mut buf = [0f32; FRAME];
    for (i, (b, &s)) in buf.iter_mut().zip(samples.iter()).enumerate() {
        let hann = 0.5 - 0.5 * (2.0 * core::f32::consts::PI * i as f32 / FRAME as f32).cos();
        *b = s as f32 / FULL_SCALE * hann;
    }
    let spec = microfft::real::rfft_4096(&mut buf);
    // bin 0 : DC (re) et Nyquist (im) empaquetes -> ignore
    let power = |k: usize| spec[k].norm_sqr();

    let bin = |hz: f32| (hz / BIN_HZ).round() as usize;
    let (mut peak_k, mut peak_p) = (0, 0f32);
    for k in bin(50.0)..=bin(2000.0) {
        if power(k) > peak_p {
            peak_k = k;
            peak_p = power(k);
        }
    }

    let mut bands = [0f32; 4];
    for (e, (lo, hi)) in bands.iter_mut().zip(BANDS_HZ) {
        *e = (bin(lo)..bin(hi)).map(power).sum();
    }
    let total: f32 = bands.iter().sum();
    if total > 0.0 {
        bands.iter_mut().for_each(|e| *e /= total);
    }

    FrameMetrics {
        db_spl,
        f_dom_hz: peak_k as f32 * BIN_HZ,
        bands,
        silent: false,
    }
}

/// Premiere regle de la consigne enfreinte par la trame (FR-AU-03).
pub fn frame_breach(m: &FrameMetrics, s: &Settings) -> Option<Rule> {
    if m.silent {
        return None;
    }
    if m.db_spl > s.lvl_max_db {
        Some(Rule::LvlMax)
    } else if m.db_spl < s.lvl_min_db {
        Some(Rule::LvlMin)
    } else if m.f_dom_hz > s.f_max_hz {
        Some(Rule::FreqMax)
    } else if m.f_dom_hz < s.f_min_hz {
        Some(Rule::FreqMin)
    } else if m.ratio_b3_b2() > s.ratio_b3_b2_max {
        Some(Rule::Ratio)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowSummary {
    pub frames: u16,
    pub db: f32,
    pub f_dom: f32,
    pub bands: [f32; 4],
    pub ratio_b3_b2: f32,
    /// Regle la plus souvent enfreinte, si la fenetre est hors consigne
    pub breach: Option<Rule>,
    pub mic_fault: bool,
}

/// Cumule les trames d'une fenetre d'ecoute.
#[derive(Default)]
pub struct WindowAcc {
    db: Vec<f32>,
    f_dom: Vec<f32>,
    bands: [f32; 4],
    rules: Vec<Rule>,
    frames: u16,
}

impl WindowAcc {
    pub fn push(&mut self, m: &FrameMetrics, s: &Settings) {
        self.frames += 1;
        if m.silent {
            return;
        }
        self.db.push(m.db_spl);
        self.f_dom.push(m.f_dom_hz);
        for (a, b) in self.bands.iter_mut().zip(m.bands) {
            *a += b;
        }
        if let Some(r) = frame_breach(m, s) {
            self.rules.push(r);
        }
    }

    pub fn finish(mut self, s: &Settings) -> WindowSummary {
        let n = self.db.len();
        if n == 0 {
            // micro debranche : pas d'information, donc pas d'alarme
            return WindowSummary {
                frames: self.frames,
                db: 0.0,
                f_dom: 0.0,
                bands: [0.0; 4],
                ratio_b3_b2: 0.0,
                breach: None,
                mic_fault: true,
            };
        }
        let db = self.db.iter().sum::<f32>() / n as f32;
        self.f_dom.sort_by(f32::total_cmp);
        let f_dom = self.f_dom[n / 2]; // mediane : robuste a un bruit isole
        let bands = self.bands.map(|b| b / n as f32);
        let ratio_b3_b2 = if bands[1] > 0.0 {
            bands[2] / bands[1]
        } else {
            0.0
        };
        let breach = if self.rules.len() as f32 >= s.breach_ratio * self.frames as f32 {
            [
                Rule::LvlMax,
                Rule::LvlMin,
                Rule::FreqMax,
                Rule::FreqMin,
                Rule::Ratio,
            ]
            .into_iter()
            .max_by_key(|r| self.rules.iter().filter(|x| *x == r).count())
        } else {
            None
        };
        WindowSummary {
            frames: self.frames,
            db,
            f_dom,
            bands,
            ratio_b3_b2,
            breach,
            mic_fault: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sinus d'amplitude `amp` (fraction de la pleine echelle).
    fn sine(hz: f32, amp: f32) -> [i32; FRAME] {
        let mut out = [0i32; FRAME];
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE;
            *o = (amp * FULL_SCALE * (2.0 * core::f32::consts::PI * hz * t).sin()) as i32;
        }
        out
    }

    #[test]
    fn dominant_frequency_within_8hz() {
        for hz in [150.0, 250.0, 450.0, 777.0] {
            let m = analyze_frame(&sine(hz, 0.1), 120.0);
            assert!((m.f_dom_hz - hz).abs() <= 8.0, "{hz} -> {}", m.f_dom_hz);
        }
    }

    #[test]
    fn level_in_db() {
        // sinus de RMS 0,5 pleine echelle = -6,02 dBFS
        let m = analyze_frame(&sine(250.0, 0.5 * core::f32::consts::SQRT_2), 120.0);
        assert!((m.db_spl - (120.0 - 6.02)).abs() < 0.1, "{}", m.db_spl);
    }

    #[test]
    fn bands_and_ratio() {
        let normal = analyze_frame(&sine(250.0, 0.1), 120.0);
        assert!(normal.bands[1] > 0.9, "{:?}", normal.bands);
        let piping = analyze_frame(&sine(450.0, 0.1), 120.0);
        assert!(piping.ratio_b3_b2() > 1.5);
        assert!((piping.bands.iter().sum::<f32>() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn silence_is_mic_fault() {
        let m = analyze_frame(&[0; FRAME], 120.0);
        assert!(m.silent);
        let s = Settings::default();
        let mut w = WindowAcc::default();
        for _ in 0..10 {
            w.push(&m, &s);
        }
        let sum = w.finish(&s);
        assert!(sum.mic_fault);
        assert_eq!(sum.breach, None);
    }

    #[test]
    fn stuck_mic_is_mic_fault() {
        // ligne DATA flottante a 1 : 0xFFFFFFFF >> 8 = -1 ; ou decalage DC fige
        for v in [-1, 1000, -8_388_608] {
            let m = analyze_frame(&[v; FRAME], 120.0);
            assert!(m.silent, "constante {v} -> {m:?}");
        }
        // un micro qui fonctionne reste au-dessus du plancher, meme tres bas
        assert!(!analyze_frame(&sine(250.0, 1e-4), 120.0).silent);
    }

    #[test]
    fn rules_in_order() {
        let s = Settings::default();
        let base = FrameMetrics {
            db_spl: 60.0,
            f_dom_hz: 250.0,
            bands: [0.1, 0.6, 0.2, 0.1],
            silent: false,
        };
        assert_eq!(frame_breach(&base, &s), None);
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    db_spl: 90.0,
                    f_dom_hz: 450.0,
                    ..base
                },
                &s
            ),
            Some(Rule::LvlMax)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    db_spl: 30.0,
                    ..base
                },
                &s
            ),
            Some(Rule::LvlMin)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    f_dom_hz: 450.0,
                    ..base
                },
                &s
            ),
            Some(Rule::FreqMax)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    f_dom_hz: 100.0,
                    ..base
                },
                &s
            ),
            Some(Rule::FreqMin)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    bands: [0.1, 0.3, 0.5, 0.1],
                    ..base
                },
                &s
            ),
            Some(Rule::Ratio)
        );
    }

    #[test]
    fn window_breach_ratio() {
        let s = Settings::default(); // breach_ratio 0,6
        let ok = FrameMetrics {
            db_spl: 60.0,
            f_dom_hz: 250.0,
            bands: [0.1, 0.6, 0.2, 0.1],
            silent: false,
        };
        let bad = FrameMetrics {
            f_dom_hz: 450.0,
            ..ok
        };
        let run = |n_bad: usize| {
            let mut w = WindowAcc::default();
            for i in 0..10 {
                w.push(if i < n_bad { &bad } else { &ok }, &s);
            }
            w.finish(&s)
        };
        assert_eq!(run(5).breach, None);
        assert_eq!(run(6).breach, Some(Rule::FreqMax));
        assert_eq!(run(6).f_dom, 450.0);
        assert_eq!(run(0).frames, 10);
    }
}
