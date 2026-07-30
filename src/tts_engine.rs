//! any-tts model loading and synthesis.

use anyhow::{Context, Result, bail};
use any_tts::{
    AudioSamples, DeviceSelection, ModelType, SynthesisRequest, TtsConfig, TtsModel, load_model,
};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Kokoro,
    OmniVoice,
    Qwen3Tts,
    VibeVoice,
    VibeVoiceRealtime,
    Voxtral,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kokoro => "kokoro",
            Self::OmniVoice => "omnivoice",
            Self::Qwen3Tts => "qwen3",
            Self::VibeVoice => "vibevoice",
            Self::VibeVoiceRealtime => "vibevoice-realtime",
            Self::Voxtral => "voxtral",
        }
    }

    pub fn model_type(self) -> Result<ModelType> {
        // Compile-time feature gates: non-Kokoro backends need an explicit cargo feature.
        match self {
            Self::Kokoro => Ok(ModelType::Kokoro),
            #[cfg(feature = "omnivoice")]
            Self::OmniVoice => Ok(ModelType::OmniVoice),
            #[cfg(not(feature = "omnivoice"))]
            Self::OmniVoice => bail!("backend 'omnivoice' requires --features omnivoice"),
            #[cfg(feature = "qwen3-tts")]
            Self::Qwen3Tts => Ok(ModelType::Qwen3Tts),
            #[cfg(not(feature = "qwen3-tts"))]
            Self::Qwen3Tts => bail!("backend 'qwen3' requires --features qwen3-tts"),
            #[cfg(feature = "vibevoice")]
            Self::VibeVoice => Ok(ModelType::VibeVoice),
            #[cfg(not(feature = "vibevoice"))]
            Self::VibeVoice => bail!("backend 'vibevoice' requires --features vibevoice"),
            #[cfg(feature = "vibevoice")]
            Self::VibeVoiceRealtime => Ok(ModelType::VibeVoiceRealtime),
            #[cfg(not(feature = "vibevoice"))]
            Self::VibeVoiceRealtime => {
                bail!("backend 'vibevoice-realtime' requires --features vibevoice")
            }
            #[cfg(feature = "voxtral")]
            Self::Voxtral => Ok(ModelType::Voxtral),
            #[cfg(not(feature = "voxtral"))]
            Self::Voxtral => bail!("backend 'voxtral' requires --features voxtral"),
        }
    }
}

impl FromStr for Backend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "kokoro" => Ok(Self::Kokoro),
            "omnivoice" | "omni" => Ok(Self::OmniVoice),
            "qwen3" | "qwen3-tts" | "qwen" => Ok(Self::Qwen3Tts),
            "vibevoice" | "vibe" => Ok(Self::VibeVoice),
            "vibevoice-realtime" | "vibe-realtime" | "realtime" => Ok(Self::VibeVoiceRealtime),
            "voxtral" => Ok(Self::Voxtral),
            other => bail!(
                "unknown backend '{other}'. Choose: kokoro, omnivoice, qwen3, vibevoice, vibevoice-realtime, voxtral"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TtsOptions {
    pub backend: Backend,
    pub model_path: Option<PathBuf>,
    pub device: DeviceKind,
    pub language: Option<String>,
    pub voice: Option<String>,
    pub instruct: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Auto,
    Cpu,
    Metal,
    Cuda,
}

impl FromStr for DeviceKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "metal" => Ok(Self::Metal),
            "cuda" => Ok(Self::Cuda),
            other => bail!("unknown device '{other}'. Choose: auto, cpu, metal, cuda"),
        }
    }
}

impl DeviceKind {
    fn to_selection(self) -> DeviceSelection {
        match self {
            Self::Auto => DeviceSelection::Auto,
            Self::Cpu => DeviceSelection::Cpu,
            Self::Metal => DeviceSelection::Metal(0),
            Self::Cuda => DeviceSelection::Cuda(0),
        }
    }
}

pub struct Engine {
    model: Box<dyn TtsModel>,
    language: Option<String>,
    voice: Option<String>,
    instruct: Option<String>,
}

impl Engine {
    pub fn load(opts: &TtsOptions) -> Result<Self> {
        let model_type = opts.backend.model_type()?;
        let mut config = TtsConfig::new(model_type)
            .with_device(opts.device.to_selection())
            .with_preferred_runtime();

        // Preferred runtime may override device; re-apply explicit non-auto choice.
        if opts.device != DeviceKind::Auto {
            config = config.with_device(opts.device.to_selection());
        }

        if let Some(path) = &opts.model_path {
            config = config.with_model_path(path.to_string_lossy());
        }

        tracing::info!(
            backend = opts.backend.as_str(),
            model_path = ?opts.model_path,
            "loading TTS model (first run may download weights from Hugging Face)"
        );

        let model = load_model(config).context("failed to load any-tts model")?;
        let info = model.model_info();
        tracing::info!(
            name = %info.name,
            sample_rate = info.sample_rate,
            "model ready"
        );

        Ok(Self {
            model,
            language: opts.language.clone(),
            voice: opts.voice.clone(),
            instruct: opts.instruct.clone(),
        })
    }

    pub fn synthesize_chunk(&self, text: &str) -> Result<AudioSamples> {
        let mut request = SynthesisRequest::new(text);
        if let Some(lang) = &self.language {
            request = request.with_language(lang);
        }
        if let Some(voice) = &self.voice {
            request = request.with_voice(voice);
        }
        if let Some(instruct) = &self.instruct {
            request = request.with_instruct(instruct);
        }

        self.model
            .synthesize(&request)
            .with_context(|| format!("synthesis failed for text: {}", truncate(text, 80)))
    }

    /// Synthesize multiple chunks and concatenate into one audio stream.
    pub fn synthesize_chunks(&self, chunks: &[String]) -> Result<AudioSamples> {
        if chunks.is_empty() {
            bail!("no text to synthesize");
        }

        let mut combined: Option<AudioSamples> = None;
        let total = chunks.len();

        for (i, chunk) in chunks.iter().enumerate() {
            tracing::info!(chunk = i + 1, total, chars = chunk.chars().count(), "synthesizing");
            let audio = self.synthesize_chunk(chunk)?;
            combined = Some(match combined {
                None => audio,
                Some(prev) => concat_audio(prev, audio)?,
            });
        }

        combined.context("synthesis produced no audio")
    }
}

fn concat_audio(mut a: AudioSamples, b: AudioSamples) -> Result<AudioSamples> {
    if a.sample_rate != b.sample_rate {
        bail!(
            "sample rate mismatch when concatenating audio: {} vs {}",
            a.sample_rate,
            b.sample_rate
        );
    }
    // ~250ms pause between chunks
    let silence_samples = (a.sample_rate as f32 * 0.25) as usize;
    a.samples.extend(std::iter::repeat_n(0.0f32, silence_samples));
    a.samples.extend(b.samples);
    Ok(a)
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}
