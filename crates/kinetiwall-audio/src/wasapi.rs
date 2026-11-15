//! Windows WASAPI Loopback Capture native COM implementation.
//!
//! Provides zero-external-dependency COM bindings to `mmdevapi.dll` and `ole32.dll`
//! to capture shared-mode system output audio without microphone access or driver installation.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::ptr;

use crate::contract::AudioCaptureError;
use crate::dsp::{
    convert_f32_le_to_mono, convert_i16_le_to_mono, convert_i24_le_to_mono, convert_i32_le_to_mono,
};

pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const CLSID_MMDEVICE_ENUMERATOR: GUID = GUID {
    data1: 0xBCDE0395,
    data2: 0xE52F,
    data3: 0x467C,
    data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};

pub const IID_IMMDEVICE_ENUMERATOR: GUID = GUID {
    data1: 0xA95664D2,
    data2: 0x9614,
    data3: 0x4F35,
    data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};

pub const IID_IAUDIO_CLIENT: GUID = GUID {
    data1: 0x1CB9AD4C,
    data2: 0xDBFA,
    data3: 0x4C32,
    data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
};

pub const IID_IAUDIO_CAPTURE_CLIENT: GUID = GUID {
    data1: 0xC8ADBD64,
    data2: 0xE71E,
    data3: 0x48A0,
    data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
};

pub const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
    data1: 0x00000003,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
};

pub const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID {
    data1: 0x00000001,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
};

pub const E_RENDER: i32 = 0;
pub const E_CONSOLE: i32 = 0;
pub const CLSCTX_ALL: u32 = 0x17;
pub const AUDCLNT_SHAREMODE_SHARED: i32 = 0;
pub const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
pub const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
pub const COINIT_MULTITHREADED: u32 = 0x0;

pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[repr(C, packed(1))]
#[derive(Debug, Clone, Copy)]
pub struct WAVEFORMATEX {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}

#[repr(C, packed(1))]
#[derive(Debug, Clone, Copy)]
pub struct WAVEFORMATEXTENSIBLE {
    pub Format: WAVEFORMATEX,
    pub Samples: u16,
    pub dwChannelMask: u32,
    pub SubFormat: GUID,
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const GUID,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
    fn CoTaskMemFree(pv: *mut c_void);
}

// COM vtable representations
#[repr(C)]
struct IUnknownVtbl {
    pub QueryInterface: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const GUID,
        ppvObject: *mut *mut c_void,
    ) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

#[repr(C)]
struct IMMDeviceEnumeratorVtbl {
    pub unknown: IUnknownVtbl,
    pub EnumAudioEndpoints: unsafe extern "system" fn(
        this: *mut c_void,
        dataFlow: i32,
        dwStateMask: u32,
        ppEndpoints: *mut *mut c_void,
    ) -> HRESULT,
    pub GetDefaultAudioEndpoint: unsafe extern "system" fn(
        this: *mut c_void,
        dataFlow: i32,
        role: i32,
        ppEndpoint: *mut *mut c_void,
    ) -> HRESULT,
    pub GetDevice: unsafe extern "system" fn(
        this: *mut c_void,
        pwstrId: *const u16,
        ppDevice: *mut *mut c_void,
    ) -> HRESULT,
    pub RegisterEndpointNotificationCallback:
        unsafe extern "system" fn(this: *mut c_void, pClient: *mut c_void) -> HRESULT,
    pub UnregisterEndpointNotificationCallback:
        unsafe extern "system" fn(this: *mut c_void, pClient: *mut c_void) -> HRESULT,
}

#[repr(C)]
struct IMMDeviceVtbl {
    pub unknown: IUnknownVtbl,
    pub Activate: unsafe extern "system" fn(
        this: *mut c_void,
        iid: *const GUID,
        dwClsCtx: u32,
        pActivationParams: *const c_void,
        ppInterface: *mut *mut c_void,
    ) -> HRESULT,
    pub OpenPropertyStore: unsafe extern "system" fn(
        this: *mut c_void,
        stgmAccess: u32,
        ppProperties: *mut *mut c_void,
    ) -> HRESULT,
    pub GetId: unsafe extern "system" fn(this: *mut c_void, ppstrId: *mut *mut u16) -> HRESULT,
    pub GetState: unsafe extern "system" fn(this: *mut c_void, pdwState: *mut u32) -> HRESULT,
}

