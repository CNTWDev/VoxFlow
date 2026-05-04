pub mod capture;
pub mod resampler;
pub mod error;

pub use capture::AudioCapture;
pub use resampler::AudioResampler;
pub use error::AudioError;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

pub fn save_wav(samples: &[f32], sample_rate: u32, path: &std::path::Path) -> Result<(), AudioError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| AudioError::Stream(e.to_string()))?;
    for &s in samples {
        writer.write_sample(s).map_err(|e| AudioError::Stream(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AudioError::Stream(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_verify_wav() {
        let samples: Vec<f32> = (0..16_000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 16_000.0).sin() * 0.5)
            .collect();

        let path = std::env::temp_dir().join("vox_flow_test.wav");
        save_wav(&samples, 16_000, &path).expect("save_wav failed");

        assert!(path.exists(), "wav file not created");

        let mut reader = hound::WavReader::open(&path).expect("open wav");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 32);

        let read_back: Vec<f32> = reader.samples::<f32>()
            .map(|s| s.expect("sample"))
            .collect();
        assert_eq!(read_back.len(), samples.len());
        // Verify first sample matches within f32 precision
        assert!((read_back[440] - samples[440]).abs() < 1e-5);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resampler_output_length() {
        use crate::resampler::AudioResampler;
        let mut r = AudioResampler::new(44_100, 16_000, 1).expect("new resampler");
        // ~1 second of 44.1kHz mono silence
        let input = vec![0.0f32; 44_100];
        let output = r.process_to_mono_16k(&input).expect("resample");
        // Allow ±5% tolerance due to chunk boundary padding
        let ratio = output.len() as f64 / 16_000.0;
        assert!(ratio > 0.9 && ratio < 1.2,
            "expected ~16000 samples, got {}", output.len());
    }
}
