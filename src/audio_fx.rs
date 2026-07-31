//! Lightweight post-processing for synthesized audio.

use any_tts::AudioSamples;

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
}
