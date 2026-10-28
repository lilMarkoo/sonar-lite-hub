#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dsp;
mod engine;
mod params;
mod presets;

use eframe::egui::{self, Color32};
use engine::{list_devices, DeviceLists, Engine};
use params::*;
use presets::{Preset, BAND_LABELS};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

const CHANNEL_COLORS: [Color32; 4] = [
    Color32::from_rgb(255, 122, 69),  // Game
    Color32::from_rgb(61, 165, 255),  // Chat
    Color32::from_rgb(93, 220, 138),  // Media
    Color32::from_rgb(193, 139, 255), // Aux
];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Mixer,
    Mic,
    Devices,
    Theme,
}

struct App {
    shared: Arc<Shared>,
    cfg: Config,
    engine: Option<Engine>,
    devices: DeviceLists,
    error: Option<String>,
    tab: Tab,
}

impl App {
    fn new() -> Self {
        let cfg = Config::load();
        let shared = Arc::new(Shared::new());
        shared.apply(&cfg);
        let mut app = Self {
            shared,
            cfg,
            engine: None,
            devices: list_devices(),
            error: None,
            tab: Tab::Mixer,
        };
        if app.cfg.autostart {
            app.start();
        }
        app
    }

    fn start(&mut self) {
        self.engine = None; // stop the old engine first so devices are released
        match Engine::start(&self.cfg.devices, self.shared.clone()) {
            Ok(e) => {
                self.engine = Some(e);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn stop(&mut self) {
        self.engine = None;
    }

    fn mixer_ui(&mut self, ui: &mut egui::Ui) {
        let sh = self.shared.clone();
        ui.horizontal_top(|ui| {
            strip(ui, "Master", &sh.master, &mut self.cfg.master.preset, presets::OUTPUT, Color32::WHITE, 100);
            ui.separator();
            for i in 0..4 {
                strip(
                    ui,
                    CHANNEL_NAMES[i],
                    &sh.channels[i],
                    &mut self.cfg.channels[i].preset,
                    presets::OUTPUT,
                    CHANNEL_COLORS[i],
                    i,
                );
                ui.separator();
            }
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label("Game");
            ui.spacing_mut().slider_width = 360.0;
            let mut cm = sh.chatmix.get();
            if ui.add(egui::Slider::new(&mut cm, 0.0..=1.0).show_value(false)).changed() {
                sh.chatmix.set(cm);
            }
            ui.label("Chat  (ChatMix)");
        });
        let mut night = sh.night_mode.load(Relaxed);
        if ui.checkbox(&mut night, "Night mode (compress loud sounds, keep quiet ones audible)").changed() {
            sh.night_mode.store(night, Relaxed);
        }
    }

    fn mic_ui(&mut self, ui: &mut egui::Ui) {
        let sh = self.shared.clone();
        ui.heading("Microphone");
        ui.add_space(6.0);
        slider_row(ui, "AI noise cancellation", &sh.mic.denoise, 0.0..=1.0, true);
        slider_row(ui, "Noise gate (voice activity)", &sh.mic.gate, 0.0..=0.9, false);
        ui.label(egui::RichText::new("   0 = gate off. Higher = stricter, only clear speech passes.").weak());
        slider_row(ui, "Mic gain (dB)", &sh.mic.gain_db, -12.0..=18.0, false);
        ui.horizontal(|ui| {
            let mut comp = sh.mic.comp.load(Relaxed);
            if ui.checkbox(&mut comp, "Compressor (even out loud/quiet speech)").changed() {
                sh.mic.comp.store(comp, Relaxed);
            }
            let mut muted = sh.mic.muted.load(Relaxed);
            if ui.toggle_value(&mut muted, "Mute mic").changed() {
                sh.mic.muted.store(muted, Relaxed);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Level");
            ui.add(egui::ProgressBar::new(level01(sh.mic.peak.get())).desired_width(320.0));
        });
        ui.add_space(8.0);
        ui.label("Mic EQ");
        preset_ui(ui, 200, &mut self.cfg.mic.preset, presets::MIC, &sh.mic.eq);

        ui.add_space(18.0);
        ui.separator();
        ui.heading("Incoming voice");
        slider_row(ui, "Clean up Chat channel (AI)", &sh.chat_voice.denoise, 0.0..=1.0, true);
        ui.label(egui::RichText::new("   Removes noise from teammates' voices coming through the Chat channel.").weak());
    }

    fn theme_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Theme");
        ui.add_space(6.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui.selectable_value(&mut self.cfg.theme.dark, true, "Dark").changed();
            changed |= ui.selectable_value(&mut self.cfg.theme.dark, false, "Light").changed();
        });
        ui.horizontal(|ui| {
            ui.label("Accent color");
            changed |= ui.color_edit_button_srgb(&mut self.cfg.theme.accent).changed();
        });
        ui.horizontal(|ui| {
            let swatches: [(&str, [u8; 3]); 5] = [
                ("Blue", [61, 165, 255]),
                ("Orange", [255, 122, 69]),
                ("Green", [93, 220, 138]),
                ("Purple", [193, 139, 255]),
                ("Red", [255, 92, 92]),
            ];
            for (name, rgb) in swatches {
                if ui.button(name).clicked() {
                    self.cfg.theme.accent = rgb;
                    changed = true;
                }
            }
        });
        if changed {
            apply_theme(ui.ctx(), &self.cfg.theme);
        }
    }

    fn devices_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Devices");
        ui.label(
            "Route each app to a virtual device (see README), then pick that device as the channel source. \
             Changes restart the audio engine.",
        );
        ui.add_space(8.0);

        let outputs = self.devices.outputs.clone();
        let inputs = self.devices.inputs.clone();
        let mut sources = inputs.clone();
        if cfg!(windows) {
            // Windows can capture any playback device via loopback.
            for o in &outputs {
                if !sources.contains(o) {
                    sources.push(o.clone());
                }
            }
        }

        let mut changed = false;
        changed |= device_combo(ui, "Headphones out", &mut self.cfg.devices.master_out, &outputs, "System default");
        ui.add_space(6.0);
        for i in 0..4 {
            changed |= device_combo(
                ui,
                &format!("{} source", CHANNEL_NAMES[i]),
                &mut self.cfg.devices.sources[i],
                &sources,
                "(none)",
            );
        }
        ui.add_space(6.0);
        changed |= device_combo(ui, "Microphone in", &mut self.cfg.devices.mic_in, &inputs, "(none)");
        changed |= device_combo(ui, "Virtual mic out", &mut self.cfg.devices.mic_out, &outputs, "(none)");

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh device list").clicked() {
                self.devices = list_devices();
            }
            ui.checkbox(&mut self.cfg.autostart, "Start audio on launch");
        });
        if changed && self.engine.is_some() {
            self.start();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(33)); // keeps meters moving

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Sonar Lite");
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Mixer, "Mixer");
                ui.selectable_value(&mut self.tab, Tab::Mic, "Microphone");
                ui.selectable_value(&mut self.tab, Tab::Devices, "Devices");
                ui.selectable_value(&mut self.tab, Tab::Theme, "Theme");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.engine.is_some() {
                        if ui.button("Stop").clicked() {
                            self.stop();
                        }
                    } else if ui.button("Start").clicked() {
                        self.start();
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Mixer => self.mixer_ui(ui),
            Tab::Mic => self.mic_ui(ui),
            Tab::Devices => self.devices_ui(ui),
            Tab::Theme => self.theme_ui(ui),
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cfg.capture_from(&self.shared);
        self.cfg.save();
    }
}

