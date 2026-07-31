# epub-tts

CLI EPUB reader that narrates books with [any-tts](https://crates.io/crates/any-tts) — local, open-weight speech synthesis via Candle.

## Features

- Open EPUB 2/3 files and inspect metadata
- List spine chapters (reading order) with TOC titles when available
- Dump plain text from chapters (HTML stripped)
- Synthesize speech with **Qwen3-TTS** by default (best quality / pronunciation)
- Optional backends: Kokoro (fast/small), OmniVoice, VibeVoice, Voxtral
- Write WAV files and/or play through the system audio device
- Full-chapter synthesis with a progress bar, then play / save

## Install / build

```bash
# macOS (Metal GPU + Qwen3) — default features
cargo build --release

# CPU only (still Qwen3; slower)
cargo build --release --no-default-features --features qwen3-tts

# Lightweight Kokoro instead of / in addition to Qwen3
cargo build --release --features kokoro
```

Binary: `target/release/epub-tts`

**Note:** Qwen3-TTS is a ~1.7B model plus speech-tokenizer weights. First run downloads several GB from Hugging Face. Prefer Metal/CUDA when available.

## Usage

```bash
# Metadata
epub-tts info book.epub

# Chapter list (0-based indices)
epub-tts chapters book.epub

# Plain text dump
epub-tts text book.epub -c 3

# Synthesize a full chapter (progress bar), then play
epub-tts read book.epub -c 1

# Pick a speaker and language explicitly
epub-tts read book.epub -c 1 \
  --language English \
  --voice vivian \
  --device metal

# Save as WAV or MP3 (format follows the file extension)
epub-tts read book.epub -c 1 -o chapter1.wav
epub-tts read book.epub -c 1 -o chapter1.mp3 --no-play

# Fast/local Kokoro (if built with --features kokoro)
epub-tts read book.epub -c 1 --backend kokoro
```

### `read` options

| Flag | Description |
|------|-------------|
| `-c, --chapter N` | Spine index (default `0`) |
| `--to N` | Inclusive end chapter for a range |
| `-o, --output PATH` | Optional `.wav` / `.mp3` file (or directory for multi-chapter) |
| `--no-play` | Print + synthesize only; do not open speakers |
| `--backend` | `qwen3` (default), `kokoro`, `omnivoice`, `vibevoice`, … |
| `--model-path` | Local model directory (skips HF download when complete) |
| `--device` | `auto`, `cpu`, `metal`, `cuda` |
| `--language` | e.g. `en` / `English` (falls back to EPUB metadata) |
| `--voice` | Named speaker (Qwen3 default: `vivian`) |
| `--instruct` | Style instruction (default audiobook-style for Qwen3) |
| `--chunk-chars` | Max characters per synthesis piece (default `800`) |
| `--speed` | Playback speed multiplier (default `1.0`) |

By default, `read` synthesizes the entire chapter (with a progress bar), then plays it.

## Models

| Backend | Cargo feature | Default? | Notes |
|---------|---------------|----------|--------|
| **Qwen3-TTS** | `qwen3-tts` | **yes** | Best quality / pronunciation; named speakers |
| Kokoro | `kokoro` | no | Small & fast; weaker English pronunciation |
| OmniVoice | `omnivoice` | no | Multilingual + instruct / voice design |
| VibeVoice | `vibevoice` | no | Long-form / reference audio |
| Voxtral | `voxtral` | no | Larger; **CC BY-NC** weights |

Model weight licenses are separate from this tool’s MIT license — check upstream terms before redistribution.

## Logging

```bash
RUST_LOG=debug epub-tts read book.epub -c 0 -o out.wav
# or
epub-tts -v read book.epub -c 0 -o out.wav
```

## License

MIT
