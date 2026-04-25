use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use rubato::{Resampler, FixedAsync, PolynomialDegree};

// ---------------------------------------------------------------------------
// WAV loading — returns interleaved stereo f32 at target_rate
// ---------------------------------------------------------------------------

fn load_wav(bytes: &[u8], target_rate: u32) -> Vec<f32> {
    let reader = hound::WavReader::new(Cursor::new(bytes))
        .expect("Failed to parse WAV");
    let spec = reader.spec();
    let src_channels = spec.channels as usize;

    let raw: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.expect("Bad WAV sample") as f32 / i16::MAX as f32)
        .collect();

    // Ensure interleaved stereo
    let stereo: Vec<f32> = match src_channels {
        1 => raw.iter().flat_map(|&s| [s, s]).collect(),
        2 => raw,
        _ => raw.chunks(src_channels)
            .flat_map(|frame| [frame[0], frame.get(1).copied().unwrap_or(frame[0])])
            .collect(),
    };

    if spec.sample_rate == target_rate {
        return stereo;
    }

    // Resample each channel independently, then re-interleave
    let left: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    let right: Vec<f32> = stereo.iter().skip(1).step_by(2).copied().collect();

    let ratio = target_rate as f64 / spec.sample_rate as f64;
    let left_r = resample_channel(&left, ratio);
    let right_r = resample_channel(&right, ratio);

    let len = left_r.len().min(right_r.len());
    let mut result = Vec::with_capacity(len * 2);
    for i in 0..len {
        result.push(left_r[i]);
        result.push(right_r[i]);
    }

    log::info!(
        "Resampled SFX: {} → {} Hz ({} → {} frames)",
        spec.sample_rate, target_rate, stereo.len() / 2, len
    );
    result
}

fn resample_channel(samples: &[f32], ratio: f64) -> Vec<f32> {
    let chunk_size = 1024;
    let mut resampler = rubato::Async::<f32>::new_poly(
        ratio, 1.1, PolynomialDegree::Cubic, chunk_size, 1, FixedAsync::Input,
    ).expect("Failed to create resampler");

    let max_out = resampler.output_frames_max();
    let mut output = Vec::with_capacity((samples.len() as f64 * ratio) as usize + max_out);
    let mut out_buf = vec![0.0f32; max_out];

    for chunk in samples.chunks(chunk_size) {
        let input = if chunk.len() < chunk_size {
            let mut padded = chunk.to_vec();
            padded.resize(chunk_size, 0.0);
            padded
        } else {
            chunk.to_vec()
        };

        use rubato::audioadapter_buffers::direct::InterleavedSlice;
        let in_adapter = InterleavedSlice::new(&input, 1, chunk_size).unwrap();
        let mut out_adapter = InterleavedSlice::new_mut(&mut out_buf, 1, max_out).unwrap();
        let (_, frames_out) = resampler.process_into_buffer(
            &in_adapter, &mut out_adapter, None,
        ).unwrap();
        output.extend_from_slice(&out_buf[..frames_out]);
    }
    output
}

