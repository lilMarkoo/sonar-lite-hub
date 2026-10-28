//! EQ presets. Bands: 60 Hz low shelf, 250 Hz, 1 kHz, 4 kHz peaking, 12 kHz high shelf. Gains in dB.

pub struct Preset {
    pub name: &'static str,
    pub gains: [f32; 5],
}

pub const BAND_LABELS: [&str; 5] = ["60", "250", "1k", "4k", "12k"];

/// Presets for Master and the Game / Chat / Media / Aux channels.
pub const OUTPUT: &[Preset] = &[
    Preset { name: "Flat", gains: [0.0; 5] },
    Preset { name: "Bass Boost", gains: [7.0, 4.0, 0.0, 0.0, 0.0] },
    Preset { name: "Treble Boost", gains: [0.0, 0.0, 0.0, 3.0, 6.0] },
    Preset { name: "Vocal Boost", gains: [-3.0, -1.0, 3.0, 4.0, 1.0] },
    Preset { name: "Footsteps (FPS)", gains: [-6.0, -3.0, 1.0, 5.0, 3.0] },
    Preset { name: "Cinema", gains: [6.0, 2.0, -1.0, 2.0, 3.0] },
    Preset { name: "Music (V-shape)", gains: [5.0, 1.0, -3.0, 1.0, 5.0] },
    Preset { name: "Late Night", gains: [-5.0, -2.0, 0.0, 1.0, -2.0] },
];

/// Presets for the microphone chain.
pub const MIC: &[Preset] = &[
    Preset { name: "Flat", gains: [0.0; 5] },
    Preset { name: "Clear Voice", gains: [-6.0, -2.0, 1.0, 3.0, 2.0] },
    Preset { name: "Warm Broadcast", gains: [3.0, 3.0, 0.0, 1.0, -1.0] },
    Preset { name: "Reduce Boominess", gains: [-8.0, -4.0, 0.0, 0.0, 0.0] },
    Preset { name: "Presence", gains: [-4.0, -1.0, 2.0, 4.0, 3.0] },
];
