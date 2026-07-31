//! epub-tts — narrate EPUB books from the command line with any-tts.

mod audio_fx;
mod book;
mod play;
mod text;
mod tts_engine;

use crate::audio_fx::change_speed;
use crate::book::Book;
use crate::text::chunk_for_tts;
use crate::tts_engine::{Backend, DeviceKind, Engine, TtsOptions};
use anyhow::{Context, Result, bail};
use any_tts::AudioSamples;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "epub-tts",
    version,
    about = "CLI EPUB reader powered by any-tts",
    long_about = "Open an EPUB, list chapters, extract text, and synthesize speech with local any-tts models (Qwen3-TTS by default for best quality)."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Increase log verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show EPUB metadata
    Info {
        /// Path to the .epub file
        epub: PathBuf,
    },

    /// List spine chapters (reading order)
    Chapters {
        /// Path to the .epub file
        epub: PathBuf,
    },

    /// Print plain text for one or more chapters (no TTS)
    Text {
        /// Path to the .epub file
        epub: PathBuf,

        /// Chapter index (0-based). Repeat or use with --to for a range.
        #[arg(short, long)]
        chapter: Option<usize>,

        /// Inclusive end chapter index when reading a range
        #[arg(long)]
        to: Option<usize>,
    },

    /// Synthesize speech for one or more chapters
    Read {
        /// Path to the .epub file
        epub: PathBuf,

        /// Chapter index (0-based). Defaults to 0.
        #[arg(short, long, default_value_t = 0)]
        chapter: usize,

        /// Inclusive end chapter index (synthesize a range)
        #[arg(long)]
        to: Option<usize>,

        /// Write WAV output to this path (directory for multi-chapter)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Skip playback (useful with --output only)
        #[arg(long)]
        no_play: bool,

        /// TTS backend / model family (qwen3 = best quality default)
        #[arg(long, default_value = "qwen3", value_parser = parse_backend)]
        backend: Backend,

        /// Local model directory (skips Hugging Face download when complete)
        #[arg(long)]
        model_path: Option<PathBuf>,

        /// Compute device: auto, cpu, metal, cuda
        #[arg(long, default_value = "auto", value_parser = parse_device)]
        device: DeviceKind,

        /// Language (e.g. en / English, de / German). Defaults from EPUB metadata.
        #[arg(long)]
        language: Option<String>,

        /// Named speaker (Qwen3 CustomVoice: Ryan, Vivian, …)
        #[arg(long)]
        voice: Option<String>,

        /// Style instruction for Qwen3 / OmniVoice (e.g. "Clear, natural audiobook narration.")
        #[arg(long)]
        instruct: Option<String>,

        /// Max characters per internal TTS piece (long chapters are split under the hood)
        #[arg(long, default_value_t = 400)]
        chunk_chars: usize,

        /// Playback/synthesis speed multiplier (1.0 = normal, 1.5 = 50% faster)
        #[arg(long, default_value_t = 1.0)]
        speed: f32,
    },
}

fn parse_backend(s: &str) -> Result<Backend, String> {
    s.parse().map_err(|e: anyhow::Error| e.to_string())
}

fn parse_device(s: &str) -> Result<DeviceKind, String> {
    s.parse().map_err(|e: anyhow::Error| e.to_string())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Commands::Info { epub } => cmd_info(epub),
        Commands::Chapters { epub } => cmd_chapters(epub),
        Commands::Text { epub, chapter, to } => cmd_text(epub, chapter, to),
        Commands::Read {
            epub,
            chapter,
            to,
            output,
            no_play,
            backend,
            model_path,
            device,
            language,
            voice,
            instruct,
            chunk_chars,
            speed,
        } => cmd_read(ReadArgs {
            epub,
            chapter,
            to,
            output,
            play: !no_play,
            backend,
            model_path,
            device,
            language,
            voice,
            instruct,
            chunk_chars,
            speed,
        }),
    }
}

