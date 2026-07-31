//! Local audio playback via rodio.

use anyhow::{Context, Result};
use any_tts::AudioSamples;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::thread;
use std::time::Duration;

/// Holds the default audio device open so consecutive sentences play without reopening.
pub struct Player {
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl Player {
    pub fn new() -> Result<Self> {
        let (stream, handle) =
            OutputStream::try_default().context("failed to open default audio output device")?;
        Ok(Self {
            _stream: stream,
            handle,
        })
    }

    /// Play synthesized audio and block until finished.
    pub fn play_blocking(&self, audio: &AudioSamples) -> Result<()> {
        let wav = audio.get_wav();
        let cursor = Cursor::new(wav);
        let sink = Sink::try_new(&self.handle).context("failed to create audio sink")?;
        let source =
            Decoder::new(cursor).context("failed to decode synthesized WAV for playback")?;
        sink.append(source);
        while !sink.empty() {
            thread::sleep(Duration::from_millis(20));
        }
        sink.sleep_until_end();
        Ok(())
    }
}

