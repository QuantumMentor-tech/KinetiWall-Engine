//! KinetiWall Real-Time Audio Reactivity & WASAPI Loopback Integration.
//!
//! Provides opt-in Windows audio loopback capture, bounded DSP feature extraction,
//! and strongly-typed visualizer contracts for live wallpaper animations.

pub mod capture;
pub mod contract;
pub mod dsp;
pub mod service;
pub mod wasapi;

pub use capture::{
    AudioCaptureBackend, NativeWasapiBackend, SimulatedAudioBackend, SimulationMode,
};
pub use contract::{
    AudioCaptureError, AudioCaptureState, AudioReactivitySettings, VisualizerFrame,
    NUM_FREQUENCY_BANDS,
};
pub use dsp::{
    compute_frequency_bands, radix2_fft, AudioFeatureExtractor, BAND_RANGES_HZ, FFT_WINDOW_SIZE,
    SILENCE_THRESHOLD,
};
pub use service::AudioReactivityService;
pub use wasapi::{ResolvedSampleFormat, WasapiLoopbackSession};
