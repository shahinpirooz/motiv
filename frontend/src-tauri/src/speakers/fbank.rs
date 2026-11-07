//! Kaldi-compatible log mel filterbank features (80 bins, 25 ms frames,
//! 10 ms shift, Hamming window, no dither), the input format of the WeSpeaker
//! speaker-embedding model. Matches `kaldi_native_fbank` / torchaudio's
//! `kaldi.fbank` with those options, followed by per-utterance mean
//! normalisation.

use realfft::RealFftPlanner;

pub const SAMPLE_RATE: usize = 16_000;
pub const NUM_BINS: usize = 80;
const FRAME_LEN: usize = 400; // 25 ms
const FRAME_SHIFT: usize = 160; // 10 ms
const FFT_LEN: usize = 512;
const PREEMPH: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;
const EPS: f32 = f32::EPSILON;

fn mel(f: f32) -> f32 {
    1127.0 * (1.0 + f / 700.0).ln()
}

/// Triangular mel filters over the FFT bins (Kaldi's MelBanks).
fn mel_banks() -> Vec<(usize, Vec<f32>)> {
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let fft_bin_width = SAMPLE_RATE as f32 / FFT_LEN as f32;
    let (lo, hi) = (mel(LOW_FREQ), mel(nyquist));
    let delta = (hi - lo) / (NUM_BINS + 1) as f32;
    (0..NUM_BINS)
        .map(|b| {
            let (left, center, right) = (lo + b as f32 * delta, lo + (b + 1) as f32 * delta, lo + (b + 2) as f32 * delta);
            let mut first = None;
            let mut weights = Vec::new();
            for i in 0..FFT_LEN / 2 {
                let m = mel(fft_bin_width * i as f32);
                if m > left && m < right {
                    let w = if m <= center { (m - left) / (center - left) } else { (right - m) / (right - center) };
                    first.get_or_insert(i);
                    weights.push(w);
                }
            }
            (first.unwrap_or(0), weights)
        })
        .collect()
}

/// Log mel filterbank, one row of `NUM_BINS` per frame, mean-normalised over
/// time. `samples` are 16 kHz mono in [-1, 1]; they are scaled to the int16
/// range as the model expects. Returns an empty Vec for audio under 25 ms.
pub fn fbank(samples: &[f32]) -> Vec<[f32; NUM_BINS]> {
    if samples.len() < FRAME_LEN {
        return Vec::new();
    }
    let n_frames = 1 + (samples.len() - FRAME_LEN) / FRAME_SHIFT;
    let window: Vec<f32> = (0..FRAME_LEN)
        .map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FRAME_LEN - 1) as f32).cos())
        .collect();
    let banks = mel_banks();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut power = vec![0f32; FFT_LEN / 2 + 1];
    let mut frame = vec![0f32; FRAME_LEN];

    let mut out = Vec::with_capacity(n_frames);
    for f in 0..n_frames {
        let start = f * FRAME_SHIFT;
        for (i, s) in samples[start..start + FRAME_LEN].iter().enumerate() {
            frame[i] = s * 32768.0;
        }
        let mean = frame.iter().sum::<f32>() / FRAME_LEN as f32;
        frame.iter_mut().for_each(|x| *x -= mean);
        for i in (1..FRAME_LEN).rev() {
            frame[i] -= PREEMPH * frame[i - 1];
        }
        frame[0] -= PREEMPH * frame[0];
        input.iter_mut().for_each(|x| *x = 0.0);
        for i in 0..FRAME_LEN {
            input[i] = frame[i] * window[i];
        }
        fft.process(&mut input, &mut spectrum).expect("fft sizes match");
        for (p, c) in power.iter_mut().zip(spectrum.iter()) {
            *p = c.re * c.re + c.im * c.im;
        }
        let mut row = [0f32; NUM_BINS];
        for (b, (first, weights)) in banks.iter().enumerate() {
            let e: f32 = weights.iter().enumerate().map(|(k, w)| w * power[first + k]).sum();
            row[b] = e.max(EPS).ln();
        }
        out.push(row);
    }
    // Cepstral mean normalisation.
    let mut mean = [0f32; NUM_BINS];
    for row in &out {
        for b in 0..NUM_BINS {
            mean[b] += row[b];
        }
    }
    for m in mean.iter_mut() {
        *m /= out.len() as f32;
    }
    for row in out.iter_mut() {
        for b in 0..NUM_BINS {
            row[b] -= mean[b];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_kaldi_native_fbank() {
        let v: serde_json::Value = serde_json::from_str(include_str!("testdata/fbank_ref.json")).unwrap();
        let samples: Vec<f32> = v["samples"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let expected: Vec<Vec<f32>> = v["fbank_cmn"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect())
            .collect();
        let got = fbank(&samples);
        assert_eq!(got.len(), expected.len());
        let mut max_err = 0f32;
        for (g, e) in got.iter().zip(&expected) {
            for b in 0..NUM_BINS {
                max_err = max_err.max((g[b] - e[b]).abs());
            }
        }
        assert!(max_err < 0.02, "max abs error {}", max_err);
    }

    #[test]
    fn short_audio_has_no_frames() {
        assert!(fbank(&[0.0; 399]).is_empty());
        assert_eq!(fbank(&[0.0; 400 + 160 * 3]).len(), 4);
    }
}