fn init_tracing(verbose: u8) {
    // Quiet by default so synthesis only shows the progress bar.
    // Use -v / -vv (or RUST_LOG=…) for diagnostics.
    let level = match verbose {
        0 => "error",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}

fn cmd_info(path: PathBuf) -> Result<()> {
    let book = Book::open(&path)?;
    let meta = book.meta();

    println!("File:       {}", book.path().display());
    println!(
        "Title:      {}",
        meta.title.as_deref().unwrap_or("(unknown)")
    );
    if !meta.creators.is_empty() {
        println!("Author(s):  {}", meta.creators.join(", "));
    }
    if let Some(lang) = &meta.language {
        println!("Language:   {lang}");
    }
    if let Some(pub_) = &meta.publisher {
        println!("Publisher:  {pub_}");
    }
    if let Some(id) = &meta.identifier {
        println!("Identifier: {id}");
    }
    println!("Chapters:   {}", meta.chapter_count);
    if let Some(desc) = &meta.description {
        let short = if desc.chars().count() > 280 {
            format!("{}…", desc.chars().take(280).collect::<String>())
        } else {
            desc.clone()
        };
        println!("Description:\n  {short}");
    }
    Ok(())
}

fn cmd_chapters(path: PathBuf) -> Result<()> {
    let book = Book::open(&path)?;
    let chapters = book.chapters();
    if chapters.is_empty() {
        println!("(no spine chapters)");
        return Ok(());
    }

    println!("{:>5}  {:<24}  {:<36}  {}", "INDEX", "ID", "PATH", "TITLE");
    for ch in chapters {
        let title = ch.title.as_deref().unwrap_or("-");
        let path = ch
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "{:>5}  {:<24}  {:<36}  {title}",
            ch.index,
            truncate(&ch.id, 24),
            truncate(&path, 36)
        );
    }
    Ok(())
}

fn cmd_text(path: PathBuf, chapter: Option<usize>, to: Option<usize>) -> Result<()> {
    let mut book = Book::open(&path)?;
    let (start, end) = chapter_range(chapter.unwrap_or(0), to, book.meta().chapter_count)?;

    for idx in start..=end {
        let text = book.chapter_text(idx)?;
        if start != end {
            println!("===== Chapter {idx} =====\n");
        }
        if text.is_empty() {
            println!("(empty chapter)\n");
        } else {
            println!("{text}\n");
        }
    }
    Ok(())
}

struct ReadArgs {
    epub: PathBuf,
    chapter: usize,
    to: Option<usize>,
    output: Option<PathBuf>,
    play: bool,
    backend: Backend,
    model_path: Option<PathBuf>,
    device: DeviceKind,
    language: Option<String>,
    voice: Option<String>,
    instruct: Option<String>,
    chunk_chars: usize,
    speed: f32,
}

