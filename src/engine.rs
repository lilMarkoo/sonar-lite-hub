//! Audio engine.
//!
//! Signal flow:
//!   Game / Media / Aux source device -> ring -> mixer (EQ, volume, ChatMix) -+
//!   Chat source device -> mono ring -> RNNoise worker -> ring -> mixer ------+-> master EQ
//!                                                                              -> volume -> night mode -> limiter -> headphones
//!   Microphone -> mono ring -> worker (gain, RNNoise, gate, EQ, compressor) -> ring -> virtual mic device
//!
//! Everything runs at 48 kHz / f32 (RNNoise requires 48 kHz).

use crate::dsp::*;
use crate::params::*;
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, Stream, StreamConfig, SupportedStreamConfigRange};
use nnnoiseless::DenoiseState;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SAMPLE_RATE: u32 = 48_000;
const FRAME: usize = nnnoiseless::FRAME_SIZE; // 480 samples = 10 ms
const RING_STEREO: usize = 48_000 * 2; // 1 s
const RING_MONO: usize = 48_000; // 1 s
const PREFILL: usize = 1_920; // 20 ms of stereo samples before playback starts
const MAX_BUF: usize = 19_200; // above 200 ms of backlog, skip ahead (clock drift)
const TARGET_BUF: usize = 4_800; // skip down to 50 ms

pub struct DeviceLists {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub fn list_devices() -> DeviceLists {
    let host = cpal::default_host();
    let inputs = match host.input_devices() {
        Ok(it) => it.filter_map(|d| d.name().ok()).collect(),
        Err(_) => vec![],
    };
    let outputs = match host.output_devices() {
        Ok(it) => it.filter_map(|d| d.name().ok()).collect(),
        Err(_) => vec![],
    };
    DeviceLists { inputs, outputs }
}

// ------------------------------------------------------------------ device helpers

fn has_output(d: &Device) -> bool {
    d.supported_output_configs().map(|mut c| c.next().is_some()).unwrap_or(false)
}

fn has_input(d: &Device) -> bool {
    d.supported_input_configs().map(|mut c| c.next().is_some()).unwrap_or(false)
}

fn find_device(host: &cpal::Host, name: &str, output: bool) -> Result<Device> {
    for d in host.devices()? {
        if d.name().ok().as_deref() != Some(name) {
            continue;
        }
        // On Windows an output device can be opened as an input (WASAPI loopback).
        let ok = if output { has_output(&d) } else { has_input(&d) || (cfg!(windows) && has_output(&d)) };
        if ok {
            return Ok(d);
        }
    }
    Err(anyhow!("device not found: {name}"))
}

fn pick_config(dev: &Device, output: bool) -> Result<StreamConfig> {
    let ranges: Vec<SupportedStreamConfigRange> = if output {
        dev.supported_output_configs()?.collect()
    } else if has_input(dev) {
        dev.supported_input_configs()?.collect()
    } else if cfg!(windows) {
        dev.supported_output_configs()?.collect() // loopback
    } else {
        vec![]
    };
    let range = ranges
        .into_iter()
        .filter(|r| r.sample_format() == SampleFormat::F32)
        .find(|r| r.min_sample_rate().0 <= SAMPLE_RATE && r.max_sample_rate().0 >= SAMPLE_RATE)
        .ok_or_else(|| {
            anyhow!(
                "'{}' does not offer 48 kHz f32 audio. Set the device to 48000 Hz in your system sound settings.",
                dev.name().unwrap_or_default()
            )
        })?;
    Ok(range.with_sample_rate(SampleRate(SAMPLE_RATE)).config())
}

fn err_cb(e: cpal::StreamError) {
    eprintln!("audio stream error: {e}");
}

#[inline]
fn write_frame(out: &mut [f32], l: f32, r: f32) {
    if out.len() == 1 {
        out[0] = (l + r) * 0.5;
    } else {
        out[0] = l;
        out[1] = r;
        for x in out[2..].iter_mut() {
            *x = 0.0;
        }
    }
}

// ------------------------------------------------------------------ ring consumer with prebuffering

struct Source {
    cons: HeapCons<f32>,
    primed: bool,
}

impl Source {
    fn new(cons: HeapCons<f32>) -> Self {
        Self { cons, primed: false }
    }

