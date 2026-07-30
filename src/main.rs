//! epub-tts — narrate EPUB books from the command line with any-tts.

mod book;
mod play;
mod text;
mod tts_engine;

use crate::book::Book;
use crate::text::chunk_for_tts;
use crate::tts_engine::{Backend, DeviceKind, Engine, TtsOptions};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "epub-tts",
    version,
    about = "CLI EPUB reader powered by any-tts",
    long_about = "Open an EPUB, list chapters, extract text, and synthesize speech with local any-tts models (Kokoro by default)."
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

        /// Play audio through the default output device
        #[arg(short, long)]
        play: bool,

        /// TTS backend / model family
        #[arg(long, default_value = "kokoro", value_parser = parse_backend)]
        backend: Backend,

        /// Local model directory (skips Hugging Face download when complete)
        #[arg(long)]
        model_path: Option<PathBuf>,

        /// Compute device: auto, cpu, metal, cuda
        #[arg(long, default_value = "auto", value_parser = parse_device)]
        device: DeviceKind,

        /// Language tag (e.g. en, de, ja). Model-dependent.
        #[arg(long)]
        language: Option<String>,

        /// Named / preset voice (model-dependent; Kokoro uses voices/*.pt names)
        #[arg(long)]
        voice: Option<String>,

        /// Style instruction (OmniVoice / Qwen3 instruct mode)
        #[arg(long)]
        instruct: Option<String>,

        /// Max characters per TTS chunk
        #[arg(long, default_value_t = 400)]
        chunk_chars: usize,
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
            play,
            backend,
            model_path,
            device,
            language,
            voice,
            instruct,
            chunk_chars,
        } => cmd_read(ReadArgs {
            epub,
            chapter,
            to,
            output,
            play,
            backend,
            model_path,
            device,
            language,
            voice,
            instruct,
            chunk_chars,
        }),
    }
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
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
}

fn cmd_read(args: ReadArgs) -> Result<()> {
    if !args.play && args.output.is_none() {
        bail!("specify --output <path.wav> and/or --play");
    }

    let mut book = Book::open(&args.epub)?;
    let meta = book.meta();
    let (start, end) = chapter_range(args.chapter, args.to, meta.chapter_count)?;

    // Default language from EPUB metadata when not provided.
    let language = args.language.or_else(|| {
        meta.language.as_ref().map(|l| {
            // Prefer short ISO-ish tag for backends like Kokoro.
            l.split(['-', '_'])
                .next()
                .unwrap_or(l)
                .to_ascii_lowercase()
        })
    });

    let engine = Engine::load(&TtsOptions {
        backend: args.backend,
        model_path: args.model_path,
        device: args.device,
        language,
        voice: args.voice,
        instruct: args.instruct,
    })?;

    let multi = start != end;
    for idx in start..=end {
        let title = book
            .chapters()
            .into_iter()
            .find(|c| c.index == idx)
            .and_then(|c| c.title)
            .unwrap_or_else(|| format!("chapter-{idx}"));

        tracing::info!(chapter = idx, %title, "reading chapter");
        let text = book.chapter_text(idx)?;
        if text.trim().is_empty() {
            tracing::warn!(chapter = idx, "skipping empty chapter");
            continue;
        }

        let chunks = chunk_for_tts(&text, args.chunk_chars);
        tracing::info!(chapter = idx, chunks = chunks.len(), "chunked text");

        let audio = engine.synthesize_chunks(&chunks)?;
        tracing::info!(
            chapter = idx,
            duration_s = format!("{:.1}", audio.duration_secs()),
            "synthesis complete"
        );

        if let Some(out) = &args.output {
            let path = if multi || out.is_dir() {
                std::fs::create_dir_all(out)
                    .with_context(|| format!("create output dir {}", out.display()))?;
                out.join(format!("{:04}.wav", idx))
            } else {
                if let Some(parent) = out.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                out.clone()
            };
            audio
                .save_wav(&path)
                .with_context(|| format!("write {}", path.display()))?;
            println!("wrote {}", path.display());
        }

        if args.play {
            println!("playing chapter {idx}…");
            play::play_blocking(&audio)?;
        }
    }

    Ok(())
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
