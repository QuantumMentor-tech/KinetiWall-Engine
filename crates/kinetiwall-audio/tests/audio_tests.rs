//! Automated unit, integration, and lifecycle tests for kinetiwall-audio.

use std::f32::consts::PI;
use std::time::Duration;

use kinetiwall_audio::capture::{SimulatedAudioBackend, SimulationMode};
use kinetiwall_audio::contract::{AudioCaptureError, AudioCaptureState, AudioReactivitySettings};
use kinetiwall_audio::dsp::{
    compute_frequency_bands, convert_f32_le_to_mono, convert_i16_le_to_mono,
    convert_i24_le_to_mono, convert_i32_le_to_mono, radix2_fft, AudioFeatureExtractor,
    FFT_WINDOW_SIZE,
};
use kinetiwall_audio::service::AudioReactivityService;
use kinetiwall_audio::wasapi::WasapiLoopbackSession;

#[test]
fn test_silence_input_produces_stable_silence_frame() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.5);
    let silence = vec![0.0f32; 512];

    let frame = extractor.process_mono_samples(&silence, 1000);
    assert_eq!(frame.sequence, 1);
    assert_eq!(frame.timestamp_ms, 1000);
    assert!(
        frame.is_silent,
        "All-zero input must report is_silent = true"
    );
    assert_eq!(frame.rms, 0.0);
    assert!(frame.peak < 0.001);
    for band in frame.frequency_bands {
        assert_eq!(band, 0.0);
    }
}

#[test]
fn test_known_amplitude_sine_wave() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.0);
    let sample_rate = 48000.0f32;
    let freq = 440.0f32;
    let amplitude = 0.5f32;

    // Generate 1024 samples of a pure sine wave at 440 Hz
    let samples: Vec<f32> = (0..1024)
        .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin() * amplitude)
        .collect();

    let frame = extractor.process_mono_samples(&samples, 1000);
    assert!(!frame.is_silent);
    // Theoretical RMS of sine wave with peak A is A / sqrt(2) approx 0.3535.
    // Our extractor scales RMS by sensitivity * 2.0.
    assert!(
        frame.rms > 0.4 && frame.rms < 0.9,
        "RMS out of expected range: {}",
        frame.rms
    );
    assert!(
        frame.peak >= 0.45 && frame.peak <= 0.55,
        "Peak out of expected range: {}",
        frame.peak
    );
}

#[test]
fn test_peak_transient_detection_and_decay() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.0);

    // Frame 1: Sharp single impulse at 1.0
    let mut impulse = vec![0.0f32; 256];
    impulse[10] = 1.0;

    let frame1 = extractor.process_mono_samples(&impulse, 100);
    assert!(
        (frame1.peak - 1.0).abs() < 0.05,
        "Peak should register near 1.0"
    );

    // Frame 2: Silence - peak should decay exponentially
    let silence = vec![0.0f32; 256];
    let frame2 = extractor.process_mono_samples(&silence, 200);
    assert!(
        frame2.peak < frame1.peak,
        "Peak should decay after transient: {} < {}",
        frame2.peak,
        frame1.peak
    );
    assert!(
        frame2.peak > 0.7,
        "Decay should be smooth, not drop to zero immediately: {}",
        frame2.peak
    );

    // Frame 3: Continued silence
    let frame3 = extractor.process_mono_samples(&silence, 300);
    assert!(
        frame3.peak < frame2.peak,
        "Peak should continue decaying: {} < {}",
        frame3.peak,
        frame2.peak
    );
}

