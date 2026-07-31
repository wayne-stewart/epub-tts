//! Lightweight post-processing for synthesized audio.

use anyhow::{Result, bail};
use any_tts::AudioSamples;

/// Join two synthesized pieces with minimal dead air at the boundary.
///
/// Each TTS call tends to add leading/trailing silence, and we used to insert
/// extra silence between chunks — both caused audible pauses. This trims edge
/// silence and applies a short crossfade so speech flows more naturally.
pub fn join_chunks(mut a: AudioSamples, b: AudioSamples) -> Result<AudioSamples> {
    if a.sample_rate != b.sample_rate {
        bail!(
            "sample rate mismatch when concatenating audio: {} vs {}",
            a.sample_rate,
            b.sample_rate
        );
    }
    if a.samples.is_empty() {
        return Ok(b);
    }
    if b.samples.is_empty() {
        return Ok(a);
    }

    let sr = a.sample_rate;
    // Keep a touch of room tone so words don't slam together (~25ms).
    let keep_ms = 0.025;
    // Crossfade length for a soft seam (~12ms).
    let fade_ms = 0.012;

    let a_trim = trim_silence_edges(&a.samples, sr, false, true, keep_ms);
    let b_trim = trim_silence_edges(&b.samples, sr, true, false, keep_ms);

    a.samples = crossfade_concat(&a_trim, &b_trim, sr, fade_ms);
    Ok(a)
}

/// Trim leading and/or trailing near-silence, keeping a short pad.
fn trim_silence_edges(
    samples: &[f32],
    sample_rate: u32,
    trim_lead: bool,
    trim_trail: bool,
    keep_secs: f32,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    const THRESH: f32 = 0.012;
    let pad = ((sample_rate as f32) * keep_secs).round() as usize;

    let mut start = 0usize;
    if trim_lead {
        while start < samples.len() && samples[start].abs() < THRESH {
            start += 1;
        }
        start = start.saturating_sub(pad);
    }

    let mut end = samples.len();
    if trim_trail {
        while end > start && samples[end - 1].abs() < THRESH {
            end -= 1;
        }
        end = (end + pad).min(samples.len());
    }

    if start >= end {
        return samples.to_vec();
    }
    samples[start..end].to_vec()
}

/// Concatenate with a linear equal-power-ish crossfade over `fade_secs`.
fn crossfade_concat(a: &[f32], b: &[f32], sample_rate: u32, fade_secs: f32) -> Vec<f32> {
    let fade = ((sample_rate as f32) * fade_secs).round() as usize;
    let fade = fade.min(a.len()).min(b.len());

    if fade == 0 {
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        return out;
    }

    let a_head = &a[..a.len() - fade];
    let a_tail = &a[a.len() - fade..];
    let b_head = &b[..fade];
    let b_tail = &b[fade..];

    let mut out = Vec::with_capacity(a_head.len() + fade + b_tail.len());
    out.extend_from_slice(a_head);
    for i in 0..fade {
        let t = (i as f32 + 1.0) / (fade as f32 + 1.0);
        // Equal-power crossfade curves reduce a mid-fade dip.
        let fade_out = (std::f32::consts::FRAC_PI_2 * (1.0 - t)).cos();
        let fade_in = (std::f32::consts::FRAC_PI_2 * t).sin();
        out.push((a_tail[i] * fade_out + b_head[i] * fade_in).clamp(-1.0, 1.0));
    }
    out.extend_from_slice(b_tail);
    out
}

/// Change playback speed by resampling the timeline.
///
/// `factor` > 1.0 is faster (e.g. `1.5` = 1.5×). Pitch rises with speed, same as
/// a typical media-player speed control without time-stretching.
pub fn change_speed(audio: &AudioSamples, factor: f32) -> AudioSamples {
    if !factor.is_finite() || factor <= 0.0 {
        return audio.clone();
    }
    if (factor - 1.0).abs() < 1e-3 || audio.samples.is_empty() {
        return audio.clone();
    }

    let src = &audio.samples;
    let new_len = ((src.len() as f32) / factor).round().max(1.0) as usize;
    let last = src.len() - 1;
    let mut samples = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_pos = i as f32 * factor;
        let i0 = (src_pos.floor() as usize).min(last);
        let i1 = (i0 + 1).min(last);
        let t = src_pos - i0 as f32;
        let s = src[i0] * (1.0 - t) + src[i1] * t;
        samples.push(s.clamp(-1.0, 1.0));
    }

    AudioSamples {
        samples,
        sample_rate: audio.sample_rate,
        channels: audio.channels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_1_5_shortens_audio() {
        let n = 15_000;
        let audio = AudioSamples {
            samples: (0..n).map(|i| (i as f32 / n as f32).sin()).collect(),
            sample_rate: 24_000,
            channels: 1,
        };
        let fast = change_speed(&audio, 1.5);
        let expected = (n as f32 / 1.5).round() as usize;
        assert_eq!(fast.sample_rate, 24_000);
        assert!((fast.samples.len() as i64 - expected as i64).abs() <= 1);
        assert!((fast.duration_secs() - audio.duration_secs() / 1.5).abs() < 0.01);
    }

    #[test]
    fn join_trims_boundary_silence() {
        let sr = 24_000u32;
        let tone: Vec<f32> = (0..sr / 10)
            .map(|i| (i as f32 * 0.1).sin() * 0.5)
            .collect();
        let silence = vec![0.0f32; sr as usize / 5]; // 200ms

        let mut a_samples = tone.clone();
        a_samples.extend_from_slice(&silence);
        let mut b_samples = silence.clone();
        b_samples.extend_from_slice(&tone);

        let a = AudioSamples {
            samples: a_samples,
            sample_rate: sr,
            channels: 1,
        };
        let b = AudioSamples {
            samples: b_samples,
            sample_rate: sr,
            channels: 1,
        };

        let naive = a.samples.len() + b.samples.len();
        let joined = join_chunks(a, b).unwrap();
        // Should drop most of the ~400ms stacked silence at the seam.
        assert!(
            joined.samples.len() < naive - (sr as usize / 5),
            "expected silence trim, got {} vs naive {}",
            joined.samples.len(),
            naive
        );
    }
}
