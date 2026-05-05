use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::FromSample;
use tokio::sync::mpsc;
use crate::error::AudioError;

pub struct AudioCapture {
    _stream: cpal::Stream,
    pub rx: mpsc::Receiver<Vec<f32>>,
    /// Native device sample rate — callers must resample to TARGET_SAMPLE_RATE themselves.
    pub native_rate: u32,
}

impl AudioCapture {
    pub fn start() -> Result<Self, AudioError> {
        ensure_microphone_permission()?;

        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or(AudioError::NoInputDevice)?;

        let config = device.default_input_config()
            .map_err(|e| AudioError::UnsupportedConfig(e.to_string()))?;

        let native_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        tracing::info!("audio device: {} | rate: {}Hz | channels: {}",
            device.name().unwrap_or_default(), native_rate, channels);

        let (tx, rx) = mpsc::channel::<Vec<f32>>(128);

        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        let stream = match sample_format {
            cpal::SampleFormat::F32 => Self::build_stream::<f32>(&device, &stream_config, channels, tx),
            cpal::SampleFormat::I16 => Self::build_stream::<i16>(&device, &stream_config, channels, tx),
            cpal::SampleFormat::U16 => Self::build_stream::<u16>(&device, &stream_config, channels, tx),
            _ => Err(AudioError::UnsupportedConfig("unsupported sample format".into())),
        }?;

        stream.play().map_err(|e| AudioError::Stream(e.to_string()))?;

        Ok(Self { _stream: stream, rx, native_rate })
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        tx: mpsc::Sender<Vec<f32>>,
    ) -> Result<cpal::Stream, AudioError>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: FromSample<T>,
    {
        let stream = device.build_input_stream(
            config,
            move |data: &[T], _| {
                // Lightweight: format-convert then mono-mix. No allocation beyond output Vec,
                // no mutex, no resampling — keeps the audio thread unblocked.
                let mono: Vec<f32> = if channels == 1 {
                    data.iter().map(|&s| f32::from_sample_(s)).collect()
                } else {
                    data.chunks(channels)
                        .map(|frame| {
                            frame.iter().map(|&s| f32::from_sample_(s)).sum::<f32>()
                                / channels as f32
                        })
                        .collect()
                };
                let _ = tx.try_send(mono);
            },
            |e| tracing::error!("input stream error: {e}"),
            None,
        ).map_err(|e| AudioError::Stream(e.to_string()))?;

        Ok(stream)
    }
}

#[cfg(target_os = "macos")]
fn ensure_microphone_permission() -> Result<(), AudioError> {
    match macos_microphone_authorization_status() {
        MicrophoneAuthorizationStatus::Authorized => Ok(()),
        MicrophoneAuthorizationStatus::NotDetermined => {
            match macos_request_microphone_access() {
                Ok(true) => Ok(()),
                Ok(false) => Err(AudioError::Stream(
                    "microphone permission denied; enable Microphone for Vox Flow in System Settings".into(),
                )),
                Err(e) => Err(e),
            }
        }
        MicrophoneAuthorizationStatus::Denied | MicrophoneAuthorizationStatus::Restricted => {
            Err(AudioError::Stream(
                "microphone permission is disabled for this app; enable Microphone for Vox Flow in System Settings and restart the app".into(),
            ))
        }
        MicrophoneAuthorizationStatus::Unknown(code) => Err(AudioError::Stream(format!(
            "unknown microphone permission status: {code}"
        ))),
    }
}

#[cfg(target_os = "macos")]
pub fn microphone_permission_granted() -> bool {
    matches!(
        macos_microphone_authorization_status(),
        MicrophoneAuthorizationStatus::Authorized
    )
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_permission_granted() -> bool {
    true
}

#[cfg(target_os = "macos")]
pub fn request_microphone_permission() -> Result<bool, AudioError> {
    match macos_microphone_authorization_status() {
        MicrophoneAuthorizationStatus::Authorized => return Ok(true),
        MicrophoneAuthorizationStatus::Denied | MicrophoneAuthorizationStatus::Restricted => {
            return Ok(false);
        }
        MicrophoneAuthorizationStatus::Unknown(code) => {
            return Err(AudioError::Stream(format!(
                "unknown microphone permission status: {code}"
            )));
        }
        MicrophoneAuthorizationStatus::NotDetermined => {}
    }

    macos_request_microphone_access()
}

#[cfg(not(target_os = "macos"))]
pub fn request_microphone_permission() -> Result<bool, AudioError> {
    Ok(true)
}

#[cfg(not(target_os = "macos"))]
fn ensure_microphone_permission() -> Result<(), AudioError> {
    Ok(())
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneAuthorizationStatus {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
    Unknown(isize),
}

#[cfg(target_os = "macos")]
fn macos_microphone_authorization_status() -> MicrophoneAuthorizationStatus {
    use std::ffi::c_void;

    #[link(name = "objc", kind = "dylib")]
    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *mut c_void;
    }

    type AuthorizationStatusForMediaType =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> isize;

    unsafe {
        let class = objc_getClass(c"AVCaptureDevice".as_ptr());
        if class.is_null() {
            return MicrophoneAuthorizationStatus::Unknown(-1);
        }

        let selector = sel_registerName(c"authorizationStatusForMediaType:".as_ptr());
        if selector.is_null() {
            return MicrophoneAuthorizationStatus::Unknown(-2);
        }

        let msg_send: AuthorizationStatusForMediaType = std::mem::transmute(objc_msgSend as *const ());
        match msg_send(class, selector, AVMediaTypeAudio) {
            0 => MicrophoneAuthorizationStatus::NotDetermined,
            1 => MicrophoneAuthorizationStatus::Restricted,
            2 => MicrophoneAuthorizationStatus::Denied,
            3 => MicrophoneAuthorizationStatus::Authorized,
            code => MicrophoneAuthorizationStatus::Unknown(code),
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_request_microphone_access() -> Result<bool, AudioError> {
    use std::ffi::c_void;
    use std::sync::mpsc;
    use std::time::Duration;

    tracing::info!("requesting microphone access via AVCaptureDevice");

    #[link(name = "objc", kind = "dylib")]
    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *mut c_void;
    }

    type RequestAccessForMediaType = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const c_void,  // ObjC block: thin pointer, not fat Rust trait-object pointer
    );

    let (tx, rx) = mpsc::channel();
    let completion = block2::RcBlock::new(move |granted: i8| {
        let _ = tx.send(granted != 0);
    });

    unsafe {
        let class = objc_getClass(c"AVCaptureDevice".as_ptr());
        if class.is_null() {
            tracing::error!("AVCaptureDevice class is null");
            return Err(AudioError::Stream(
                "AVCaptureDevice class is unavailable".into(),
            ));
        }

        let selector = sel_registerName(c"requestAccessForMediaType:completionHandler:".as_ptr());
        if selector.is_null() {
            tracing::error!("requestAccessForMediaType selector is null");
            return Err(AudioError::Stream(
                "AVCaptureDevice requestAccessForMediaType selector is unavailable".into(),
            ));
        }

        tracing::info!("calling requestAccessForMediaType:completionHandler:");
        let msg_send: RequestAccessForMediaType = std::mem::transmute(objc_msgSend as *const ());
        let block_ptr = block2::RcBlock::as_ptr(&completion).cast::<c_void>();
        msg_send(
            class,
            selector,
            AVMediaTypeAudio,
            block_ptr,
        );
    }

    let result = rx.recv_timeout(Duration::from_secs(60))
        .map_err(|e| AudioError::Stream(format!("microphone permission prompt timed out: {e}")));
    tracing::info!("microphone access request result: {result:?}");
    result
}
