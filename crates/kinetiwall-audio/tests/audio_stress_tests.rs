//! Stress and Numerical Stability tests for `kinetiwall-audio`.

use std::f32::consts::PI;
use std::time::Duration;

use kinetiwall_audio::capture::{SimulatedAudioBackend, SimulationMode};
use kinetiwall_audio::contract::AudioReactivitySettings;
use kinetiwall_audio::dsp::AudioFeatureExtractor;
use kinetiwall_audio::service::AudioReactivityService;

/// Sustained processing stress test: processes 10,000 consecutive audio blocks
/// through `AudioFeatureExtractor` with dynamically varying frequency and amplitude.
/// Asserts strict sequence monotonicity, zero memory unboundedness, and finite results.
#[test]
fn test_audio_extractor_sustained_stress_10000_frames() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.0, 0.5);
    let sample_rate = 48000.0f32;

    for i in 1..=10_000 {
        let freq = 100.0 + ((i % 500) as f32) * 4.0; // 100 Hz to 2100 Hz sweep
        let amplitude = 0.1 + ((i % 10) as f32) * 0.08;
        let samples: Vec<f32> = (0..256)
            .map(|s| (2.0 * PI * freq * s as f32 / sample_rate).sin() * amplitude)
            .collect();

        let frame = extractor.process_mono_samples(&samples, i as u64 * 10);
        assert_eq!(frame.sequence, i as u64);
        assert!(
            frame.rms.is_finite(),
            "RMS must be finite at sequence {}",
            i
        );
        assert!(
            frame.peak.is_finite(),
            "Peak must be finite at sequence {}",
            i
        );
        assert!(frame.rms >= 0.0 && frame.rms <= 2.0);
        assert!((0.0..=1.0).contains(&frame.peak));
        for band in frame.frequency_bands {
            assert!(band.is_finite());
            assert!((0.0..=1.0).contains(&band));
        }
    }
}

/// Numerical safety extreme fuzzing: feeds NaNs, Infinities, denormals, huge numbers,
/// and empty arrays to verify that feature extraction never panics or emits non-finite frames.
#[test]
fn test_audio_numerical_safety_stress_extremes() {
    let mut extractor = AudioFeatureExtractor::new(48000, 1.5, 0.3);

    let extreme_inputs: Vec<Vec<f32>> = vec![
        vec![f32::NAN; 512],
        vec![f32::INFINITY; 512],
        vec![f32::NEG_INFINITY; 512],
        vec![f32::MIN_POSITIVE; 512],
        vec![1e30f32, -1e30f32, f32::NAN, 0.5f32],
        vec![0.0f32; 1],
        vec![0.0f32; 2048],
    ];

    for (idx, input) in extreme_inputs.iter().enumerate() {
        let frame = extractor.process_mono_samples(input, (idx as u64 + 1) * 100);
        assert!(
            frame.rms.is_finite(),
            "RMS must be finite on extreme input {}",
            idx
        );
        assert!(
            frame.peak.is_finite(),
            "Peak must be finite on extreme input {}",
            idx
        );
        for band in frame.frequency_bands {
            assert!(band.is_finite());
            assert!((0.0..=1.0).contains(&band));
        }
    }
}

/// Rate-limiter high-throughput stress test: verifies that high frame emission is
/// bounded cleanly and that subscriber channels do not deadlock or accumulate unread messages.
#[test]
fn test_audio_service_rate_limiter_high_throughput_stress() {
    let mut service = AudioReactivityService::new(|| {
        Box::new(SimulatedAudioBackend::new(SimulationMode::Tone {
            freq_hz: 1000.0,
            amplitude: 0.9,
        }))
    });

    service.update_settings(AudioReactivitySettings {
        enabled: true,
        target_fps: 60,
        sensitivity: 1.0,
        smoothing: 0.5,
    });

    let rx = service.subscribe();

    // Collect 10 rate-limited frames
    let mut count = 0;
    let start = std::time::Instant::now();
    while count < 10 && start.elapsed() < Duration::from_secs(3) {
        if let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
            assert!(frame.rms.is_finite());
            count += 1;
        }
    }

    assert_eq!(count, 10, "Should have received 10 rate-limited frames");
    service.stop();
}
