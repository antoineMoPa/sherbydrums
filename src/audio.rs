use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use sherbydrum::{
    engine::{Command, Engine, Position},
    groove::Style,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{Receiver, SyncSender, sync_channel},
};

#[derive(Default)]
pub struct Telemetry {
    position: AtomicU64,
    recording_step: AtomicU64,
    pub failed: AtomicBool,
    pub panic_keys: AtomicBool,
}

impl Telemetry {
    pub fn recording_step(&self) -> Option<usize> {
        let value = self.recording_step.load(Ordering::Relaxed);
        (value > 0).then(|| (value - 1) as usize)
    }

    fn publish(&self, position: Position) {
        let bits = u64::from(position.running)
            | ((position.step as u64) << 2)
            | ((position.style.index() as u64) << 6)
            | (u64::from(position.fill_pending) << 10)
            | (u64::from(position.filling) << 11)
            | ((position.bar & ((1_u64 << 36) - 1)) << 12)
            | (u64::from(position.custom_id) << 48);
        self.position.store(bits, Ordering::Relaxed);
    }

    pub fn position(&self) -> Position {
        let bits = self.position.load(Ordering::Relaxed);
        Position {
            custom_id: (bits >> 48) as u16,
            running: bits & 1 != 0,
            step: ((bits >> 2) & 15) as usize,
            style: Style::ALL[((bits >> 6) & 15) as usize],
            fill_pending: bits & (1 << 10) != 0,
            filling: bits & (1 << 11) != 0,
            bar: (bits >> 12) & ((1_u64 << 36) - 1),
        }
    }
}

pub struct Output {
    _stream: cpal::Stream,
    sender: SyncSender<Command>,
    pub telemetry: Arc<Telemetry>,
    pub description: String,
}

impl Output {
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No output device found. Connect speakers or headphones, then retry.")?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("Cannot read audio configuration: {error}"))?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let description = format!(
            "{} · {} Hz · {} ch",
            device.name().unwrap_or_else(|_| "Default output".into()),
            config.sample_rate.0,
            config.channels
        );
        let telemetry = Arc::new(Telemetry::default());
        let (sender, receiver) = sync_channel(512);
        let engine = Engine::new(config.sample_rate.0);
        telemetry.publish(engine.position());
        let shared = Arc::clone(&telemetry);
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, engine, receiver, shared),
            SampleFormat::F64 => build::<f64>(&device, &config, engine, receiver, shared),
            SampleFormat::I8 => build::<i8>(&device, &config, engine, receiver, shared),
            SampleFormat::I16 => build::<i16>(&device, &config, engine, receiver, shared),
            SampleFormat::I32 => build::<i32>(&device, &config, engine, receiver, shared),
            SampleFormat::I64 => build::<i64>(&device, &config, engine, receiver, shared),
            SampleFormat::U8 => build::<u8>(&device, &config, engine, receiver, shared),
            SampleFormat::U16 => build::<u16>(&device, &config, engine, receiver, shared),
            SampleFormat::U32 => build::<u32>(&device, &config, engine, receiver, shared),
            SampleFormat::U64 => build::<u64>(&device, &config, engine, receiver, shared),
            _ => return Err(format!("Unsupported output sample format: {format}")),
        }
        .map_err(|error| format!("Cannot open output: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("Cannot start output: {error}"))?;
        Ok(Self {
            _stream: stream,
            sender,
            telemetry,
            description,
        })
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.sender.try_send(command).map_err(|error| {
            // Losing a note-off must never leave a key sounding indefinitely.
            self.telemetry.panic_keys.store(true, Ordering::Release);
            format!("Audio command was not accepted: {error}")
        })
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
    receiver: Receiver<Command>,
    telemetry: Arc<Telemetry>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels);
    let errors = Arc::clone(&telemetry);
    device.build_output_stream(
        config,
        move |buffer: &mut [T], _| {
            for command in receiver.try_iter().take(512) {
                engine.command(command);
            }
            if telemetry.panic_keys.swap(false, Ordering::AcqRel) {
                engine.command(Command::PanicKeys);
            }
            for frame in buffer.chunks_mut(channels) {
                let sample = engine.next_sample();
                frame.fill(T::from_sample(sample));
            }
            telemetry.publish(engine.position());
            telemetry.recording_step.store(
                engine.recording_step().map_or(0, |step| step as u64 + 1),
                Ordering::Relaxed,
            );
        },
        move |_| {
            errors.failed.store(true, Ordering::Relaxed);
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_preserves_custom_identity_and_transport_bits() {
        let telemetry = Telemetry::default();
        let expected = Position {
            custom_id: u16::MAX,
            running: true,
            step: 15,
            bar: 123_456,
            style: Style::Blues,
            fill_pending: true,
            filling: true,
        };
        // Exercise the fourth style bit independently of fill flags and bar bits.
        for style in Style::ALL {
            for flags in 0..16 {
                let expected = Position {
                    style,
                    bar: (1_u64 << 36) - 1,
                    running: flags & 1 != 0,
                    fill_pending: flags & 4 != 0,
                    filling: flags & 8 != 0,
                    ..expected
                };
                telemetry.publish(expected);
                assert_eq!(telemetry.position(), expected);
            }
        }
    }
}
