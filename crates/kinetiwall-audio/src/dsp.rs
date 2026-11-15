//! Real-time Digital Signal Processing (DSP) pipeline for audio feature extraction.
//!
//! Provides:
//! - Multi-format audio sample conversion (PCM 16/24/32-bit & IEEE Float) to mono float buffers.
//! - Root Mean Square (RMS) energy calculation with ambient noise-floor gating.
//! - Peak amplitude detection with exponential transient decay.
//! - In-place Radix-2 Cooley-Tukey FFT with Hann windowing for 8 acoustic frequency bands.
//! - Temporal attack/decay smoothing and strict numerical sanitization.

use std::f32::consts::PI;

use crate::contract::{VisualizerFrame, NUM_FREQUENCY_BANDS};

/// Minimum energy threshold below which input is treated as silence (-60 dBFS approx).
pub const SILENCE_THRESHOLD: f32 = 0.001;

/// Standard window size for Radix-2 FFT spectral analysis (must be a power of 2).
pub const FFT_WINDOW_SIZE: usize = 256;

/// Band boundaries in Hertz: [min, max] for each of the 8 acoustic frequency bands.
pub const BAND_RANGES_HZ: [[f32; 2]; NUM_FREQUENCY_BANDS] = [
    [20.0, 60.0],       // Sub-bass
    [60.0, 250.0],      // Bass
    [250.0, 500.0],     // Low-mid
    [500.0, 1500.0],    // Mid
    [1500.0, 3000.0],   // Upper-mid
    [3000.0, 6000.0],   // Presence
    [6000.0, 12000.0],  // Brilliance
    [12000.0, 20000.0], // Highs
];

/// Converts raw IEEE 32-bit float interleaved bytes into a normalized mono f32 vector.
pub fn convert_f32_le_to_mono(raw_bytes: &[u8], channels: usize) -> Vec<f32> {
    if channels == 0 || raw_bytes.len() < 4 * channels {
        return Vec::new();
    }

    let frame_bytes = 4 * channels;
    let num_frames = raw_bytes.len() / frame_bytes;
    let mut mono = Vec::with_capacity(num_frames);

    for frame_idx in 0..num_frames {
        let frame_start = frame_idx * frame_bytes;
        let mut sum = 0.0f32;

        for ch in 0..channels {
            let offset = frame_start + ch * 4;
            let sample_bytes = [
                raw_bytes[offset],
                raw_bytes[offset + 1],
                raw_bytes[offset + 2],
                raw_bytes[offset + 3],
            ];
            let sample = f32::from_le_bytes(sample_bytes);
            if sample.is_finite() {
                sum += sample;
            }
        }
        mono.push(sum / channels as f32);
    }

    mono
}

/// Converts raw 16-bit signed PCM interleaved bytes into a normalized mono f32 vector [-1.0, 1.0].
pub fn convert_i16_le_to_mono(raw_bytes: &[u8], channels: usize) -> Vec<f32> {
    if channels == 0 || raw_bytes.len() < 2 * channels {
        return Vec::new();
    }

    let frame_bytes = 2 * channels;
    let num_frames = raw_bytes.len() / frame_bytes;
    let mut mono = Vec::with_capacity(num_frames);

    for frame_idx in 0..num_frames {
        let frame_start = frame_idx * frame_bytes;
        let mut sum = 0.0f32;

        for ch in 0..channels {
            let offset = frame_start + ch * 2;
            let sample_bytes = [raw_bytes[offset], raw_bytes[offset + 1]];
            let sample = i16::from_le_bytes(sample_bytes);
            sum += sample as f32 / 32768.0;
        }
        mono.push(sum / channels as f32);
    }

    mono
}

/// Converts raw 24-bit signed PCM interleaved bytes into a normalized mono f32 vector [-1.0, 1.0].
pub fn convert_i24_le_to_mono(raw_bytes: &[u8], channels: usize) -> Vec<f32> {
    if channels == 0 || raw_bytes.len() < 3 * channels {
        return Vec::new();
    }

    let frame_bytes = 3 * channels;
    let num_frames = raw_bytes.len() / frame_bytes;
    let mut mono = Vec::with_capacity(num_frames);

    for frame_idx in 0..num_frames {
        let frame_start = frame_idx * frame_bytes;
        let mut sum = 0.0f32;

        for ch in 0..channels {
            let offset = frame_start + ch * 3;
            // Sign-extend 24-bit to 32-bit signed integer
            let b0 = raw_bytes[offset] as i32;
            let b1 = raw_bytes[offset + 1] as i32;
            let b2 = raw_bytes[offset + 2] as i8 as i32; // sign extended
            let sample = (b2 << 16) | (b1 << 8) | b0;
            sum += sample as f32 / 8388608.0;
        }
        mono.push(sum / channels as f32);
    }

    mono
}

