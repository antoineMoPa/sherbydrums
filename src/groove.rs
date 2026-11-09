//! Typed groove and instrument mappings. Patterns use sixteen subdivisions of 4/4.

pub const STEPS: usize = 16;
pub const INSTRUMENTS: usize = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Rock,
    Blues,
    Funk,
    Pop,
    Samba,
    HalfTime,
    BoomBap,
    Garage,
    Breakbeat,
    DeepHouse,
    Bossa,
    TripHop,
}

impl Style {
    pub const ALL: [Self; 12] = [
        Self::Rock,
        Self::Blues,
        Self::Funk,
        Self::Pop,
        Self::Samba,
        Self::HalfTime,
        Self::BoomBap,
        Self::Garage,
        Self::Breakbeat,
        Self::DeepHouse,
        Self::Bossa,
        Self::TripHop,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Rock => 0,
            Self::Blues => 1,
            Self::Funk => 2,
            Self::Pop => 3,
            Self::Samba => 4,
            Self::HalfTime => 5,
            Self::BoomBap => 6,
            Self::Garage => 7,
            Self::Breakbeat => 8,
            Self::DeepHouse => 9,
            Self::Bossa => 10,
            Self::TripHop => 11,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Rock => "Pocket rock",
            Self::Blues => "Blues shuffle",
            Self::Funk => "Loose funk",
            Self::Pop => "Four on the floor",
            Self::Samba => "Electronic samba",
            Self::HalfTime => "Half time",
            Self::BoomBap => "Boom bap",
            Self::Garage => "UK garage",
            Self::Breakbeat => "Breakbeat",
            Self::DeepHouse => "Deep house",
            Self::Bossa => "Electronic bossa",
            Self::TripHop => "Trip hop",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Rock => "A steady backbeat. Plug in and settle into the pocket.",
            Self::Blues => "Swung eighths and a rim backbeat for a twelve-bar blues.",
            Self::Funk => "Syncopated kick, sixteenth hats, and snare ghost notes.",
            Self::Pop => "Four kicks to the bar with bright offbeat open hats.",
            Self::Samba => "Syncopated electronic percussion over a steady samba pulse.",
            Self::HalfTime => "A spacious beat with the snare landing on beat three.",
            Self::BoomBap => "A heavy backbeat with soft hat accents and snare ghosts.",
            Self::Garage => "A skipping two-step kick with lilting sixteenth hat accents.",
            Self::Breakbeat => "A broken kick pattern with busy snare ghosts and open-hat lifts.",
            Self::DeepHouse => "A steady house pulse with soft hats and syncopated rim clicks.",
            Self::Bossa => {
                "A bossa-inspired electronic remix: soft kick pulse, syncopated rim and offbeat hats."
            }
            Self::TripHop => "A sparse downtempo break with a heavy snare and rim accents.",
        }
    }

    pub const fn suggested_bpm(self) -> u16 {
        match self {
            Self::Rock => 100,
            Self::Blues => 88,
            Self::Funk => 106,
            Self::Pop => 120,
            Self::Samba => 112,
            Self::HalfTime => 80,
            Self::BoomBap => 90,
            Self::Garage => 130,
            Self::Breakbeat => 126,
            Self::DeepHouse => 120,
            Self::Bossa => 108,
            Self::TripHop => 82,
        }
    }

    /// Extra swing for shuffle styles; the slider adds sixteenth-note swing separately.
    pub const fn eighth_swing(self) -> f64 {
        match self {
            Self::Blues => 1.0 / 3.0,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instrument {
    Kick,
    Snare,
    ClosedHat,
    OpenHat,
    Rim,
    Tom,
    Crash,
}

impl Instrument {
    pub const ALL: [Self; INSTRUMENTS] = [
        Self::Kick,
        Self::Snare,
        Self::ClosedHat,
        Self::OpenHat,
        Self::Rim,
        Self::Tom,
        Self::Crash,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Kick => 0,
            Self::Snare => 1,
            Self::ClosedHat => 2,
            Self::OpenHat => 3,
            Self::Rim => 4,
            Self::Tom => 5,
            Self::Crash => 6,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Kick => "KICK",
            Self::Snare => "SNARE",
            Self::ClosedHat => "HI-HAT",
            Self::OpenHat => "OPEN HAT",
            Self::Rim => "RIM",
            Self::Tom => "TOM",
            Self::Crash => "CRASH",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub hits: [[u8; STEPS]; INSTRUMENTS],
}

impl Pattern {
    pub const fn velocity(&self, instrument: Instrument, step: usize) -> u8 {
        self.hits[instrument.index()][step]
    }
}

const fn pattern(
    kick: [u8; 16],
    snare: [u8; 16],
    hat: [u8; 16],
    open: [u8; 16],
    rim: [u8; 16],
) -> Pattern {
    Pattern {
        hits: [kick, snare, hat, open, rim, [0; 16], [0; 16]],
    }
}

const EMPTY: [u8; 16] = [0; 16];

pub const BANK: [Pattern; 12] = [
    pattern(
        [110, 0, 0, 0, 0, 0, 88, 0, 105, 0, 0, 0, 0, 0, 76, 0],
        [0, 0, 0, 0, 112, 0, 0, 0, 0, 0, 0, 0, 118, 0, 0, 0],
        [82, 0, 58, 0, 76, 0, 55, 0, 82, 0, 58, 0, 78, 0, 55, 0],
        EMPTY,
        EMPTY,
    ),
    pattern(
        [105, 0, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0, 0, 0, 75, 0],
        EMPTY,
        [82, 0, 58, 0, 75, 0, 58, 0, 82, 0, 58, 0, 75, 0, 58, 0],
        EMPTY,
        [0, 0, 0, 0, 110, 0, 0, 0, 0, 0, 0, 0, 115, 0, 0, 0],
    ),
    pattern(
        [115, 0, 0, 68, 0, 0, 96, 0, 0, 0, 110, 0, 0, 72, 0, 0],
        [0, 0, 0, 30, 115, 0, 0, 0, 0, 32, 0, 0, 118, 0, 0, 35],
        [
            83, 40, 58, 40, 78, 38, 64, 40, 83, 40, 58, 38, 80, 40, 0, 40,
        ],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 74, 0],
        EMPTY,
    ),
    pattern(
        [106, 0, 0, 0, 106, 0, 0, 0, 106, 0, 0, 0, 106, 0, 0, 0],
        [0, 0, 0, 0, 110, 0, 0, 0, 0, 0, 0, 0, 115, 0, 0, 0],
        [65, 0, 0, 0, 62, 0, 0, 0, 65, 0, 0, 0, 62, 0, 0, 0],
        [0, 0, 80, 0, 0, 0, 75, 0, 0, 0, 80, 0, 0, 0, 75, 0],
        EMPTY,
    ),
    pattern(
        [108, 0, 0, 65, 94, 0, 0, 0, 110, 0, 0, 68, 96, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 70, 0, 0, 0, 0, 0, 0, 62, 0, 0],
        [
            74, 38, 58, 40, 70, 36, 60, 42, 76, 38, 57, 40, 70, 36, 0, 42,
        ],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 65, 0],
        [90, 0, 0, 76, 0, 0, 88, 0, 0, 82, 0, 0, 86, 0, 0, 74],
    ),
    pattern(
        [115, 0, 0, 0, 0, 0, 82, 0, 0, 0, 0, 0, 0, 0, 92, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 120, 0, 0, 0, 0, 0, 0, 30],
        [78, 0, 50, 0, 65, 0, 50, 0, 78, 0, 50, 0, 65, 0, 50, 0],
        EMPTY,
        EMPTY,
    ),
    // Boom bap: backbeat anchors, quiet ghosts, and a late kick pickup.
    pattern(
        [112, 0, 0, 0, 0, 0, 80, 0, 104, 0, 0, 72, 0, 0, 0, 65],
        [0, 0, 0, 22, 116, 0, 0, 0, 0, 0, 28, 0, 120, 0, 0, 30],
        [76, 0, 47, 0, 70, 0, 50, 28, 78, 0, 45, 0, 72, 0, 48, 24],
        EMPTY,
        EMPTY,
    ),
    // Garage: two-step kick placement; velocity gives the hats their lilt.
    pattern(
        [110, 0, 0, 0, 0, 0, 75, 0, 0, 0, 104, 0, 0, 0, 0, 80],
        [0, 0, 0, 0, 112, 0, 0, 25, 0, 0, 0, 0, 116, 0, 0, 30],
        [65, 25, 0, 46, 62, 24, 54, 0, 66, 22, 0, 48, 60, 24, 50, 0],
        [0, 0, 68, 0, 0, 0, 0, 0, 0, 0, 70, 0, 0, 0, 0, 62],
        [0, 0, 0, 0, 0, 0, 0, 48, 0, 0, 0, 0, 0, 0, 0, 52],
    ),
    pattern(
        [114, 0, 78, 0, 0, 0, 0, 86, 108, 0, 0, 0, 0, 0, 90, 0],
        [0, 0, 0, 28, 116, 0, 0, 38, 0, 32, 0, 0, 118, 0, 42, 30],
        [78, 32, 52, 28, 72, 30, 0, 34, 80, 32, 55, 28, 74, 30, 0, 36],
        [0, 0, 0, 0, 0, 0, 76, 0, 0, 0, 0, 0, 0, 0, 80, 0],
        EMPTY,
    ),
    pattern(
        [102, 0, 0, 0, 102, 0, 0, 0, 102, 0, 0, 0, 102, 0, 0, 0],
        [0, 0, 0, 0, 82, 0, 0, 0, 0, 0, 0, 0, 88, 0, 0, 0],
        [48, 0, 0, 25, 45, 0, 0, 28, 50, 0, 0, 25, 46, 0, 0, 30],
        [0, 0, 62, 0, 0, 0, 58, 0, 0, 0, 64, 0, 0, 0, 60, 0],
        [0, 0, 0, 50, 0, 0, 0, 0, 0, 48, 0, 0, 0, 0, 0, 54],
    ),
    // Bossa remix: restrained two-beat bass pulse with anticipations and rim syncopation.
    pattern(
        [94, 0, 0, 58, 0, 0, 64, 0, 96, 0, 0, 56, 0, 0, 68, 0],
        [0, 0, 0, 0, 0, 0, 0, 22, 0, 0, 0, 0, 0, 0, 0, 26],
        [58, 22, 42, 20, 55, 22, 0, 24, 60, 20, 44, 22, 54, 20, 0, 24],
        [0, 0, 0, 0, 0, 0, 48, 0, 0, 0, 0, 0, 0, 0, 52, 0],
        [78, 0, 0, 0, 0, 0, 70, 0, 0, 0, 74, 0, 0, 80, 0, 0],
    ),
    pattern(
        [110, 0, 0, 0, 0, 0, 0, 64, 0, 0, 96, 0, 0, 0, 0, 72],
        [0, 0, 0, 0, 105, 0, 0, 0, 0, 0, 0, 26, 112, 0, 0, 0],
        [62, 0, 38, 0, 56, 0, 34, 22, 60, 0, 40, 0, 55, 0, 0, 20],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 48, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 42, 0, 0, 0, 0, 0, 0],
    ),
];

pub const fn groove(style: Style) -> &'static Pattern {
    &BANK[style.index()]
}

/// Fill replaces the second half of one bar, keeping the downbeat grounded.
pub fn fill_velocity(instrument: Instrument, step: usize) -> u8 {
    match (instrument, step) {
        (Instrument::Snare, 8 | 10) => 105,
        (Instrument::Snare, 9 | 11) => 65,
        (Instrument::Tom, 12 | 14) => 112,
        (Instrument::Tom, 13 | 15) => 85,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_bank_has_distinct_playable_patterns_and_aligned_bossa() {
        assert_eq!(Style::Bossa.index(), Style::Samba.index() + 6);
        for (index, style) in Style::ALL.into_iter().enumerate() {
            assert_eq!(style.index(), index);
            let pattern = groove(style);
            assert!(pattern.hits.iter().flatten().any(|v| *v > 0));
            assert!(pattern.hits.iter().flatten().all(|v| *v <= 127));
            for previous in &BANK[..index] {
                assert_ne!(pattern, previous);
            }
        }
    }
}