// ------------------------------------------------------------------ widgets

fn level01(peak: f32) -> f32 {
    ((dsp::lin_to_db(peak) + 60.0) / 60.0).clamp(0.0, 1.0) // -60 dB .. 0 dB
}

fn meter(ui: &mut egui::Ui, peak: f32, height: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_gray(40));
    let fill_top = rect.bottom() - rect.height() * level01(peak);
    let fill = egui::Rect::from_min_max(egui::pos2(rect.left(), fill_top), rect.right_bottom());
    painter.rect_filled(fill, 2.0, color);
}

fn preset_ui(ui: &mut egui::Ui, id: usize, preset: &mut String, presets: &[Preset], eq: &[AtomicF32; BANDS]) {
    egui::ComboBox::from_id_salt(("preset", id))
        .selected_text(preset.as_str())
        .width(110.0)
        .show_ui(ui, |ui| {
            for pr in presets {
                if ui.selectable_value(&mut *preset, pr.name.to_string(), pr.name).clicked() {
                    for (a, g) in eq.iter().zip(pr.gains) {
                        a.set(g);
                    }
                }
            }
        });
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = 70.0;
        ui.horizontal(|ui| {
            for b in 0..BANDS {
                ui.vertical(|ui| {
                    let mut g = eq[b].get();
                    if ui
                        .add(egui::Slider::new(&mut g, -12.0..=12.0).vertical().show_value(false))
                        .on_hover_text(format!("{:+.1} dB", g))
                        .changed()
                    {
                        eq[b].set(g);
                        *preset = "Custom".to_string();
                    }
                    ui.small(BAND_LABELS[b]);
                });
            }
        });
    });
}