#[repr(C)]
struct IAudioClientVtbl {
    pub unknown: IUnknownVtbl,
    pub Initialize: unsafe extern "system" fn(
        this: *mut c_void,
        shareMode: i32,
        streamFlags: u32,
        hnsBufferDuration: i64,
        hnsPeriodicity: i64,
        pFormat: *const WAVEFORMATEX,
        AudioSessionGuid: *const GUID,
    ) -> HRESULT,
    pub GetBufferSize:
        unsafe extern "system" fn(this: *mut c_void, pNumBufferFrames: *mut u32) -> HRESULT,
    pub GetStreamLatency:
        unsafe extern "system" fn(this: *mut c_void, phnsLatency: *mut i64) -> HRESULT,
    pub GetCurrentPadding:
        unsafe extern "system" fn(this: *mut c_void, pNumPaddingFrames: *mut u32) -> HRESULT,
    pub IsFormatSupported: unsafe extern "system" fn(
        this: *mut c_void,
        shareMode: i32,
        pFormat: *const WAVEFORMATEX,
        ppClosestMatch: *mut *mut WAVEFORMATEX,
    ) -> HRESULT,
    pub GetMixFormat: unsafe extern "system" fn(
        this: *mut c_void,
        ppDeviceFormat: *mut *mut WAVEFORMATEX,
    ) -> HRESULT,
    pub GetDevicePeriod: unsafe extern "system" fn(
        this: *mut c_void,
        phnsDefaultDevicePeriod: *mut i64,
        phnsMinimumDevicePeriod: *mut i64,
    ) -> HRESULT,
    pub Start: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    pub Stop: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    pub Reset: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    pub SetEventHandle:
        unsafe extern "system" fn(this: *mut c_void, eventHandle: *mut c_void) -> HRESULT,
    pub GetService: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct IAudioCaptureClientVtbl {
    pub unknown: IUnknownVtbl,
    pub GetBuffer: unsafe extern "system" fn(
        this: *mut c_void,
        ppData: *mut *mut u8,
        pNumFramesToRead: *mut u32,
        pdwFlags: *mut u32,
        pu64DevicePosition: *mut u64,
        pu64QPCPosition: *mut u64,
    ) -> HRESULT,
    pub ReleaseBuffer: unsafe extern "system" fn(this: *mut c_void, numFramesRead: u32) -> HRESULT,
    pub GetNextPacketSize:
        unsafe extern "system" fn(this: *mut c_void, pNumFramesInNextPacket: *mut u32) -> HRESULT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSampleFormat {
    IeeeFloat,
    Pcm16,
    Pcm24,
    Pcm32,
}

/// Active native WASAPI loopback capture session.
pub struct WasapiLoopbackSession {
    audio_client: *mut c_void,
    capture_client: *mut c_void,
    channels: usize,
    sample_rate: u32,
    bits_per_sample: u16,
    sample_format: ResolvedSampleFormat,
    com_initialized: bool,
}

// Safety: WasapiLoopbackSession owns its raw COM pointers on the capture worker thread.
unsafe impl Send for WasapiLoopbackSession {}

impl WasapiLoopbackSession {
    /// Attempts to initialize COM, open the default audio rendering endpoint,
    /// configure shared loopback mode, and start the capture stream.
    pub fn open_default() -> Result<Self, AudioCaptureError> {
        let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
        let com_initialized = hr == S_OK || hr == 1; // S_FALSE = 1 (already initialized on this thread)

        let mut enumerator: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_MMDEVICE_ENUMERATOR,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IMMDEVICE_ENUMERATOR,
                &mut enumerator,
            )
        };
        if hr != S_OK || enumerator.is_null() {
            if com_initialized {
                unsafe { CoUninitialize() };
            }
            return Err(AudioCaptureError::InitializationFailed(format!(
                "CoCreateInstance(MMDeviceEnumerator) failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let enum_vtbl = unsafe { &*(*(enumerator as *mut *mut IMMDeviceEnumeratorVtbl)) };
        let mut device: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            (enum_vtbl.GetDefaultAudioEndpoint)(enumerator, E_RENDER, E_CONSOLE, &mut device)
        };

        // Release enumerator
        unsafe { (enum_vtbl.unknown.Release)(enumerator) };

        if hr != S_OK || device.is_null() {
            if com_initialized {
                unsafe { CoUninitialize() };
            }
            return Err(AudioCaptureError::DeviceUnavailable(format!(
                "No default render endpoint found: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let dev_vtbl = unsafe { &*(*(device as *mut *mut IMMDeviceVtbl)) };
        let mut audio_client: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            (dev_vtbl.Activate)(
                device,
                &IID_IAUDIO_CLIENT,
                CLSCTX_ALL,
                ptr::null(),
                &mut audio_client,
            )
        };

        // Release device
        unsafe { (dev_vtbl.unknown.Release)(device) };

        if hr != S_OK || audio_client.is_null() {
            if com_initialized {
                unsafe { CoUninitialize() };
            }
            return Err(AudioCaptureError::InitializationFailed(format!(
                "IMMDevice::Activate(IAudioClient) failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let client_vtbl = unsafe { &*(*(audio_client as *mut *mut IAudioClientVtbl)) };
        let mut pwfx: *mut WAVEFORMATEX = ptr::null_mut();
        let hr = unsafe { (client_vtbl.GetMixFormat)(audio_client, &mut pwfx) };

        if hr != S_OK || pwfx.is_null() {
            unsafe {
                ((client_vtbl.unknown.Release)(audio_client));
                if com_initialized {
                    CoUninitialize();
                }
            }
            return Err(AudioCaptureError::InitializationFailed(format!(
                "IAudioClient::GetMixFormat failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let format = unsafe { *pwfx };
        let (sample_format, bits_per_sample) = match format.wFormatTag {
            WAVE_FORMAT_IEEE_FLOAT => (ResolvedSampleFormat::IeeeFloat, format.wBitsPerSample),
            WAVE_FORMAT_PCM => {
                let fmt = match format.wBitsPerSample {
                    16 => ResolvedSampleFormat::Pcm16,
                    24 => ResolvedSampleFormat::Pcm24,
                    32 => ResolvedSampleFormat::Pcm32,
                    other => {
                        unsafe {
                            CoTaskMemFree(pwfx as *mut c_void);
                            ((client_vtbl.unknown.Release)(audio_client));
                            if com_initialized {
                                CoUninitialize();
                            }
                        }
                        return Err(AudioCaptureError::UnsupportedFormat(format!(
                            "Unsupported PCM bit depth: {} bits",
                            other
                        )));
                    }
                };
                (fmt, format.wBitsPerSample)
            }
            WAVE_FORMAT_EXTENSIBLE => {
                let ext = unsafe { *(pwfx as *const WAVEFORMATEXTENSIBLE) };
                let sub_format = ext.SubFormat;
                if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                    (ResolvedSampleFormat::IeeeFloat, format.wBitsPerSample)
                } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM {
                    let fmt = match format.wBitsPerSample {
                        16 => ResolvedSampleFormat::Pcm16,
                        24 => ResolvedSampleFormat::Pcm24,
                        32 => ResolvedSampleFormat::Pcm32,
                        other => {
                            unsafe {
                                CoTaskMemFree(pwfx as *mut c_void);
                                ((client_vtbl.unknown.Release)(audio_client));
                                if com_initialized {
                                    CoUninitialize();
                                }
                            }
                            return Err(AudioCaptureError::UnsupportedFormat(format!(
                                "Unsupported Extensible PCM bit depth: {} bits",
                                other
                            )));
                        }
                    };
                    (fmt, format.wBitsPerSample)
                } else {
                    unsafe {
                        CoTaskMemFree(pwfx as *mut c_void);
                        ((client_vtbl.unknown.Release)(audio_client));
                        if com_initialized {
                            CoUninitialize();
                        }
                    }
                    return Err(AudioCaptureError::UnsupportedFormat(
                        "Extensible audio format is neither IEEE Float nor PCM".to_string(),
                    ));
                }
            }
            other => {
                unsafe {
                    CoTaskMemFree(pwfx as *mut c_void);
                    ((client_vtbl.unknown.Release)(audio_client));
                    if com_initialized {
                        CoUninitialize();
                    }
                }
                return Err(AudioCaptureError::UnsupportedFormat(format!(
                    "Unsupported wave format tag: 0x{:04X}",
                    other
                )));
            }
        };

        let channels = format.nChannels as usize;
        let sample_rate = format.nSamplesPerSec;

        // 100 milliseconds buffer duration in 100-nanosecond units (100 ms = 1,000,000 hns)
        let buffer_duration_hns: i64 = 1_000_000;
        let hr = unsafe {
            (client_vtbl.Initialize)(
                audio_client,
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                buffer_duration_hns,
                0,
                pwfx,
                ptr::null(),
            )
        };

        unsafe { CoTaskMemFree(pwfx as *mut c_void) };

        if hr != S_OK {
            unsafe {
                ((client_vtbl.unknown.Release)(audio_client));
                if com_initialized {
                    CoUninitialize();
                }
            }
            return Err(AudioCaptureError::InitializationFailed(format!(
                "IAudioClient::Initialize (Loopback) failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let mut capture_client: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            (client_vtbl.GetService)(
                audio_client,
                &IID_IAUDIO_CAPTURE_CLIENT,
                &mut capture_client,
            )
        };

        if hr != S_OK || capture_client.is_null() {
            unsafe {
                ((client_vtbl.unknown.Release)(audio_client));
                if com_initialized {
                    CoUninitialize();
                }
            }
            return Err(AudioCaptureError::InitializationFailed(format!(
                "IAudioClient::GetService(IAudioCaptureClient) failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        // Start capture stream
        let hr = unsafe { (client_vtbl.Start)(audio_client) };
        if hr != S_OK {
            let cap_vtbl = unsafe { &*(*(capture_client as *mut *mut IAudioCaptureClientVtbl)) };
            unsafe {
                ((cap_vtbl.unknown.Release)(capture_client));
                ((client_vtbl.unknown.Release)(audio_client));
                if com_initialized {
                    CoUninitialize();
                }
            }
            return Err(AudioCaptureError::StreamError(format!(
                "IAudioClient::Start failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        Ok(Self {
            audio_client,
            capture_client,
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
            com_initialized,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Reads next available audio packet, returning mono f32 samples.
    pub fn read_packet(&mut self) -> Result<Option<Vec<f32>>, AudioCaptureError> {
        if self.capture_client.is_null() {
            return Ok(None);
        }

        let cap_vtbl = unsafe { &*(*(self.capture_client as *mut *mut IAudioCaptureClientVtbl)) };

        let mut packet_size: u32 = 0;
        let hr = unsafe { (cap_vtbl.GetNextPacketSize)(self.capture_client, &mut packet_size) };
        if hr != S_OK || packet_size == 0 {
            return Ok(None);
        }

        let mut p_data: *mut u8 = ptr::null_mut();
        let mut num_frames: u32 = 0;
        let mut flags: u32 = 0;
        let mut dev_pos: u64 = 0;
        let mut qpc_pos: u64 = 0;

        let hr = unsafe {
            (cap_vtbl.GetBuffer)(
                self.capture_client,
                &mut p_data,
                &mut num_frames,
                &mut flags,
                &mut dev_pos,
                &mut qpc_pos,
            )
        };

        if hr != S_OK {
            return Err(AudioCaptureError::StreamError(format!(
                "IAudioCaptureClient::GetBuffer failed: HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        if num_frames == 0 || p_data.is_null() {
            let _ = unsafe { (cap_vtbl.ReleaseBuffer)(self.capture_client, num_frames) };
            return Ok(None);
        }

        let mono = if flags & AUDCLNT_BUFFERFLAGS_SILENT != 0 {
            vec![0.0f32; num_frames as usize]
        } else {
            let bytes_per_sample = (self.bits_per_sample / 8) as usize;
            let total_bytes = num_frames as usize * self.channels * bytes_per_sample;
            let slice = unsafe { std::slice::from_raw_parts(p_data, total_bytes) };

            match self.sample_format {
                ResolvedSampleFormat::IeeeFloat => convert_f32_le_to_mono(slice, self.channels),
                ResolvedSampleFormat::Pcm16 => convert_i16_le_to_mono(slice, self.channels),
                ResolvedSampleFormat::Pcm24 => convert_i24_le_to_mono(slice, self.channels),
                ResolvedSampleFormat::Pcm32 => convert_i32_le_to_mono(slice, self.channels),
            }
        };

        let _ = unsafe { (cap_vtbl.ReleaseBuffer)(self.capture_client, num_frames) };
        Ok(Some(mono))
    }
}

impl Drop for WasapiLoopbackSession {
    fn drop(&mut self) {
        if !self.capture_client.is_null() {
            let cap_vtbl =
                unsafe { &*(*(self.capture_client as *mut *mut IAudioCaptureClientVtbl)) };
            unsafe { (cap_vtbl.unknown.Release)(self.capture_client) };
            self.capture_client = ptr::null_mut();
        }

        if !self.audio_client.is_null() {
            let client_vtbl = unsafe { &*(*(self.audio_client as *mut *mut IAudioClientVtbl)) };
            unsafe {
                let _ = (client_vtbl.Stop)(self.audio_client);
                (client_vtbl.unknown.Release)(self.audio_client);
            }
            self.audio_client = ptr::null_mut();
        }

        if self.com_initialized {
            unsafe { CoUninitialize() };
            self.com_initialized = false;
        }
    }
}