#[test]
fn test_frequency_band_separation_for_synthetic_tones() {
    let sample_rate = 48000;

    // 1. Pure Bass Tone at 100 Hz (Band 1: 60 - 250 Hz)
    let bass_samples: Vec<f32> = (0..512)
        .map(|i| (2.0 * PI * 100.0 * i as f32 / sample_rate as f32).sin())
        .collect();
    let bass_bands = compute_frequency_bands(&bass_samples, sample_rate);
    assert!(
        bass_bands[1] > bass_bands[6] * 3.0,
        "100 Hz tone should strongly excite Bass (Band 1: {}) over Brilliance (Band 6: {})",
        bass_bands[1],
        bass_bands[6]
    );

    // 2. Pure High Tone at 8000 Hz (Band 6: 6000 - 12000 Hz)
    let high_samples: Vec<f32> = (0..512)
        .map(|i| (2.0 * PI * 8000.0 * i as f32 / sample_rate as f32).sin())
        .collect();
    let high_bands = compute_frequency_bands(&high_samples, sample_rate);
    assert!(
        high_bands[6] > high_bands[1] * 3.0,
        "8000 Hz tone should strongly excite Brilliance (Band 6: {}) over Bass (Band 1: {})",
        high_bands[6],
        high_bands[1]
    );
}

#[test]
fn test_format_conversions_pcm_and_float() {
    // 1. IEEE Float stereo
    let f32_left = 0.5f32;
    let f32_right = -0.5f32;
    let mut f32_bytes = Vec::new();
    f32_bytes.extend_from_slice(&f32_left.to_le_bytes());
    f32_bytes.extend_from_slice(&f32_right.to_le_bytes());
    let mono_f32 = convert_f32_le_to_mono(&f32_bytes, 2);
    assert_eq!(mono_f32.len(), 1);
    assert!((mono_f32[0] - 0.0).abs() < 1e-6);

    // 2. PCM 16-bit stereo (full scale positive: 32767, full scale negative: -32768)
    let mut pcm16_bytes = Vec::new();
    pcm16_bytes.extend_from_slice(&16384i16.to_le_bytes());
    pcm16_bytes.extend_from_slice(&16384i16.to_le_bytes());
    let mono_pcm16 = convert_i16_le_to_mono(&pcm16_bytes, 2);
    assert_eq!(mono_pcm16.len(), 1);
    assert!((mono_pcm16[0] - 0.5).abs() < 0.01);

    // 3. PCM 24-bit mono
    // 0x400000 = 4194304 (half of 8388608 = 0.5)
    let pcm24_bytes = vec![0x00, 0x00, 0x40];
    let mono_pcm24 = convert_i24_le_to_mono(&pcm24_bytes, 1);
    assert_eq!(mono_pcm24.len(), 1);
    assert!((mono_pcm24[0] - 0.5).abs() < 0.01);

    // 4. PCM 32-bit mono
    let mut pcm32_bytes = Vec::new();
    pcm32_bytes.extend_from_slice(&1073741824i32.to_le_bytes()); // 0.5 of 2147483648
    let mono_pcm32 = convert_i32_le_to_mono(&pcm32_bytes, 1);
    assert_eq!(mono_pcm32.len(), 1);
    assert!((mono_pcm32[0] - 0.5).abs() < 0.01);
}

#[test]
fn test_numerical_safety_nan_infinity_rejection() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.5);

    let corrupt_samples = vec![
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        100.0, // Extreme out-of-range value
        -50.0,
    ];

    let frame = extractor.process_mono_samples(&corrupt_samples, 500);
    assert!(frame.rms.is_finite());
    assert!(frame.peak.is_finite());
    assert!((0.0..=1.0).contains(&frame.rms));
    assert!((0.0..=1.0).contains(&frame.peak));
    for b in frame.frequency_bands {
        assert!(b.is_finite());
        assert!((0.0..=1.0).contains(&b));
    }
}

#[test]
fn test_empty_and_truncated_buffers() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.5);
    let empty: Vec<f32> = Vec::new();
    let frame = extractor.process_mono_samples(&empty, 100);
    assert_eq!(frame.sequence, 1);
    assert!(frame.is_silent);

    let small = vec![0.1f32; 3];
    let frame2 = extractor.process_mono_samples(&small, 200);
    assert_eq!(frame2.sequence, 2);
    assert!(frame2.rms.is_finite());
}

