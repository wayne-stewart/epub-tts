//! Local audio playback via rodio.

use anyhow::{Context, Result};
use any_tts::AudioSamples;
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;
use std::thread;
use std::time::Duration;

/// Play synthesized audio and block until finished.
pub fn play_blocking(audio: &AudioSamples) -> Result<()> {
    let wav = audio.get_wav();
    let cursor = Cursor::new(wav);

    let (_stream, handle) =
        OutputStream::try_default().context("failed to open default audio output device")?;
    let sink = Sink::try_new(&handle).context("failed to create audio sink")?;

    let source = Decoder::new(cursor).context("failed to decode synthesized WAV for playback")?;
    sink.append(source);

    // Wait until playback completes.
    while !sink.empty() {
        thread::sleep(Duration::from_millis(50));
    }
    sink.sleep_until_end();
    Ok(())
}
