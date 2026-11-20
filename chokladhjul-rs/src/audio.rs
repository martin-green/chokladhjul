//! Sound playback. Sounds are decoded once at startup; if there is no audio
//! device the game simply runs silently.

use std::io::Cursor;

use rodio::static_buffer::StaticSamplesBuffer;
use rodio::{ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Sample, SampleRate, Source};

struct Sound {
    channels: ChannelCount,
    rate: SampleRate,
    samples: &'static [Sample],
}

impl Sound {
    fn decode(bytes: &'static [u8]) -> Result<Self, String> {
        let decoder = Decoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let channels = decoder.channels();
        let rate = decoder.sample_rate();
        let samples: Vec<Sample> = decoder.collect();
        Ok(Sound { channels, rate, samples: Box::leak(samples.into_boxed_slice()) })
    }

    fn source(&self) -> StaticSamplesBuffer {
        StaticSamplesBuffer::new(self.channels, self.rate, self.samples)
    }
}

pub struct Audio {
    sink: Option<MixerDeviceSink>,
    tick: Option<Sound>,
    fanfare: Option<Sound>,
}

impl Audio {
    pub fn new(tick: &'static [u8], fanfare: &'static [u8]) -> Self {
        let sink = match DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                Some(sink)
            }
            Err(e) => {
                crate::warn(&format!("no audio output: {e}"));
                None
            }
        };
        let load = |name: &str, bytes| {
            Sound::decode(bytes).map_err(|e| crate::warn(&format!("could not decode {name}: {e}"))).ok()
        };
        Audio { tick: load("tick.mp3", tick), fanfare: load("fanfar.mp3", fanfare), sink }
    }

    pub fn silent() -> Self {
        Audio { sink: None, tick: None, fanfare: None }
    }

    fn play(&self, sound: &Option<Sound>, volume: f32) {
        if let (Some(sink), Some(sound)) = (&self.sink, sound) {
            sink.mixer().add(sound.source().amplify(volume));
        }
    }

    /// `strength` 0..1 from the peg impact speed.
    pub fn tick(&self, strength: f32) {
        self.play(&self.tick, 0.35 + 0.65 * strength.clamp(0.0, 1.0));
    }

    pub fn fanfare(&self) {
        self.play(&self.fanfare, 1.0);
    }
}
