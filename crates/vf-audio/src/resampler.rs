use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use crate::error::AudioError;

pub struct AudioResampler {
    resampler: SincFixedIn<f32>,
    input_channel_count: usize,
    chunk_size: usize,
}

impl AudioResampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Result<Self, AudioError> {
        let ratio = to_rate as f64 / from_rate as f64;
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            oversampling_factor: 128,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::BlackmanHarris2,
        };
        let chunk_size = 1024;
        let resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1)
            .map_err(|e| AudioError::Resampler(e.to_string()))?;
        Ok(Self { resampler, input_channel_count: channels, chunk_size })
    }

    pub fn process_to_mono_16k(&mut self, samples: &[f32]) -> Result<Vec<f32>, AudioError> {
        // Mix to mono
        let mono: Vec<f32> = if self.input_channel_count == 1 {
            samples.to_vec()
        } else {
            let ch = self.input_channel_count;
            samples.chunks(ch)
                .map(|frame| frame.iter().sum::<f32>() / ch as f32)
                .collect()
        };

        // Process in chunks
        let mut output = Vec::new();
        let mut pos = 0;
        while pos < mono.len() {
            let end = (pos + self.chunk_size).min(mono.len());
            let chunk = &mono[pos..end];

            // Pad if needed
            let mut padded = chunk.to_vec();
            if padded.len() < self.chunk_size {
                padded.resize(self.chunk_size, 0.0);
            }

            let resampled = self.resampler.process(&[padded], None)
                .map_err(|e| AudioError::Resampler(e.to_string()))?;
            output.extend_from_slice(&resampled[0]);
            pos = end;
        }
        Ok(output)
    }
}
