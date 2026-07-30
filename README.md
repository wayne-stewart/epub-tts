# epub-tts

CLI EPUB reader that narrates books with [any-tts](https://crates.io/crates/any-tts) — local, open-weight speech synthesis via Candle.

## Features

- Open EPUB 2/3 files and inspect metadata
- List spine chapters (reading order) with TOC titles when available
- Dump plain text from chapters (HTML stripped)
- Synthesize speech with **Kokoro-82M** by default (small, fast, Apache-2.0)
- Optional backends: OmniVoice, Qwen3-TTS, VibeVoice, Voxtral
- Write WAV files and/or play through the system audio device
- Chunks long chapters at paragraph/sentence boundaries for stable TTS

## Install / build

```bash
# macOS (Metal GPU) — default features
cargo build --release

# CPU only
cargo build --release --no-default-features

# Extra model families (larger compile + downloads)
cargo build --release --features qwen3-tts,omnivoice,vibevoice
```

Binary: `target/release/epub-tts`

## Usage

```bash
# Metadata
epub-tts info book.epub

# Chapter list (0-based indices)
epub-tts chapters book.epub

# Plain text dump
epub-tts text book.epub -c 3
epub-tts text book.epub -c 0 --to 2

# Narrate a chapter → WAV
epub-tts read book.epub -c 1 -o chapter1.wav

# Play while synthesizing (and optionally save)
epub-tts read book.epub -c 1 --play -o chapter1.wav

# Range of chapters into a directory
epub-tts read book.epub -c 0 --to 4 -o ./audio/

# Voice / language / model path
epub-tts read book.epub -c 1 -o out.wav \
  --language en \
  --voice af_heart \
  --model-path ./models/Kokoro-82M \
  --device metal
```

### `read` options

| Flag | Description |
|------|-------------|
| `-c, --chapter N` | Spine index (default `0`) |
| `--to N` | Inclusive end chapter for a range |
| `-o, --output PATH` | WAV file, or directory when synthesizing multiple chapters |
| `-p, --play` | Play through the default audio device |
| `--backend` | `kokoro` (default), `omnivoice`, `qwen3`, `vibevoice`, … |
| `--model-path` | Local model directory (skips HF download when complete) |
| `--device` | `auto`, `cpu`, `metal`, `cuda` |
| `--language` | e.g. `en`, `de`, `ja` (falls back to EPUB metadata) |
| `--voice` | Named/preset voice (model-dependent) |
| `--instruct` | Style instruction (OmniVoice / Qwen3) |
| `--chunk-chars` | Max characters per synthesis chunk (default `400`) |

At least one of `--output` or `--play` is required for `read`.

## Models

First run of a backend downloads weights from Hugging Face (needs network). Kokoro is the lightest default:

| Backend | Cargo feature | Notes |
|---------|---------------|--------|
| Kokoro | always on | Best default for local CLI use |
| OmniVoice | `omnivoice` | Multilingual + instruct |
| Qwen3-TTS | `qwen3-tts` | Strong control / named speakers |
| VibeVoice | `vibevoice` | Long-form / reference audio |
| Voxtral | `voxtral` | Larger; **CC BY-NC** weights |

Model weight licenses are separate from this tool’s MIT license — check upstream terms before redistribution.

## Logging

```bash
RUST_LOG=debug epub-tts read book.epub -c 0 -o out.wav
# or
epub-tts -v read book.epub -c 0 -o out.wav
```

## License

MIT