fn cmd_read(args: ReadArgs) -> Result<()> {
    if !args.play && args.output.is_none() {
        bail!("nothing to do: enable playback (default) or pass --output");
    }
    if !args.speed.is_finite() || args.speed <= 0.0 {
        bail!("--speed must be a positive number (got {})", args.speed);
    }

    let mut book = Book::open(&args.epub)?;
    let meta = book.meta();
    let (start, end) = chapter_range(args.chapter, args.to, meta.chapter_count)?;

    // Default language from EPUB metadata when not provided.
    let language = args
        .language
        .or_else(|| meta.language.clone())
        .or_else(|| Some("en".into()));

    // Clear reading style helps Qwen3 pronunciation/pacing for long-form text.
    let instruct = args.instruct.or_else(|| {
        if args.backend == Backend::Qwen3Tts {
            Some("Clear, natural audiobook narration with accurate pronunciation.".into())
        } else {
            None
        }
    });

    let engine = Engine::load(&TtsOptions {
        backend: args.backend,
        model_path: args.model_path,
        device: args.device,
        language,
        voice: args.voice,
        instruct,
    })?;

    let player = if args.play {
        Some(play::Player::new()?)
    } else {
        None
    };

    let multi = start != end;
    let chapters = book.chapters();

    for idx in start..=end {
        let title = chapters
            .iter()
            .find(|c| c.index == idx)
            .and_then(|c| c.title.clone())
            .unwrap_or_else(|| format!("chapter-{idx}"));

        tracing::debug!(chapter = idx, %title, "synthesizing full chapter");

        let text = book.chapter_text(idx)?;
        if text.trim().is_empty() {
            tracing::debug!(chapter = idx, "skipping empty chapter");
            continue;
        }

        let pieces = chunk_for_tts(&text, args.chunk_chars);
        let label = format!("ch{idx} {}", truncate_msg(&title, 36));
        let mut audio = synthesize_chapter_with_progress(&engine, &pieces, &label)?;

        if (args.speed - 1.0).abs() >= 1e-3 {
            audio = change_speed(&audio, args.speed);
            tracing::debug!(
                speed = args.speed,
                duration_s = audio.duration_secs(),
                "applied speed"
            );
        }

        if let Some(out_path) = &args.output {
            let path = if multi || out_path.is_dir() {
                std::fs::create_dir_all(out_path)
                    .with_context(|| format!("create output dir {}", out_path.display()))?;
                out_path.join(format!("{:04}.wav", idx))
            } else {
                if let Some(parent) = out_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                out_path.clone()
            };
            audio
                .save_wav(&path)
                .with_context(|| format!("write {}", path.display()))?;
            // One quiet confirmation line after the progress bar clears.
            eprintln!("wrote {}", path.display());
        }

        if let Some(player) = &player {
            player.play_blocking(&audio)?;
        }
    }

    Ok(())
}

/// Synthesize every piece of a chapter, showing a progress bar until complete.
fn synthesize_chapter_with_progress(
    engine: &Engine,
    pieces: &[String],
    label: &str,
) -> Result<AudioSamples> {
    if pieces.is_empty() {
        bail!("no text to synthesize");
    }

    let bar = ProgressBar::new(pieces.len() as u64);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}",
        )
        .expect("valid progress template")
        .progress_chars("=>-"),
    );
    bar.set_message(label.to_string());
    bar.enable_steady_tick(Duration::from_millis(100));

    let mut combined: Option<AudioSamples> = None;
    for piece in pieces {
        let audio = engine.synthesize_chunk(piece).map_err(|err| {
            bar.abandon_with_message("failed");
            err
        })?;
        combined = Some(match combined {
            None => audio,
            Some(prev) => concat_audio(prev, audio)?,
        });
        bar.inc(1);
    }

    bar.finish_and_clear();
    combined.context("synthesis produced no audio")
}

fn concat_audio(mut a: AudioSamples, b: AudioSamples) -> Result<AudioSamples> {
    if a.sample_rate != b.sample_rate {
        bail!(
            "sample rate mismatch when concatenating audio: {} vs {}",
            a.sample_rate,
            b.sample_rate
        );
    }
    // Brief pause between internal pieces so joins sound natural.
    let silence_samples = (a.sample_rate as f32 * 0.15) as usize;
    a.samples
        .extend(std::iter::repeat_n(0.0f32, silence_samples));
    a.samples.extend(b.samples);
    Ok(a)
}

fn truncate_msg(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn chapter_range(start: usize, to: Option<usize>, total: usize) -> Result<(usize, usize)> {
    if total == 0 {
        bail!("EPUB has no spine chapters");
    }
    let end = to.unwrap_or(start);
    if start >= total {
        bail!("chapter {start} out of range (0..{total})");
    }
    if end >= total {
        bail!("chapter {end} out of range (0..{total})");
    }
    if end < start {
        bail!("--to ({end}) must be >= --chapter ({start})");
    }
    Ok((start, end))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}
