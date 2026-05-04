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