fn strip(
    ui: &mut egui::Ui,
    title: &str,
    p: &ChannelParams,
    preset: &mut String,
    presets: &[Preset],
    color: Color32,
    id: usize,
) {
    ui.vertical(|ui| {
        ui.set_width(128.0);
        ui.label(egui::RichText::new(title).strong().color(color));
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 150.0;
            let mut v = p.volume.get();
            if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).vertical().show_value(false)).changed() {
                p.volume.set(v);
            }
            meter(ui, p.peak.get(), 150.0, color);
        });
        ui.label(format!("{:.0}%", p.volume.get() * 100.0));
        let mut muted = p.muted.load(Relaxed);
        if ui.toggle_value(&mut muted, "Mute").changed() {
            p.muted.store(muted, Relaxed);
        }
        ui.add_space(6.0);
        preset_ui(ui, id, preset, presets, &p.eq);
    });
}

fn slider_row(ui: &mut egui::Ui, label: &str, a: &AtomicF32, range: std::ops::RangeInclusive<f32>, pct: bool) {
    ui.horizontal(|ui| {
        ui.add_sized([210.0, 18.0], egui::Label::new(label));
        let mut v = a.get();
        let mut slider = egui::Slider::new(&mut v, range);
        if pct {
            slider = slider
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0));
        }
        if ui.add(slider).changed() {
            a.set(v);
        }
    });
}

fn device_combo(ui: &mut egui::Ui, label: &str, sel: &mut Option<String>, names: &[String], none: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized([130.0, 18.0], egui::Label::new(label));
        egui::ComboBox::from_id_salt(label)
            .width(340.0)
            .selected_text(sel.clone().unwrap_or_else(|| none.to_string()))
            .show_ui(ui, |ui| {
                if ui.selectable_label(sel.is_none(), none).clicked() {
                    *sel = None;
                    changed = true;
                }
                for n in names {
                    if ui.selectable_label(sel.as_deref() == Some(n.as_str()), n).clicked() {
                        *sel = Some(n.clone());
                        changed = true;
                    }
                }
            });
    });
    changed
}

fn apply_theme(ctx: &egui::Context, t: &ThemeCfg) {
    let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let accent = Color32::from_rgb(t.accent[0], t.accent[1], t.accent[2]);
    v.selection.bg_fill = accent;
    v.hyperlink_color = accent;
    v.widgets.hovered.bg_stroke.color = accent;
    v.widgets.active.bg_fill = accent;
    ctx.set_visuals(v);
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 560.0])
            .with_title("Sonar Lite"),
        ..Default::default()
    };
    eframe::run_native(
        "Sonar Lite",
        options,
        Box::new(|cc| {
            let app = App::new();
            apply_theme(&cc.egui_ctx, &app.cfg.theme);
            Ok(Box::new(app))
        }),
    )
}
