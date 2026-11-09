//! Headless MIDI input monitor. Connections live until the process exits.
use midir::{Ignore, MidiInput};
use std::{
    error::Error,
    io::{self, Write},
    sync::mpsc,
};

pub fn run() -> Result<(), Box<dyn Error>> {
    let input = MidiInput::new("Sherbydrum MIDI debug")?;
    let ports = input.ports();
    if ports.is_empty() {
        return Err("No MIDI inputs found. Connect a MIDI device and run midi-debug again.".into());
    }

    let (sender, receiver) = mpsc::channel();
    let mut connections = Vec::new();
    for (index, port) in ports.iter().enumerate() {
        let name = input
            .port_name(port)
            .unwrap_or_else(|_| format!("Input {}", index + 1));
        let mut input = MidiInput::new("Sherbydrum MIDI debug")?;
        input.ignore(Ignore::None);
        let sender = sender.clone();
        match input.connect(
            port,
            "sherbydrum-midi-debug",
            move |timestamp, bytes, _| {
                let _ = sender.send((index, timestamp, bytes.to_vec()));
            },
            (),
        ) {
            Ok(connection) => {
                eprintln!("Listening to input {}: {name}", index + 1);
                connections.push(connection);
            }
            Err(error) => eprintln!("Cannot open input {} ({name}): {error}", index + 1),
        }
    }
    drop(sender);
    if connections.is_empty() {
        return Err("Could not open any MIDI inputs.".into());
    }
    eprintln!("MIDI debug: Ctrl+C to stop. Restart to discover newly connected inputs.");
    let mut stdout = io::stdout().lock();
    for (index, timestamp, bytes) in receiver {
        if let Err(error) = writeln!(
            stdout,
            "input={} timestamp_us={} {} bytes={:02X?}",
            index + 1,
            timestamp,
            message_label(&bytes),
            bytes
        )
        .and_then(|()| stdout.flush())
        {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(error.into());
        }
    }
    Ok(())
}

fn message_label(bytes: &[u8]) -> String {
    let Some(&status) = bytes.first() else {
        return "Empty message".into();
    };
    let channel = (status & 0x0f) + 1;
    match (status & 0xf0, bytes) {
        (0x80, [_, note, velocity]) => {
            format!("Note off ch={channel} note={note} velocity={velocity}")
        }
        (0x90, [_, note, 0]) => format!("Note off ch={channel} note={note} velocity=0 (note-on)"),
        (0x90, [_, note, velocity]) => {
            format!("Note on ch={channel} note={note} velocity={velocity}")
        }
        (0xa0, [_, note, pressure]) => {
            format!("Poly pressure ch={channel} note={note} pressure={pressure}")
        }
        (0xb0, [_, control, value]) => {
            format!("Control change ch={channel} control={control} value={value}")
        }
        (0xc0, [_, program]) => format!("Program change ch={channel} program={program}"),
        (0xd0, [_, pressure]) => format!("Channel pressure ch={channel} pressure={pressure}"),
        (0xe0, [_, low, high]) => format!(
            "Pitch bend ch={channel} value={}",
            u16::from(*low) | (u16::from(*high) << 7)
        ),
        _ => match status {
            0xf0 => "SysEx",
            0xf1 => "Time code",
            0xf2 => "Song position",
            0xf3 => "Song select",
            0xf6 => "Tune request",
            0xf7 => "End of SysEx",
            0xf8 => "Clock",
            0xfa => "Start",
            0xfb => "Continue",
            0xfc => "Stop",
            0xfe => "Active sensing",
            0xff => "Reset",
            _ => "Unknown or incomplete message",
        }
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_channel_messages_without_changing_raw_data() {
        assert_eq!(
            message_label(&[0x99, 38, 100]),
            "Note on ch=10 note=38 velocity=100"
        );
        assert_eq!(
            message_label(&[0x99, 38, 0]),
            "Note off ch=10 note=38 velocity=0 (note-on)"
        );
        assert_eq!(message_label(&[0xef, 0, 64]), "Pitch bend ch=16 value=8192");
    }

    #[test]
    fn handles_system_and_short_messages() {
        assert_eq!(message_label(&[0xf8]), "Clock");
        assert_eq!(message_label(&[0xf0, 0x7d, 1, 0xf7]), "SysEx");
        assert_eq!(message_label(&[0x90, 60]), "Unknown or incomplete message");
        assert_eq!(message_label(&[]), "Empty message");
    }
}
