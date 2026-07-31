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
        match self {
            #[cfg(feature = "kokoro")]
            Self::Kokoro => Ok(ModelType::Kokoro),
            #[cfg(not(feature = "kokoro"))]
            Self::Kokoro => bail!("backend 'kokoro' requires --features kokoro"),
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

    /// Normalize a language tag for this backend (ISO codes → model language names).
    pub fn normalize_language(self, lang: &str) -> String {
        let raw = lang.trim();
        let short = raw
            .split(['-', '_'])
            .next()
            .unwrap_or(raw)
            .to_ascii_lowercase();

        match self {
            // Qwen3 CustomVoice expects full language names in many checkpoints.
            Self::Qwen3Tts => match short.as_str() {
                "en" | "eng" | "english" => "English".into(),
                "zh" | "cmn" | "chinese" | "zho" => "Chinese".into(),
                "ja" | "jpn" | "japanese" => "Japanese".into(),
                "ko" | "kor" | "korean" => "Korean".into(),
                "de" | "deu" | "german" => "German".into(),
                "fr" | "fra" | "fre" | "french" => "French".into(),
                "ru" | "rus" | "russian" => "Russian".into(),
                "pt" | "por" | "portuguese" => "Portuguese".into(),
                "es" | "spa" | "spanish" => "Spanish".into(),
                "it" | "ita" | "italian" => "Italian".into(),
                "auto" => "auto".into(),
                _ if raw.chars().next().is_some_and(|c| c.is_uppercase()) => raw.to_string(),
                _ => raw.to_string(),
            },
            // Kokoro and most others prefer short ISO tags.
            _ => short,
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

        tracing::debug!(
            backend = opts.backend.as_str(),
            model_path = ?opts.model_path,
            "loading TTS model"
        );

        let model = load_model(config).context("failed to load any-tts model")?;
        let info = model.model_info();
        tracing::debug!(
            name = %info.name,
            sample_rate = info.sample_rate,
            voices = ?info.voices,
            "model ready"
        );

        let language = opts
            .language
            .as_deref()
            .map(|l| opts.backend.normalize_language(l));

        // Prefer an explicit voice; otherwise pick a sensible CustomVoice default for Qwen3.
        let voice = opts.voice.clone().or_else(|| {
            if opts.backend == Backend::Qwen3Tts {
                default_qwen3_voice(&info.voices, language.as_deref())
            } else {
                None
            }
        });

        if let Some(v) = &voice {
            tracing::debug!(voice = %v, language = ?language, "using voice");
        }

        Ok(Self {
            model,
            language,
            voice,
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

/// Pick a CustomVoice speaker when the user did not pass --voice.
fn default_qwen3_voice(voices: &[String], language: Option<&str>) -> Option<String> {
    if voices.is_empty() {
        // Preferred default on the public CustomVoice checkpoint.
        return Some("Vivian".into());
    }

    let preferred = match language {
        Some("English") | None => &["Vivian", "Ryan", "Aiden", "Serena"][..],
        Some("Chinese") => &["Vivian", "Serena", "Uncle_Fu", "Dylan"][..],
        Some("Japanese") => &["Ono_Anna", "Sohee"][..],
        Some("Korean") => &["Sohee", "Ono_Anna"][..],
        _ => &["Vivian", "Ryan", "Aiden"][..],
    };

    for name in preferred {
        if voices.iter().any(|v| v.eq_ignore_ascii_case(name)) {
            return Some((*name).to_string());
        }
    }

    voices.first().cloned()
}
