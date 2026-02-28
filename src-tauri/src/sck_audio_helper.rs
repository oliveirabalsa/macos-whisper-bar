use std::{
    io::{self, BufRead, BufWriter, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context};
use screencapturekit::prelude::*;
use screencapturekit::AudioBufferList;

const OUTPUT_SAMPLE_RATE: f32 = 16_000.0;
const OUTPUT_GAIN: f32 = 1.25;

pub fn run() -> anyhow::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    spawn_stdin_stop_watcher(stop.clone());

    let content = SCShareableContent::get().context("failed to read ScreenCaptureKit content")?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no displays available for ScreenCaptureKit capture"))?;

    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();

    let config = SCStreamConfiguration::new()
        .with_width(display.width() as u32)
        .with_height(display.height() as u32)
        .with_captures_audio(true)
        .with_sample_rate(48_000)
        .with_channel_count(2);

    let handler = AudioOutputHandler::new(stop.clone());
    let mut stream = SCStream::new(&filter, &config);
    stream.add_output_handler(handler, SCStreamOutputType::Audio);
    stream
        .start_capture()
        .context("failed to start ScreenCaptureKit capture")?;

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(120));
    }

    stream
        .stop_capture()
        .context("failed to stop ScreenCaptureKit capture")?;
    Ok(())
}

fn spawn_stdin_stop_watcher(stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        while !stop.load(Ordering::Relaxed) {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
                Ok(_) => {
                    if line.trim().eq_ignore_ascii_case("stop") {
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                Err(_) => {
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });
}

struct AudioOutputHandler {
    writer: Arc<Mutex<BufWriter<io::Stdout>>>,
    stop: Arc<AtomicBool>,
}

impl AudioOutputHandler {
    fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            writer: Arc::new(Mutex::new(BufWriter::new(io::stdout()))),
            stop,
        }
    }
}

impl SCStreamOutputTrait for AudioOutputHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, output_type: SCStreamOutputType) {
        if self.stop.load(Ordering::Relaxed) || output_type != SCStreamOutputType::Audio {
            return;
        }

        let Some(format) = sample.format_description() else {
            return;
        };
        let source_rate = format.audio_sample_rate().unwrap_or(48_000.0).max(1.0) as f32;
        let is_float = format.audio_is_float();

        let Some(buffers) = sample.audio_buffer_list() else {
            return;
        };

        let mono = mix_to_mono(&buffers, is_float);
        if mono.is_empty() {
            return;
        }

        let resampled = resample_to_output_rate(&mono, source_rate);
        if resampled.is_empty() {
            return;
        }

        let pcm = float_to_pcm_bytes(&resampled);
        if pcm.is_empty() {
            return;
        }

        if let Ok(mut writer) = self.writer.lock() {
            if writer.write_all(&pcm).is_err() || writer.flush().is_err() {
                self.stop.store(true, Ordering::Relaxed);
            }
        } else {
            self.stop.store(true, Ordering::Relaxed);
        }
    }
}

fn mix_to_mono(buffers: &AudioBufferList, is_float: bool) -> Vec<f32> {
    let mut per_buffer: Vec<Vec<f32>> = Vec::new();
    for buffer in buffers {
        let channel_count = (buffer.number_channels.max(1)) as usize;
        let bytes = buffer.data();
        let decoded = if is_float {
            decode_f32_mono(bytes, channel_count)
        } else {
            decode_i16_mono(bytes, channel_count)
        };
        if !decoded.is_empty() {
            per_buffer.push(decoded);
        }
    }

    if per_buffer.is_empty() {
        return Vec::new();
    }

    if per_buffer.len() == 1 {
        return per_buffer.remove(0);
    }

    let Some(min_len) = per_buffer.iter().map(Vec::len).min() else {
        return Vec::new();
    };
    if min_len == 0 {
        return Vec::new();
    }

    let mut mixed = vec![0.0_f32; min_len];
    for stream in &per_buffer {
        for (index, sample) in stream.iter().take(min_len).enumerate() {
            mixed[index] += *sample;
        }
    }

    let divisor = per_buffer.len() as f32;
    for sample in &mut mixed {
        *sample /= divisor;
    }

    mixed
}

fn decode_f32_mono(bytes: &[u8], channel_count: usize) -> Vec<f32> {
    let mut out = Vec::new();
    if channel_count == 0 {
        return out;
    }

    if channel_count == 1 {
        out.reserve(bytes.len() / 4);
        for chunk in bytes.chunks_exact(4) {
            out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        return out;
    }

    let mut frame_acc = 0.0_f32;
    let mut channel_index = 0usize;
    for chunk in bytes.chunks_exact(4) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        frame_acc += value;
        channel_index += 1;
        if channel_index == channel_count {
            out.push(frame_acc / channel_count as f32);
            channel_index = 0;
            frame_acc = 0.0;
        }
    }

    out
}

fn decode_i16_mono(bytes: &[u8], channel_count: usize) -> Vec<f32> {
    let mut out = Vec::new();
    if channel_count == 0 {
        return out;
    }

    if channel_count == 1 {
        out.reserve(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            out.push(sample as f32 / 32768.0);
        }
        return out;
    }

    let mut frame_acc = 0.0_f32;
    let mut channel_index = 0usize;
    for chunk in bytes.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
        frame_acc += sample;
        channel_index += 1;
        if channel_index == channel_count {
            out.push(frame_acc / channel_count as f32);
            channel_index = 0;
            frame_acc = 0.0;
        }
    }

    out
}

fn resample_to_output_rate(input: &[f32], source_rate: f32) -> Vec<f32> {
    if input.is_empty() || source_rate <= 0.0 {
        return Vec::new();
    }

    if (source_rate - OUTPUT_SAMPLE_RATE).abs() < 1.0 {
        return input.to_vec();
    }

    let ratio = source_rate / OUTPUT_SAMPLE_RATE;
    let output_len = ((input.len() as f32) / ratio).floor().max(0.0) as usize;
    if output_len == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let source_pos = index as f32 * ratio;
        let base_index = source_pos.floor() as usize;
        if base_index >= input.len() {
            break;
        }
        let next_index = (base_index + 1).min(input.len().saturating_sub(1));
        let fraction = source_pos - base_index as f32;
        let value = input[base_index] * (1.0 - fraction) + input[next_index] * fraction;
        out.push(value);
    }

    out
}

fn float_to_pcm_bytes(input: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(input.len() * 2);
    for sample in input {
        let amplified = (sample * OUTPUT_GAIN).clamp(-1.0, 1.0);
        let pcm = (amplified * i16::MAX as f32).round() as i16;
        bytes.extend_from_slice(&pcm.to_le_bytes());
    }
    bytes
}
