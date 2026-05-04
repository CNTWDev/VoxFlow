use std::io::Cursor;
use crate::error::AsrError;

pub fn encode_wav_f32_mono(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, AsrError> {
    let mut buf = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(&mut buf, spec)
        .map_err(|e| AsrError::Encoding(e.to_string()))?;
    for &s in samples {
        writer.write_sample(s).map_err(|e| AsrError::Encoding(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AsrError::Encoding(e.to_string()))?;
    Ok(buf.into_inner())
}
