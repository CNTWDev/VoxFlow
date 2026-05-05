use std::io::Cursor;
use crate::error::AsrError;

pub fn encode_wav_f32_mono(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, AsrError> {
    // 44-byte WAV header + 4 bytes per f32 sample
    let buf = Vec::with_capacity(44 + samples.len() * 4);
    let mut cursor = Cursor::new(buf);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec)
        .map_err(|e| AsrError::Encoding(e.to_string()))?;
    for &s in samples {
        writer.write_sample(s).map_err(|e| AsrError::Encoding(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AsrError::Encoding(e.to_string()))?;
    Ok(cursor.into_inner())
}
