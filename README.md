# Sonar Lite

A small native Rust replacement for the Sonar part of SteelSeries GG. No account, no store, no news feed, no game hub.

## Features

- Master + 4 channels (Game, Chat, Media, Aux): volume, mute, level meters
- ChatMix: slide toward Chat to duck Game, toward Game to duck Chat
- 5-band EQ per channel and on Master, with one-click presets:
  Bass Boost, Treble Boost, Vocal Boost, Footsteps (FPS), Cinema, Music (V-shape), Late Night
- Night mode: compressor that tames loud sounds and keeps quiet ones audible, plus an always-on output limiter
- Microphone chain: gain, **AI noise cancellation** (RNNoise), voice-activity noise gate, EQ presets
  (Clear Voice, Warm Broadcast, Reduce Boominess, Presence), compressor, mute
- **AI cleanup for incoming voice** on the Chat channel
- Settings saved automatically on exit

## Get the executable

1. Install Rust from https://rustup.rs
2. Windows: double-click `build.bat` and you get `SonarLite.exe` (no console window).
   Linux / macOS: run `./build.sh` and you get `./SonarLite`.

No Rust installed? Use GitHub to build it and publish a download page, like most open-source apps do:

1. Create a free GitHub repository and upload the contents of this folder (including `.github`).
   Easiest: install GitHub Desktop, add this folder, and publish it.
2. On the repo page open Releases > Draft a new release, type a new tag such as `v0.1.0`, and click Publish release.
3. Wait a few minutes. The workflow (`.github/workflows/build.yml`) builds Windows, Linux and macOS versions and
   attaches `SonarLite-windows.exe`, `SonarLite-linux` and `SonarLite-macos` to that release page.
   Anyone can download from there.

For a test build without publishing: Actions > release > Run workflow, then download the file from the finished run.

Windows may show a SmartScreen warning for an unsigned app. Choose More info > Run anyway.

## Theme

The Theme tab switches between Dark and Light and lets you pick any accent color (or one of five quick swatches).
It is saved with the rest of your settings.

## Build for development

```
cargo run --release
```

Needs a recent stable Rust toolchain. On Linux you also need ALSA and a windowing stack
(`libasound2-dev`, `libxkbcommon-dev`, `libwayland-dev` or X11 dev packages).

All streams run at 48 kHz f32 (RNNoise requires 48 kHz). If a device refuses, set it to 48000 Hz in your system sound settings.

## How audio gets into the channels

Like Sonar, this app mixes **virtual devices**. A normal program cannot grab audio from individual apps, so you send each
app to its own virtual device, then choose that device as the channel source under *Devices*.

**Windows**
- Install VB-CABLE (A, B, C, D variants give you several cables) or similar virtual devices.
- Windows Settings > Sound > Volume mixer: set each app's output to a cable (Game -> cable A, Discord -> cable B, ...).
- In Sonar Lite, set the Game source to that cable's *recording* side (e.g. "CABLE-A Output"). Any playback device
  also works as a source through loopback.
- Mic: pick your real mic as *Microphone in*, a cable's *playback* side (e.g. "CABLE-C Input") as *Virtual mic out*,
  then select "CABLE-C Output" as the microphone inside Discord, OBS, games.

**Linux (PipeWire / PulseAudio)**
```
pactl load-module module-null-sink sink_name=game sink.description=Game
pactl load-module module-null-sink sink_name=chat sink.description=Chat
pactl load-module module-null-sink sink_name=cleanmic sink.description=CleanMic
pactl load-module module-remap-source master=cleanmic.monitor source_name=cleanmic_src source_properties=device.description=CleanMic
```
Send apps to the sinks with pavucontrol, use "Monitor of Game" as the Game source, "cleanmic" as Virtual mic out,
and "CleanMic" as the mic inside your apps.

**macOS**: BlackHole (2ch / 16ch) as the virtual devices, same idea.

## Not included

Per-app auto-routing (needs a kernel/virtual driver), spatial/Immersive Audio, parametric EQ with custom curves,
hardware (GameDAC / headset) integration, and game-specific presets.

## Layout

```
src/main.rs     egui UI
src/engine.rs   cpal streams, ring buffers, mixer, RNNoise workers
src/dsp.rs      biquad EQ, compressor, limiter
src/params.rs   lock-free shared parameters + saved config
src/presets.rs  EQ presets
```
