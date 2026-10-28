//! Lock-free parameters shared between the UI thread and the audio threads,
//! plus the serializable `Config` that is saved to disk.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

pub const BANDS: usize = 5;
pub const CHANNEL_NAMES: [&str; 4] = ["Game", "Chat", "Media", "Aux"];
/// Index of the Chat channel (it gets the optional incoming-voice AI cleanup).
pub const CHAT: usize = 1;

pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    #[inline]
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed)
    }
}

pub struct ChannelParams {
    pub volume: AtomicF32,
    pub muted: AtomicBool,
    pub eq: [AtomicF32; BANDS],
    pub peak: AtomicF32,
}

impl ChannelParams {
    fn new(volume: f32) -> Self {
        Self {
            volume: AtomicF32::new(volume),
            muted: AtomicBool::new(false),
            eq: std::array::from_fn(|_| AtomicF32::new(0.0)),
            peak: AtomicF32::new(0.0),
        }
    }
    pub fn eq_gains(&self) -> [f32; BANDS] {
        std::array::from_fn(|i| self.eq[i].get())
    }
}

pub struct VoiceParams {
    /// 0.0 = off, 1.0 = full AI noise removal.
    pub denoise: AtomicF32,
    /// Voice-activity gate threshold (0 = gate off).
    pub gate: AtomicF32,
    pub gain_db: AtomicF32,
    pub comp: AtomicBool,
    pub muted: AtomicBool,
    pub eq: [AtomicF32; BANDS],
    pub peak: AtomicF32,
}

impl VoiceParams {
    fn new(denoise: f32) -> Self {
        Self {
            denoise: AtomicF32::new(denoise),
            gate: AtomicF32::new(0.0),
            gain_db: AtomicF32::new(0.0),
            comp: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            eq: std::array::from_fn(|_| AtomicF32::new(0.0)),
            peak: AtomicF32::new(0.0),
        }
    }
    pub fn eq_gains(&self) -> [f32; BANDS] {
        std::array::from_fn(|i| self.eq[i].get())
    }
}

pub struct Shared {
    pub master: ChannelParams,
    /// Game, Chat, Media, Aux.
    pub channels: [ChannelParams; 4],
    pub mic: VoiceParams,
    /// AI cleanup applied to incoming voice on the Chat channel.
    pub chat_voice: VoiceParams,
    /// 0.0 = full Game focus, 0.5 = balanced, 1.0 = full Chat focus.
    pub chatmix: AtomicF32,
    pub night_mode: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            master: ChannelParams::new(0.8),
            channels: std::array::from_fn(|_| ChannelParams::new(0.8)),
            mic: VoiceParams::new(0.9),
            chat_voice: VoiceParams::new(0.0),
            chatmix: AtomicF32::new(0.5),
            night_mode: AtomicBool::new(false),
        }
    }

    pub fn apply(&self, c: &Config) {
        apply_chan(&self.master, &c.master);
        for i in 0..4 {
            apply_chan(&self.channels[i], &c.channels[i]);
        }
        let m = &c.mic;
        self.mic.denoise.set(m.denoise);
        self.mic.gate.set(m.gate);
        self.mic.gain_db.set(m.gain_db);
        self.mic.comp.store(m.comp, Relaxed);
        self.mic.muted.store(m.muted, Relaxed);
        for (a, g) in self.mic.eq.iter().zip(m.eq) {
            a.set(g);
        }
        self.chat_voice.denoise.set(c.chat_denoise);
        self.chatmix.set(c.chatmix);
        self.night_mode.store(c.night_mode, Relaxed);
    }
}

fn apply_chan(p: &ChannelParams, c: &ChanCfg) {
    p.volume.set(c.volume);
    p.muted.store(c.muted, Relaxed);
    for (a, g) in p.eq.iter().zip(c.eq) {
        a.set(g);
    }
}

// ---------------------------------------------------------------- persisted config

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ChanCfg {
    pub volume: f32,
    pub muted: bool,
    pub eq: [f32; BANDS],
    pub preset: String,
}

impl Default for ChanCfg {
    fn default() -> Self {
        Self { volume: 0.8, muted: false, eq: [0.0; BANDS], preset: "Flat".into() }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct VoiceCfg {
    pub denoise: f32,
    pub gate: f32,
    pub gain_db: f32,
    pub comp: bool,
    pub muted: bool,
    pub eq: [f32; BANDS],
    pub preset: String,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            denoise: 0.9,
            gate: 0.0,
            gain_db: 0.0,
            comp: false,
            muted: false,
            eq: [0.0; BANDS],
            preset: "Flat".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Devices {
    pub master_out: Option<String>,
    pub mic_in: Option<String>,
    pub mic_out: Option<String>,
    /// Game, Chat, Media, Aux source devices (virtual cables / loopback).
    pub sources: [Option<String>; 4],
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ThemeCfg {
    pub dark: bool,
    pub accent: [u8; 3],
}

impl Default for ThemeCfg {
    fn default() -> Self {
        Self { dark: true, accent: [61, 165, 255] }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub theme: ThemeCfg,
    pub master: ChanCfg,
    pub channels: [ChanCfg; 4],
    pub mic: VoiceCfg,
    pub chat_denoise: f32,
    pub chatmix: f32,
    pub night_mode: bool,
    pub autostart: bool,
    pub devices: Devices,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeCfg::default(),
            master: ChanCfg::default(),
            channels: std::array::from_fn(|_| ChanCfg::default()),
            mic: VoiceCfg::default(),
            chat_denoise: 0.0,
            chatmix: 0.5,
            night_mode: false,
            autostart: true,
            devices: Devices::default(),
        }
    }
}

impl Config {
    fn path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("dev", "sonar-lite", "sonar-lite")
            .map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    /// Copy live values from the audio parameters (keeps preset names and devices).
    pub fn capture_from(&mut self, s: &Shared) {
        capture_chan(&mut self.master, &s.master);
        for i in 0..4 {
            capture_chan(&mut self.channels[i], &s.channels[i]);
        }
        self.mic.denoise = s.mic.denoise.get();
        self.mic.gate = s.mic.gate.get();
        self.mic.gain_db = s.mic.gain_db.get();
        self.mic.comp = s.mic.comp.load(Relaxed);
        self.mic.muted = s.mic.muted.load(Relaxed);
        self.mic.eq = s.mic.eq_gains();
        self.chat_denoise = s.chat_voice.denoise.get();
        self.chatmix = s.chatmix.get();
        self.night_mode = s.night_mode.load(Relaxed);
    }
}

fn capture_chan(c: &mut ChanCfg, p: &ChannelParams) {
    c.volume = p.volume.get();
    c.muted = p.muted.load(Relaxed);
    c.eq = p.eq_gains();
}