    /// Fills `buf` (interleaved stereo). Handles underruns, prebuffering and drift.
    fn pull(&mut self, buf: &mut [f32]) {
        let n = buf.len();
        let avail = self.cons.occupied_len();
        if avail > MAX_BUF {
            self.cons.skip((avail - TARGET_BUF) & !1);
        }
        if !self.primed && self.cons.occupied_len() >= PREFILL.max(n) {
            self.primed = true;
        }
        if !self.primed {
            buf.fill(0.0);
            return;
        }
        let got = self.cons.pop_slice(buf);
        if got < n {
            buf[got..].fill(0.0);
            self.primed = false;
        }
    }
}

// ------------------------------------------------------------------ stream builders

fn build_stereo_source(dev: &Device, mut prod: HeapProd<f32>) -> Result<Stream> {
    let cfg = pick_config(dev, false)?;
    let ch = cfg.channels as usize;
    let mut tmp: Vec<f32> = Vec::with_capacity(16_384);
    let stream = dev.build_input_stream(
        &cfg,
        move |data: &[f32], _| {
            tmp.clear();
            for fr in data.chunks_exact(ch) {
                tmp.push(fr[0]);
                tmp.push(if ch > 1 { fr[1] } else { fr[0] });
            }
            prod.push_slice(&tmp);
        },
        err_cb,
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

fn build_mono_source(dev: &Device, mut prod: HeapProd<f32>) -> Result<Stream> {
    let cfg = pick_config(dev, false)?;
    let ch = cfg.channels as usize;
    let mut tmp: Vec<f32> = Vec::with_capacity(8_192);
    let stream = dev.build_input_stream(
        &cfg,
        move |data: &[f32], _| {
            tmp.clear();
            for fr in data.chunks_exact(ch) {
                tmp.push(fr.iter().sum::<f32>() / ch as f32);
            }
            prod.push_slice(&tmp);
        },
        err_cb,
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

fn build_stereo_out(dev: &Device, mut src: Source) -> Result<Stream> {
    let cfg = pick_config(dev, true)?;
    let ch = cfg.channels as usize;
    let mut buf = vec![0.0f32; 8_192];
    let stream = dev.build_output_stream(
        &cfg,
        move |data: &mut [f32], _| {
            let frames = data.len() / ch;
            let n = frames * 2;
            if buf.len() < n {
                buf.resize(n, 0.0);
            }
            src.pull(&mut buf[..n]);
            for f in 0..frames {
                write_frame(&mut data[f * ch..(f + 1) * ch], buf[2 * f], buf[2 * f + 1]);
            }
        },
        err_cb,
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

fn build_master(dev: &Device, mut srcs: Vec<Source>, shared: Arc<Shared>) -> Result<Stream> {
    let cfg = pick_config(dev, true)?;
    let ch = cfg.channels as usize;
    let mut eqs: Vec<Eq5> = (0..4).map(|_| Eq5::new()).collect();
    let mut master_eq = Eq5::new();
    let mut night = Compressor::new(10.0, 200.0);
    let mut limiter = Limiter::new();
    let mut bufs: Vec<Vec<f32>> = (0..4).map(|_| vec![0.0f32; 8_192]).collect();
    let mut mix = vec![0.0f32; 8_192];

    let stream = dev.build_output_stream(
        &cfg,
        move |data: &mut [f32], _| {
            let frames = data.len() / ch;
            let n = frames * 2;
            if mix.len() < n {
                mix.resize(n, 0.0);
                for b in bufs.iter_mut() {
                    b.resize(n, 0.0);
                }
            }
            mix[..n].fill(0.0);

            // ChatMix: slide toward Chat to duck Game, toward Game to duck Chat.
            let cm = shared.chatmix.get();
            let cm_gain = [
                if cm <= 0.5 { 1.0 } else { 2.0 * (1.0 - cm) },
                if cm >= 0.5 { 1.0 } else { 2.0 * cm },
                1.0,
                1.0,
            ];

            for i in 0..4 {
                let p = &shared.channels[i];
                srcs[i].pull(&mut bufs[i][..n]);
                eqs[i].update(p.eq_gains());
                let vol = if p.muted.load(Relaxed) { 0.0 } else { p.volume.get() * cm_gain[i] };
                let b = &bufs[i];
                let mut peak = 0.0f32;
                for f in 0..frames {
                    let (l, r) = eqs[i].stereo(b[2 * f], b[2 * f + 1]);
                    let (l, r) = (l * vol, r * vol);
                    peak = peak.max(l.abs()).max(r.abs());
                    mix[2 * f] += l;
                    mix[2 * f + 1] += r;
                }
                p.peak.set(peak.max(p.peak.get() * 0.9));
            }

            let m = &shared.master;
            master_eq.update(m.eq_gains());
            let mv = if m.muted.load(Relaxed) { 0.0 } else { m.volume.get() };
            let night_on = shared.night_mode.load(Relaxed);
            let mut peak = 0.0f32;
            for f in 0..frames {
                let (mut l, mut r) = master_eq.stereo(mix[2 * f], mix[2 * f + 1]);
                l *= mv;
                r *= mv;
                if night_on {
                    let g = night.gain(l.abs().max(r.abs()), -24.0, 4.0) * 1.8;
                    l *= g;
                    r *= g;
                }
                let (l, r) = limiter.process(l, r);
                peak = peak.max(l.abs()).max(r.abs());
                write_frame(&mut data[f * ch..(f + 1) * ch], l, r);
            }
            m.peak.set(peak.max(m.peak.get() * 0.9));
        },
        err_cb,
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

// ------------------------------------------------------------------ voice worker (AI noise cancellation)

#[derive(Clone, Copy)]
enum Kind {
    Mic,
    Chat,
}

fn voice(s: &Shared, k: Kind) -> &VoiceParams {
    match k {
        Kind::Mic => &s.mic,
        Kind::Chat => &s.chat_voice,
    }
}

fn spawn_worker(
    mut input: HeapCons<f32>,
    mut output: HeapProd<f32>,
    shared: Arc<Shared>,
    kind: Kind,
    running: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let p = voice(&shared, kind);
        let mut dn = DenoiseState::new();
        let mut eq = Eq5::new();
        let mut comp = Compressor::new(5.0, 120.0);
        let mut fin = [0.0f32; FRAME];
        let mut fout = [0.0f32; FRAME];
        let mut stereo = [0.0f32; FRAME * 2];
        let mut gate_gain = 1.0f32;
        let mut hold = 0u32;
        let mut warm = true;

        while running.load(Relaxed) {
            let avail = input.occupied_len();
            if avail < FRAME {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            if avail > FRAME * 20 {
                input.skip(avail - FRAME * 4); // drop backlog if we fell behind
            }
            input.pop_slice(&mut fin);

            // RNNoise expects samples in 16-bit range.
            let g = db_to_lin(p.gain_db.get()) * 32768.0;
            for s in fin.iter_mut() {
                *s *= g;
            }
            let vad = dn.process_frame(&mut fout, &fin);
            if warm {
                warm = false; // first RNNoise frame is a fade-in, discard it
                continue;
            }

            let amt = p.denoise.get().clamp(0.0, 1.0);
            let thr = p.gate.get();
            let target = if thr <= 0.0 || vad >= thr {
                hold = 12; // ~120 ms hold so word endings are not chopped
                1.0
            } else if hold > 0 {
                hold -= 1;
                1.0
            } else {
                0.0
            };
            gate_gain += (target - gate_gain) * if target > gate_gain { 0.5 } else { 0.2 };

            eq.update(p.eq_gains());
            let comp_on = p.comp.load(Relaxed);
            let muted = p.muted.load(Relaxed);
            let mut peak = 0.0f32;
            for i in 0..FRAME {
                let mut x = (fout[i] * amt + fin[i] * (1.0 - amt)) / 32768.0 * gate_gain;
                x = eq.mono(x);
                if comp_on {
                    x *= comp.gain(x.abs(), -18.0, 3.0) * 1.4;
                }
                if muted {
                    x = 0.0;
                }
                x = x.clamp(-1.0, 1.0);
                peak = peak.max(x.abs());
                stereo[2 * i] = x;
                stereo[2 * i + 1] = x;
            }
            p.peak.set(peak.max(p.peak.get() * 0.85));
            output.push_slice(&stereo);
        }
    })
}

// ------------------------------------------------------------------ engine

pub struct Engine {
    streams: Vec<Stream>,
    workers: Vec<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl Engine {
    pub fn start(dev: &Devices, shared: Arc<Shared>) -> Result<Engine> {
        let host = cpal::default_host();
        // Created first so that a failure part-way still stops any workers (see Drop).
        let mut eng = Engine { streams: vec![], workers: vec![], running: Arc::new(AtomicBool::new(true)) };

        let master_dev = match &dev.master_out {
            Some(n) => find_device(&host, n, true)?,
            None => host.default_output_device().ok_or_else(|| anyhow!("no default output device"))?,
        };

        // Playback channels: Game, Chat, Media, Aux.
        let mut srcs = Vec::with_capacity(4);
        for i in 0..4 {
            let (prod, cons) = HeapRb::<f32>::new(RING_STEREO).split();
            match &dev.sources[i] {
                Some(name) => {
                    let d = find_device(&host, name, false)?;
                    if i == CHAT {
                        let (raw_p, raw_c) = HeapRb::<f32>::new(RING_MONO).split();
                        eng.streams.push(build_mono_source(&d, raw_p)?);
                        eng.workers.push(spawn_worker(raw_c, prod, shared.clone(), Kind::Chat, eng.running.clone()));
                    } else {
                        eng.streams.push(build_stereo_source(&d, prod)?);
                    }
                }
                None => drop(prod),
            }
            srcs.push(Source::new(cons));
        }
        eng.streams.push(build_master(&master_dev, srcs, shared.clone())?);

        // Microphone chain.
        if let Some(name) = &dev.mic_in {
            let d = find_device(&host, name, false)?;
            let (raw_p, raw_c) = HeapRb::<f32>::new(RING_MONO).split();
            let (out_p, out_c) = HeapRb::<f32>::new(RING_STEREO).split();
            eng.streams.push(build_mono_source(&d, raw_p)?);
            eng.workers.push(spawn_worker(raw_c, out_p, shared.clone(), Kind::Mic, eng.running.clone()));
            if let Some(out_name) = &dev.mic_out {
                let od = find_device(&host, out_name, true)?;
                eng.streams.push(build_stereo_out(&od, Source::new(out_c))?);
            }
        }
        Ok(eng)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.streams.clear();
        self.running.store(false, Relaxed);
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}