/// Converts raw 32-bit signed PCM interleaved bytes into a normalized mono f32 vector [-1.0, 1.0].
pub fn convert_i32_le_to_mono(raw_bytes: &[u8], channels: usize) -> Vec<f32> {
    if channels == 0 || raw_bytes.len() < 4 * channels {
        return Vec::new();
    }

    let frame_bytes = 4 * channels;
    let num_frames = raw_bytes.len() / frame_bytes;
    let mut mono = Vec::with_capacity(num_frames);

    for frame_idx in 0..num_frames {
        let frame_start = frame_idx * frame_bytes;
        let mut sum = 0.0f32;

        for ch in 0..channels {
            let offset = frame_start + ch * 4;
            let sample_bytes = [
                raw_bytes[offset],
                raw_bytes[offset + 1],
                raw_bytes[offset + 2],
                raw_bytes[offset + 3],
            ];
            let sample = i32::from_le_bytes(sample_bytes);
            sum += sample as f32 / 2147483648.0;
        }
        mono.push(sum / channels as f32);
    }

    mono
}

/// In-place Radix-2 Cooley-Tukey Fast Fourier Transform (DIT).
/// `n` must be a power of 2.
pub fn radix2_fft(real: &mut [f32], imag: &mut [f32]) {
    let n = real.len();
    debug_assert!(n.is_power_of_two());
    debug_assert_eq!(n, imag.len());

    // 1. Bit-reversal permutation
    let mut j = 0;
    for i in 0..(n - 1) {
        if i < j {
            real.swap(i, j);
            imag.swap(i, j);
        }
        let mut k = n >> 1;
        while k <= j {
            j -= k;
            k >>= 1;
        }
        j += k;
    }

    // 2. Butterfly computation
    let mut len = 2;
    while len <= n {
        let half = len >> 1;
        let angle = -2.0 * PI / (len as f32);
        let w_step_re = angle.cos();
        let w_step_im = angle.sin();

        let mut i = 0;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;

            for k in 0..half {
                let u_re = real[i + k];
                let u_im = imag[i + k];

                let v_re = real[i + k + half] * w_re - imag[i + k + half] * w_im;
                let v_im = real[i + k + half] * w_im + imag[i + k + half] * w_re;

                real[i + k] = u_re + v_re;
                imag[i + k] = u_im + v_im;
                real[i + k + half] = u_re - v_re;
                imag[i + k + half] = u_im - v_im;

                let next_w_re = w_re * w_step_re - w_im * w_step_im;
                let next_w_im = w_re * w_step_im + w_im * w_step_re;
                w_re = next_w_re;
                w_im = next_w_im;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Computes acoustic band energies using a 256-point Hann-windowed Radix-2 FFT.
pub fn compute_frequency_bands(samples: &[f32], sample_rate: u32) -> [f32; NUM_FREQUENCY_BANDS] {
    let mut bands = [0.0f32; NUM_FREQUENCY_BANDS];
    if samples.is_empty() || sample_rate == 0 {
        return bands;
    }

    let n = FFT_WINDOW_SIZE;
    let mut real = vec![0.0f32; n];
    let mut imag = vec![0.0f32; n];

    // Take the most recent n samples or pad with zeros
    let start_idx = if samples.len() > n {
        samples.len() - n
    } else {
        0
    };
    let slice = &samples[start_idx..];

    // Apply Hann window
    for (i, &s) in slice.iter().enumerate().take(n) {
        if s.is_finite() {
            let hann = 0.5 * (1.0 - (2.0 * PI * i as f32 / (n as f32 - 1.0)).cos());
            real[i] = s * hann;
        }
    }

    radix2_fft(&mut real, &mut imag);

    let bin_resolution = sample_rate as f32 / n as f32;
    let half_n = n / 2;

    for (band_idx, range) in BAND_RANGES_HZ.iter().enumerate() {
        let min_hz = range[0];
        let max_hz = range[1];

        let min_bin = ((min_hz / bin_resolution).floor() as usize).clamp(1, half_n - 1);
        let max_bin = ((max_hz / bin_resolution).ceil() as usize).clamp(min_bin, half_n - 1);

        let mut sum_mag = 0.0f32;
        let mut count = 0;

        for k in min_bin..=max_bin {
            let mag = (real[k] * real[k] + imag[k] * imag[k]).sqrt() / (half_n as f32);
            if mag.is_finite() {
                sum_mag += mag;
                count += 1;
            }
        }

        if count > 0 {
            // Apply scale multiplier for visual prominence
            bands[band_idx] = (sum_mag / count as f32 * 4.0).clamp(0.0, 1.0);
        }
    }

    bands
}

/// Stateful real-time audio feature extractor with smoothing and transient tracking.
pub struct AudioFeatureExtractor {
    pub sample_rate: u32,
    pub sensitivity: f32,
    pub smoothing: f32,
    prev_peak: f32,
    prev_bands: [f32; NUM_FREQUENCY_BANDS],
    sequence: u64,
}

impl AudioFeatureExtractor {
    pub fn new(sample_rate: u32, sensitivity: f32, smoothing: f32) -> Self {
        Self {
            sample_rate: if sample_rate > 0 { sample_rate } else { 48000 },
            sensitivity: if sensitivity.is_finite() {
                sensitivity.clamp(0.1, 5.0)
            } else {
                1.0
            },
            smoothing: if smoothing.is_finite() {
                smoothing.clamp(0.0, 0.95)
            } else {
                0.5
            },
            prev_peak: 0.0,
            prev_bands: [0.0; NUM_FREQUENCY_BANDS],
            sequence: 0,
        }
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        if sensitivity.is_finite() {
            self.sensitivity = sensitivity.clamp(0.1, 5.0);
        }
    }

    pub fn set_smoothing(&mut self, smoothing: f32) {
        if smoothing.is_finite() {
            self.smoothing = smoothing.clamp(0.0, 0.95);
        }
    }

    /// Processes a block of mono float samples and extracts a sanitized VisualizerFrame.
    pub fn process_mono_samples(&mut self, samples: &[f32], timestamp_ms: u64) -> VisualizerFrame {
        self.sequence = self.sequence.wrapping_add(1);

        if samples.is_empty() {
            self.prev_peak *= 0.85;
            for b in &mut self.prev_bands {
                *b *= 0.85;
            }
            return VisualizerFrame::silence(self.sequence, timestamp_ms);
        }

        let mut sum_sq = 0.0f32;
        let mut raw_peak = 0.0f32;
        let mut valid_samples = 0;

        for &s in samples {
            if s.is_finite() {
                let abs_s = s.abs();
                sum_sq += s * s;
                if abs_s > raw_peak {
                    raw_peak = abs_s;
                }
                valid_samples += 1;
            }
        }

        if valid_samples == 0 {
            return VisualizerFrame::silence(self.sequence, timestamp_ms);
        }

        let rms = (sum_sq / valid_samples as f32).sqrt();

        // Noise gate: below noise floor returns silence
        if rms < SILENCE_THRESHOLD && raw_peak < SILENCE_THRESHOLD {
            self.prev_peak *= 0.85;
            for b in &mut self.prev_bands {
                *b *= 0.85;
            }
            let mut frame = VisualizerFrame::silence(self.sequence, timestamp_ms);
            frame.peak = self.prev_peak;
            frame.frequency_bands = self.prev_bands;
            return frame;
        }

        // Apply user sensitivity scaling
        let scaled_rms = (rms * self.sensitivity * 2.0).clamp(0.0, 1.0);
        let scaled_peak = (raw_peak * self.sensitivity).clamp(0.0, 1.0);

        // Peak transient tracking with exponential decay
        let peak = if scaled_peak > self.prev_peak {
            scaled_peak
        } else {
            (self.prev_peak * 0.92).max(scaled_peak)
        };
        self.prev_peak = peak;

        // Compute FFT frequency bands
        let raw_bands = compute_frequency_bands(samples, self.sample_rate);

        // Apply temporal smoothing: alpha = 1.0 - smoothing
        let alpha = (1.0 - self.smoothing).clamp(0.05, 1.0);
        let mut smoothed_bands = [0.0f32; NUM_FREQUENCY_BANDS];

        for i in 0..NUM_FREQUENCY_BANDS {
            let target = (raw_bands[i] * self.sensitivity).clamp(0.0, 1.0);
            smoothed_bands[i] =
                (alpha * target + (1.0 - alpha) * self.prev_bands[i]).clamp(0.0, 1.0);
            self.prev_bands[i] = smoothed_bands[i];
        }

        let mut frame = VisualizerFrame {
            sequence: self.sequence,
            timestamp_ms,
            rms: scaled_rms,
            peak,
            frequency_bands: smoothed_bands,
            is_silent: false,
        };
        frame.sanitize();
        frame
    }
}