macro_rules! include_sfx {
    ($target_rate:expr, $( $name:expr => $path:expr ),+ $(,)?) => {{
        let mut map = HashMap::new();
        $(
            let samples = load_wav(include_bytes!($path), $target_rate);
            map.insert($name.to_string(), Arc::new(samples));
        )+
        map
    }};
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

struct PlayCmd {
    samples: Arc<Vec<f32>>, // interleaved stereo
    volume: f32,
}

struct ActiveVoice {
    samples: Arc<Vec<f32>>, // interleaved stereo
    position: usize,        // index into interleaved array (advances by 2 per frame)
    volume: f32,
}

// ---------------------------------------------------------------------------
// SfxPlayer — Send + Sync, stored as Tauri managed state
// ---------------------------------------------------------------------------

pub struct SfxPlayer {
    library: HashMap<String, Arc<Vec<f32>>>,
    play_tx: std::sync::mpsc::Sender<PlayCmd>,
}

impl SfxPlayer {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host.default_output_device().expect("No default audio output device");
        let supported = device.default_output_config().expect("No default output config");
        let sample_format = supported.sample_format();
        let channels = supported.channels() as usize;
        let device_rate = supported.sample_rate();
        log::info!("[sfx] Output: {} Hz, {} ch, {:?}", device_rate, channels, sample_format);

        // POTENTIAL FIX: buffer underrun/overrun on Arch (PipeWire/PulseAudio).
        // The default buffer size can be very small on some systems, causing
        // constant underruns.  1024 frames ≈ 21ms at 48kHz — imperceptible for
        // SFX but should give the audio thread enough headroom.  Untested on
        // the affected machine.
        let buffer_size = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                cpal::BufferSize::Fixed(1024.clamp(*min, *max))
            }
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
        };
        let config = cpal::StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size,
        };

        let library = include_sfx!(device_rate,
            "mute"              => "../../static/sfx/mute.wav",
            "unmute"            => "../../static/sfx/unmute.wav",
            "deafen"            => "../../static/sfx/deafen.wav",
            "undeafen"          => "../../static/sfx/undeafen.wav",
            "server_join"       => "../../static/sfx/server_join.wav",
            "server_disconnect" => "../../static/sfx/server_disconnect.wav",
            "user_join"         => "../../static/sfx/user_join.wav",
            "user_leave"        => "../../static/sfx/user_leave.wav",
            "new_notif"         => "../../static/sfx/new_notif.wav",
        );

        let (play_tx, play_rx) = std::sync::mpsc::channel::<PlayCmd>();

        // Mix directly in the cpal callback — no ring buffer, no timing issues
        let stream = build_output_stream(&device, &config, sample_format, channels, play_rx);
        stream.play().expect("Failed to start SFX output stream");

        // Leak the stream so it lives for the entire app lifetime
        // (SfxPlayer can't hold it because cpal::Stream isn't Sync)
        Box::leak(Box::new(stream));

        Self { library, play_tx }
    }

    pub fn play(&self, name: &str, volume: f32) {
        if let Some(samples) = self.library.get(name) {
            let _ = self.play_tx.send(PlayCmd {
                samples: Arc::clone(samples),
                volume: volume.clamp(0.0, 1.0),
            });
        } else {
            log::warn!("[sfx] Unknown sound: {name}");
        }
    }
}

// ---------------------------------------------------------------------------
// cpal stream — mixing happens directly in the audio callback
// ---------------------------------------------------------------------------

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: SampleFormat,
    channels: usize,
    play_rx: std::sync::mpsc::Receiver<PlayCmd>,
) -> cpal::Stream {
    // Mixing is done inside the callback: drain new play commands, sum active
    // voices, write stereo to device channels.  This avoids ring-buffer timing
    // issues entirely — the callback fires at exactly the device sample rate.
    macro_rules! build {
        ($t:ty, $from_f32:expr) => {{
            let from_f32: fn(f32) -> $t = $from_f32;
            let mut voices: Vec<ActiveVoice> = Vec::new();

            device.build_output_stream(
                config,
                move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                    // Pick up any new play commands
                    while let Ok(cmd) = play_rx.try_recv() {
                        voices.push(ActiveVoice {
                            samples: cmd.samples,
                            position: 0,
                            volume: cmd.volume,
                        });
                    }

                    if voices.is_empty() {
                        // Fast path: silence
                        for s in data.iter_mut() { *s = from_f32(0.0); }
                        return;
                    }

                    for frame in data.chunks_mut(channels) {
                        let mut left = 0.0f32;
                        let mut right = 0.0f32;

                        for voice in voices.iter_mut() {
                            if voice.position + 1 < voice.samples.len() {
                                left  += voice.samples[voice.position]     * voice.volume;
                                right += voice.samples[voice.position + 1] * voice.volume;
                                voice.position += 2;
                            }
                        }

                        left = left.clamp(-1.0, 1.0);
                        right = right.clamp(-1.0, 1.0);

                        // Map stereo SFX to device channel layout
                        if channels == 1 {
                            frame[0] = from_f32((left + right) * 0.5);
                        } else {
                            frame[0] = from_f32(left);
                            frame[1] = from_f32(right);
                            for ch in frame.iter_mut().skip(2) { *ch = from_f32(0.0); }
                        }
                    }

                    voices.retain(|v| v.position + 1 < v.samples.len());
                },
                |err| log::error!("[sfx] Audio output error: {}", err),
                None,
            ).expect("Failed to build SFX output stream")
        }};
    }

    match sample_format {
        SampleFormat::F32 => build!(f32, |s| s),
        SampleFormat::I16 => build!(i16, |s: f32|
            (s * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16),
        SampleFormat::I32 => build!(i32, |s: f32|
            (s * i32::MAX as f32).clamp(i32::MIN as f32, i32::MAX as f32) as i32),
        SampleFormat::U16 => build!(u16, |s: f32|
            ((s + 1.0) * i16::MAX as f32).clamp(0.0, u16::MAX as f32) as u16),
        fmt => panic!("Unsupported SFX output sample format: {fmt:?}"),
    }
}
