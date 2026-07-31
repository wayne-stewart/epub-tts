//! Write synthesized audio to WAV or MP3.

use anyhow::{Context, Result};
use any_tts::AudioSamples;
use shine_rs::{Mp3EncoderConfig, StereoMode, encode_pcm_to_mp3};
use std::fs;
use std::path::{Path, PathBuf};

/// Formats we can write from synthesized PCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
}

impl AudioFormat {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("mp3") => Self::Mp3,
            _ => Self::Wav,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
        }
    }
}

/// Save audio; format is inferred from the path extension (`.mp3` → MP3, else WAV).
pub fn save_audio(audio: &AudioSamples, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }

    match AudioFormat::from_path(path) {
        AudioFormat::Wav => audio
            .save_wav(path)
            .with_context(|| format!("write WAV {}", path.display())),
        AudioFormat::Mp3 => save_mp3(audio, path),
    }
}

fn save_mp3(audio: &AudioSamples, path: &Path) -> Result<()> {
    let (samples, sample_rate) = prepare_for_mp3(audio)?;
    let pcm: Vec<i16> = samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect();

    // MPEG-1 (32–48 kHz) allows up to 320 kbps; MPEG-2 (≤24 kHz) caps at 160.
    let bitrate = if sample_rate > 24_000 { 192 } else { 160 };

    let config = Mp3EncoderConfig::new()
        .sample_rate(sample_rate)
        .bitrate(bitrate)
        .channels(1)
        .stereo_mode(StereoMode::Mono)
        .original(true);

    let mp3 = encode_pcm_to_mp3(config, &pcm)
        .map_err(|e| anyhow::anyhow!("MP3 encode failed: {e}"))?;

    fs::write(path, mp3).with_context(|| format!("write MP3 {}", path.display()))?;
    Ok(())
}

/// Ensure sample rate is one shine supports; resample if needed.
fn prepare_for_mp3(audio: &AudioSamples) -> Result<(Vec<f32>, u32)> {
    // MPEG Layer III rates commonly accepted by shine-style encoders.
    const RATES: &[u32] = &[
        8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
    ];

    if RATES.contains(&audio.sample_rate) {
        return Ok((audio.samples.clone(), audio.sample_rate));
    }

    let target = nearest_rate(audio.sample_rate, RATES)
        .with_context(|| format!("no MP3 sample rate near {}", audio.sample_rate))?;
    let resampled = resample_linear(&audio.samples, audio.sample_rate, target);
    Ok((resampled, target))
}

fn nearest_rate(rate: u32, supported: &[u32]) -> Option<u32> {
    supported.iter().copied().min_by_key(|r| rate.abs_diff(*r))
}

fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == to {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let new_len = ((samples.len() as f64) / ratio).round().max(1.0) as usize;
    let last = samples.len() - 1;
    let mut out = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let src = i as f64 * ratio;
        let i0 = (src.floor() as usize).min(last);
        let i1 = (i0 + 1).min(last);
        let t = (src - i0 as f64) as f32;
        out.push(samples[i0] * (1.0 - t) + samples[i1] * t);
    }
    out
}

/// Resolve output path for a chapter, honoring directory vs file and format extension.
pub fn chapter_output_path(
    out: &Path,
    chapter_index: usize,
    multi_chapter: bool,
) -> Result<PathBuf> {
    // Explicit single file (not a multi-chapter run, not an existing directory).
    if !multi_chapter && !out.is_dir() && out.extension().is_some() {
        return Ok(out.to_path_buf());
    }

    let format = AudioFormat::from_path(out);
    let ext = format.extension();

    if multi_chapter && out.extension().is_some() && !out.is_dir() {
        // e.g. -o book.mp3 --to 3 → book-0000.mp3, book-0001.mp3, …
        let stem = out
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("chapter");
        let parent = out.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        return Ok(parent.join(format!("{stem}-{chapter_index:04}.{ext}")));
    }

    // Directory (existing or requested without extension).
    let dir = out.to_path_buf();
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    // Default chapter files in a directory to WAV unless the dir path itself ends with .mp3
    // (unusual). Prefer wav for directories.
    let ext = if matches!(format, AudioFormat::Mp3) {
        "mp3"
    } else {
        "wav"
    };
    Ok(dir.join(format!("{chapter_index:04}.{ext}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_from_extension() {
        assert_eq!(AudioFormat::from_path(Path::new("a.mp3")), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_path(Path::new("a.MP3")), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_path(Path::new("a.wav")), AudioFormat::Wav);
        assert_eq!(AudioFormat::from_path(Path::new("a")), AudioFormat::Wav);
    }

    #[test]
    fn mp3_encode_smoke() {
        let sr = 24_000;
        let samples: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.2)
            .collect();
        let audio = AudioSamples {
            samples,
            sample_rate: sr,
            channels: 1,
        };
        let dir = std::env::temp_dir().join("epub-tts-mp3-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("tone.mp3");
        save_audio(&audio, &path).expect("encode mp3");
        let bytes = fs::read(&path).expect("read mp3");
        assert!(bytes.len() > 100, "mp3 too small: {}", bytes.len());
        let _ = fs::remove_file(&path);
    }
}
