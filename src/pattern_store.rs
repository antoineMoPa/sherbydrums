//! Versioned custom-pattern bank, written atomically when the editor saves.
use crate::groove::{INSTRUMENTS, Pattern, STEPS, Style};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPattern {
    pub name: String,
    pub style: Style,
    pub pattern: Pattern,
}

const MAGIC: &[u8; 8] = b"SHDRUM\0\x01";
// These positions are part of the file format; keep them stable.
const STORED_STYLES: [Style; 12] = [
    Style::Rock,
    Style::Blues,
    Style::Funk,
    Style::Pop,
    Style::Samba,
    Style::HalfTime,
    Style::BoomBap,
    Style::Garage,
    Style::Breakbeat,
    Style::DeepHouse,
    Style::Bossa,
    Style::TripHop,
];

pub fn bank_path() -> io::Result<PathBuf> {
    let env_path = |key| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let root = if cfg!(target_os = "macos") {
        env_path("HOME").map(|p| p.join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        env_path("APPDATA")
    } else {
        env_path("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env_path("HOME").map(|p| p.join(".local/share")))
    };
    root.map(|p| p.join("sherbydrum/patterns.bin"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "User data directory unavailable"))
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or unsupported pattern bank",
    )
}

pub fn load(path: &Path) -> io::Result<Vec<SavedPattern>> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    if &header != MAGIC {
        return Err(invalid());
    }
    let mut count = [0; 2];
    file.read_exact(&mut count)?;
    let mut bank = Vec::new();
    for _ in 0..u16::from_le_bytes(count) {
        let mut length = [0; 2];
        file.read_exact(&mut length)?;
        let mut name = vec![0; u16::from_le_bytes(length) as usize];
        file.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| invalid())?;
        let mut style = [0];
        file.read_exact(&mut style)?;
        let style = *STORED_STYLES.get(style[0] as usize).ok_or_else(invalid)?;
        let mut hits = [[0; STEPS]; INSTRUMENTS];
        for row in &mut hits {
            file.read_exact(row)?;
            if row.iter().any(|v| *v > 127) {
                return Err(invalid());
            }
        }
        bank.push(SavedPattern {
            name,
            style,
            pattern: Pattern { hits },
        });
    }
    if file.read(&mut [0])? != 0 {
        return Err(invalid());
    }
    Ok(bank)
}

pub fn save(path: &Path, bank: &[SavedPattern]) -> io::Result<()> {
    let count = u16::try_from(bank.len()).map_err(|_| invalid())?;
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.extend_from_slice(&count.to_le_bytes());
    for entry in bank {
        let length = u16::try_from(entry.name.len()).map_err(|_| invalid())?;
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(entry.name.as_bytes());
        bytes.push(
            STORED_STYLES
                .iter()
                .position(|s| *s == entry.style)
                .ok_or_else(invalid)? as u8,
        );
        for row in entry.pattern.hits {
            if row.iter().any(|v| *v > 127) {
                return Err(invalid());
            }
            bytes.extend_from_slice(&row);
        }
    }
    fs::create_dir_all(path.parent().ok_or_else(invalid)?)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "sherbydrum-store-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
        fn path(&self) -> PathBuf {
            self.0.join("patterns.bin")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn bank_survives_reopening_and_editing() {
        let dir = Directory::new();
        assert!(load(&dir.path()).unwrap().is_empty());
        let mut bank: Vec<_> = STORED_STYLES
            .iter()
            .map(|style| SavedPattern {
                name: format!("Étude 🥁 {style:?}"),
                style: *style,
                pattern: *crate::groove::groove(*style),
            })
            .collect();
        bank[0].pattern.hits[6][15] = 127;
        bank[0].pattern.hits[0][0] = 1;
        save(&dir.path(), &bank).unwrap();
        assert_eq!(load(&dir.path()).unwrap(), bank);
        bank[1].name = "Edited".into();
        bank[1].pattern.hits[1][3] = 42;
        save(&dir.path(), &bank).unwrap();
        assert_eq!(load(&dir.path()).unwrap(), bank);
    }
    #[test]
    fn rejects_corrupt_and_future_banks_without_modifying_them() {
        let dir = Directory::new();
        fs::create_dir_all(&dir.0).unwrap();
        for bytes in [
            b"SHDRUM\0\x02\0\0".as_slice(),
            b"SHDRUM\0\x01\x01\0",
            b"garbage",
        ] {
            fs::write(dir.path(), bytes).unwrap();
            assert!(load(&dir.path()).is_err());
            assert_eq!(fs::read(dir.path()).unwrap(), bytes);
        }
    }
    #[test]
    fn invalid_save_keeps_previous_bank() {
        let dir = Directory::new();
        save(&dir.path(), &[]).unwrap();
        let entry = SavedPattern {
            name: "bad".into(),
            style: Style::Rock,
            pattern: Pattern {
                hits: [[255; STEPS]; INSTRUMENTS],
            },
        };
        assert!(save(&dir.path(), &[entry]).is_err());
        assert!(load(&dir.path()).unwrap().is_empty());
    }
    #[test]
    fn write_failure_is_reported() {
        let dir = Directory::new();
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(dir.path(), b"occupied").unwrap();
        assert!(save(&dir.path().join("patterns.bin"), &[]).is_err());
        assert_eq!(fs::read(dir.path()).unwrap(), b"occupied");
    }
}