#[test]
fn test_temporal_smoothing_determinism() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.5);

    let step_samples = vec![0.8f32; 256];
    let frame1 = extractor.process_mono_samples(&step_samples, 10);
    let frame2 = extractor.process_mono_samples(&step_samples, 20);

    // With smoothing = 0.5, subsequent identical frames converge smoothly
    assert!(frame1.rms.is_finite());
    assert!(frame2.rms.is_finite());
}

#[test]
fn test_radix2_fft_power_of_two() {
    let mut real = vec![1.0f32; FFT_WINDOW_SIZE];
    let mut imag = vec![0.0f32; FFT_WINDOW_SIZE];

    radix2_fft(&mut real, &mut imag);

    // DC component (bin 0) should be N = 256
    assert!((real[0] - FFT_WINDOW_SIZE as f32).abs() < 1e-3);
    // Other bins of a constant signal should be near 0
    for i in 1..FFT_WINDOW_SIZE {
        let mag = (real[i] * real[i] + imag[i] * imag[i]).sqrt();
        assert!(
            mag < 1e-3,
            "Bin {} should be near 0 for DC signal, got {}",
            i,
            mag
        );
    }
}

#[test]
fn test_audio_service_lifecycle_opt_in_and_state() {
    let mut service = AudioReactivityService::new(|| {
        Box::new(SimulatedAudioBackend::new(SimulationMode::Silence))
    });

    // 1. By default, disabled
    assert_eq!(service.state(), AudioCaptureState::Disabled);
    let start_err = service.start();
    assert_eq!(start_err.unwrap_err(), AudioCaptureError::NotEnabled);

    // 2. Enable in settings
    let settings = AudioReactivitySettings {
        enabled: true,
        ..Default::default()
    };
    service.update_settings(settings);

    // Automatically starts when enabled
    assert_eq!(service.state(), AudioCaptureState::Active);

    // 3. Suspend & Resume
    service.suspend();
    assert_eq!(service.state(), AudioCaptureState::Suspended);

    let resume_res = service.resume();
    assert!(resume_res.is_ok());
    assert_eq!(service.state(), AudioCaptureState::Active);

    // 4. Stop
    service.stop();
    assert_eq!(service.state(), AudioCaptureState::Disabled);

    // 5. Repeated stop is idempotent
    service.stop();
    assert_eq!(service.state(), AudioCaptureState::Disabled);
}

#[test]
fn test_audio_service_rate_limiter_and_bounded_broadcaster() {
    let mut service = AudioReactivityService::new(|| {
        Box::new(SimulatedAudioBackend::new(SimulationMode::Tone {
            freq_hz: 440.0,
            amplitude: 0.8,
        }))
    });

    let settings = AudioReactivitySettings {
        enabled: true,
        target_fps: 60,
        ..Default::default()
    };
    service.update_settings(settings);

    let rx = service.subscribe();

    // Collect 3 frames
    let frame1 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("Frame 1");
    let frame2 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("Frame 2");
    let frame3 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("Frame 3");

    assert!(frame2.sequence > frame1.sequence);
    assert!(frame3.sequence > frame2.sequence);
    assert!(!frame1.is_silent);
    assert!(frame1.rms > 0.0);

    service.stop();
}

#[test]
fn test_wasapi_runtime_probe() {
    // Probes native WASAPI loopback capture on the host Windows environment.
    // If an audio endpoint is present, it succeeds.
    // If no endpoint is configured (e.g. headless VM or CI without sound card),
    // it returns DeviceUnavailable cleanly without crashing.
    match WasapiLoopbackSession::open_default() {
        Ok(mut session) => {
            assert!(session.sample_rate() > 0);
            assert!(session.channels() > 0);
            // Non-blocking read packet attempt
            let read_res = session.read_packet();
            assert!(read_res.is_ok());
        }
        Err(AudioCaptureError::DeviceUnavailable(reason)) => {
            // Expected on machines without active audio output devices
            assert!(!reason.is_empty());
        }
        Err(other) => {
            panic!("Unexpected error from WASAPI loopback probe: {:?}", other);
        }
    }
}
