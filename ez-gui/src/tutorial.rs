use crate::app::Tab;
use crate::user_level::UserLevel;

pub struct TutorialStep {
    pub title: &'static str,
    pub body: &'static str,
    pub highlight: Option<&'static str>,
    pub tab: Option<Tab>,
    pub action: Option<(&'static str, TutorialAction)>,
}

pub enum TutorialAction {
    TuneFreq(u64),
    SetDemod(&'static str),
    StartAudio,
    OpenSettings,
}

fn steps_for_level(level: UserLevel) -> Vec<TutorialStep> {
    match level {
        UserLevel::Beginner => beginner_steps(),
        UserLevel::Intermediate => intermediate_steps(),
        UserLevel::Advanced => advanced_steps(),
        UserLevel::ClerkMaxwell => clerk_maxwell_steps(),
    }
}

fn beginner_steps() -> Vec<TutorialStep> {
    vec![
        TutorialStep {
            title: "👋 Welcome to EZ-SDR!",
            body: "EZ-SDR is a Software-Defined Radio that turns your computer into a powerful radio receiver. \
                   You can listen to aircraft, satellites, FM radio, weather stations, and much more.\n\n\
                   This tutorial will walk you through everything — don't worry, we'll start simple.\n\n\
                   Interface overview: The main window has a status bar at top (signal meter, frequency, \
                   recording status), a large spectrum/waterfall display on the left that shows all signals \
                   visually, and a tabbed panel area on the right for controls. Use the tabs to switch \
                   between SDR controls, satellite tracking, ADS-B aircraft, and more.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "📡 No Hardware? No Problem!",
            body: "Don't have an SDR dongle yet? EZ-SDR runs in DEMO mode automatically — it generates \
                   synthetic signals so you can explore the full interface without any hardware.\n\n\
                   When you're ready for real signals, get an RTL-SDR Blog V3 or V4 ($25-35), \
                   connect it, and rebuild with the 'rtlsdr' feature enabled.\n\n\
                   For now, let's explore what the app can do with demo signals.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "📻 SDR Panel — Tuning In",
            body: "The 📻 SDR tab (on the right) is your main control panel. Here's what matters right now:\n\n\
                   • Frequency: Type a number (in MHz) or use the large ±1M and ±100k buttons to tune.\n\
                   • Band presets: Click 'FM' to jump to 98 MHz, 'Air' for aircraft, 'NOAA' for weather.\n\
                   • Demod mode: Pick AM, FM, or WFM depending on the signal type.\n\
                   • Gain: Start around 40 dB. Higher = more signals but also more noise.\n\
                   • Squelch: Press 'Auto' to silence static between transmissions.\n\n\
                   Let's try it: click 'FM Radio' in the next step!",
            highlight: Some("sdr_panel"),
            tab: Some(Tab::Sdr),
            action: None,
        },
        TutorialStep {
            title: "🔊 Listening to Audio (One Click!)",
            body: "Click the button below to instantly tune to a strong FM radio station. \
                   This sets everything up automatically: frequency, demod mode, gain, and audio.\n\n\
                   After clicking, you'll hear music or talk — just like a regular FM radio.\n\n\
                   Audio controls: Press 'M' to mute/unmute, or adjust the Volume slider in the status bar.",
            highlight: None,
            tab: None,
            action: Some(("📻  Listen to FM Radio", TutorialAction::TuneFreq(98_000_000))),
        },
        TutorialStep {
            title: "📊 Spectrum & Waterfall",
            body: "The large display on the left is your signal window:\n\n\
                   • The top half is the SPECTRUM — shows live signal strength across frequencies. \
                   Peaks = signals.\n\
                   • The bottom half is the WATERFALL — a scrolling history. Bright colors = strong signals.\n\n\
                   Quick tips:\n\
                   • Click anywhere on the spectrum to tune there.\n\
                   • Scroll to zoom in/out.\n\
                   • Press 'F' to freeze the display.\n\
                   • Press 'C' to cycle through color schemes.\n\n\
                   This is your most powerful tool — learn to read it and you'll find signals everywhere.",
            highlight: Some("spectrum"),
            tab: Some(Tab::Spectrum),
            action: None,
        },
        TutorialStep {
            title: "🤖 AI Agent — Your Co-Pilot",
            body: "Not sure what to do? The AI Agent tab is your built-in assistant. \
                   Click the 🤖 AI Agent tab and ask questions in plain English:\n\n\
                   • \"What can I listen to?\"\n\
                   • \"Tune to 98 MHz FM\"\n\
                   • \"Find me a satellite pass\"\n\
                   • \"What's this signal at 137 MHz?\"\n\n\
                   The AI can control the SDR directly — tuning, changing modes, setting gain, \
                   even starting recordings. Think of it as a radio expert sitting next to you.\n\n\
                   For beginners, this is the fastest way to get results.",
            highlight: Some("ai_panel"),
            tab: Some(Tab::AiAgent),
            action: None,
        },
        TutorialStep {
            title: "✈ ADS-B — Tracking Aircraft",
            body: "Every commercial aircraft broadcasts its position, altitude, and speed on 1090 MHz. \
                   EZ-SDR can decode this and show planes on a live map.\n\n\
                   Quick start: Switch to the ✈ ADS-B tab, complete the antenna checklist, \
                   and click 'Start ADS-B'. Aircraft will appear as dots on the map.\n\n\
                   You can also just ask the AI: \"Start ADS-B tracking\" and it'll handle everything.",
            highlight: Some("adsb_panel"),
            tab: Some(Tab::AdsB),
            action: None,
        },
        TutorialStep {
            title: "🛸 Satellite Tracking",
            body: "EZ-SDR can track NOAA weather satellites, the ISS, and more. \
                   These satellites broadcast live images and data as they pass overhead.\n\n\
                   Quick start: Go to the 🛸 Satellite tab, download TLE data (orbital info), \
                   and click 'Update Passes'. The app will show upcoming passes and can \
                   auto-tune when a satellite rises.\n\n\
                   Or just ask the AI: \"Track NOAA 19\" and it'll set everything up.",
            highlight: Some("satellite_panel"),
            tab: Some(Tab::Satellite),
            action: None,
        },
        TutorialStep {
            title: "🔍 Scanner — Find Active Signals",
            body: "The Scanner sweeps across a frequency range and logs every active signal it finds. \
                   It's excellent for discovering what's happening on the air.\n\n\
                   Quick start: Go to the 🔍 Scanner tab, pick a preset (like 'VHF' or 'Airband'), \
                   and click Start. The scanner will sweep and build a list of hits.\n\n\
                   You can also ask the AI: \"Scan the VHF band for active signals.\"",
            highlight: Some("scanner_panel"),
            tab: Some(Tab::Scanner),
            action: None,
        },
        TutorialStep {
            title: "⭐ Bookmarks & Recorder",
            body: "Found an interesting frequency? Save it as a bookmark (Ctrl+B) so you can \
                   return anytime. Organize bookmarks by category and star your favorites.\n\n\
                   For recording interesting signals, use the ⏺ Recorder tab. You can record \
                   raw I/Q data (for later analysis) or decoded audio (WAV files).\n\n\
                   Both are accessible from their respective tabs in the right panel.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "🎉 You're Ready!",
            body: "You now know the basics of EZ-SDR! Here's a quick recap:\n\n\
                   • 📻 SDR tab — tune, demodulate, listen\n\
                   • 📊 Spectrum — see signals, click to tune\n\
                   • 🤖 AI Agent — ask anything in plain English\n\
                   • ✈ ADS-B — track aircraft live\n\
                   • 🛸 Satellites — catch NOAA passes\n\
                   • 🔍 Scanner — discover active frequencies\n\n\
                   Explore freely! You can change your experience level in ⚙ Settings \
                   anytime — increase it to see more advanced controls as you learn.\n\n\
                   Have fun exploring the radio spectrum! 📡",
            highlight: None,
            tab: None,
            action: None,
        },
    ]
}

fn intermediate_steps() -> Vec<TutorialStep> {
    vec![
        TutorialStep {
            title: "👋 Welcome to EZ-SDR",
            body: "EZ-SDR is a full-featured software-defined radio application. You can receive \
                   aircraft ADS-B, NOAA weather satellites, FM/AM broadcasts, ham radio, and more.\n\n\
                   The main window has the spectrum/waterfall on the left and tabbed control panels \
                   on the right. The status bar shows frequency, signal strength, and audio status.\n\n\
                   This tutorial will walk through the key panels.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "📻 SDR Panel — Full Controls",
            body: "The 📻 SDR tab has everything you need to tune and demodulate:\n\n\
                   • Frequency: Type MHz or use buttons. Band presets for quick jumps.\n\
                   • Demod: AM for voice/aircraft, FM for two-way radio, WFM for broadcast, LSB/USB for ham.\n\
                   • Gain: 30-45 dB typical. Too high = overload (phantom signals).\n\
                   • Squelch: Set 5 dB above noise floor. 'Auto' works well.\n\
                   • VFO A vs VFO B: Press 'V' to swap between two frequencies.\n\
                   • Filter bandwidth: Wider for music, narrower for clear voice.\n\n\
                   The spectrum shows your tuned position — you can click anywhere to tune.",
            highlight: Some("sdr_panel"),
            tab: Some(Tab::Sdr),
            action: None,
        },
        TutorialStep {
            title: "📊 Spectrum & Waterfall",
            body: "This is your real-time radio window:\n\n\
                   • Spectrum (top): Current signal strengths across frequencies. \
                   Peaks = strongest signals.\n\
                   • Waterfall (bottom): Time scrolling down — past signals visible as colored trails.\n\n\
                   Controls: Freeze (F), zoom (scroll), pan (Shift+scroll), colormap (C), \
                   peak hold (P), and averaging speed (Fast/Med/Slow).\n\n\
                   Right-click on a signal for context menu: tune, bookmark, or ask the AI about it.\n\
                   Left-drag on the waterfall to pan the visible range.",
            highlight: Some("spectrum"),
            tab: Some(Tab::Spectrum),
            action: None,
        },
        TutorialStep {
            title: "🤖 AI Agent",
            body: "The AI Agent can control the SDR and answer questions. It understands \
                   natural language commands like:\n\n\
                   • \"Tune to 118.5 MHz AM\"\n\
                   • \"Set gain to 35 dB\"\n\
                   • \"Start recording\"\n\
                   • \"What's the ADS-B traffic?\"\n\n\
                   It's great for quick tasks and for identifying unknown signals. \
                   Configure your API key in ⚙ Settings under AI Agent.",
            highlight: Some("ai_panel"),
            tab: Some(Tab::AiAgent),
            action: None,
        },
        TutorialStep {
            title: "✈ ADS-B + 🛸 Satellites",
            body: "ADS-B: Switch to the ✈ ADS-B tab, complete the checklist, and start decoding. \
                   All aircraft within range appear with ICAO, callsign, altitude, speed, and heading. \
                   The map view shows positions. Use filters to narrow down traffic.\n\n\
                   Satellites: Go to 🛸 Satellite tab, download TLE data, then click 'Update Passes'. \
                   Enable 'Auto-tune to downlink + Doppler' for automatic tracking. \
                   The scheduler shows upcoming pass times.\n\n\
                   Both features work best with a proper antenna setup — the checklists help.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "🔍 Scanner & ⏺ Recorder",
            body: "Scanner: Sweeps a frequency range and logs hits. Choose a preset or set \
                   custom start/stop/step values. Hits are sorted by strength or discovery order. \
                   'Hold on active' pauses on signals that exceed the threshold.\n\n\
                   Recorder: Records I/Q (raw) or audio (WAV). You can set max duration, \
                   enable squelch-triggered recording (auto-starts when a signal appears), \
                   and browse recorded files in the file list.\n\n\
                   Both have CSV export for further analysis.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "⭐ Bookmarks & 🗓 Scheduler",
            body: "Bookmarks: Save frequencies with names, categories, and notes. \
                   Star favorites for quick filtering. Import/export CSV for sharing. \
                   Press B to jump to the nearest bookmark.\n\n\
                   Scheduler: Shows satellite pass times on a timeline. Custom tasks let you \
                   schedule frequency changes at specific times. Use it to catch passes \
                   automatically even when you're away.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "⚙ Settings & Integrations",
            body: "Open ⚙ Settings to configure:\n\n\
                   • AI Agent: API provider, key, model, temperature\n\
                   • MQTT: Publish SDR state to external systems\n\
                   • Web Remote: Control from a browser (port 5259)\n\
                   • Discord: Get notifications for signals, aircraft, passes\n\
                   • PPM calibration: Fix frequency offset from crystal drift\n\
                   • Appearance: Theme, font scale, color editor\n\
                   • User Experience: Change your level anytime\n\n\
                   Settings are saved to ez_sdr_config.json (Ctrl+S to save manually).",
            highlight: None,
            tab: Some(Tab::Settings),
            action: None,
        },
        TutorialStep {
            title: "✅ You're All Set!",
            body: "You've covered the main features of EZ-SDR:\n\n\
                   • 📻 SDR Panel — tune, demod, audio\n\
                   • 📊 Spectrum & Waterfall — visual signal analysis\n\
                   • 🤖 AI Agent — natural language radio control\n\
                   • ✈ ADS-B — live aircraft tracking\n\
                   • 🛸 Satellites — NOAA/ISS pass prediction\n\
                   • 🔍 Scanner — automatic signal discovery\n\
                   • ⏺ Recorder — save IQ and audio\n\
                   • ⭐ Bookmarks — organize frequencies\n\
                   • ⚙ Settings — full configuration\n\n\
                   As you get more comfortable, raise your User Level in Settings to unlock \
                   more controls and experimental features. Happy listening! 📡",
            highlight: None,
            tab: None,
            action: None,
        },
    ]
}

fn advanced_steps() -> Vec<TutorialStep> {
    vec![
        TutorialStep {
            title: "👋 EZ-SDR — Advanced User",
            body: "Full software-defined radio suite with DSP chain, ADS-B decoder, \
                   satellite pass engine, frequency scanner, AI integration, MQTT, \
                   Discord notifications, and web remote control.\n\n\
                   Brief layout: spectrum/waterfall left, control tabs right, \
                   status bar top. All controls are visible at this level.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "📻 SDR Panel — Complete",
            body: "All controls are exposed:\n\n\
                   • VFO A/B with swap (V key), Set B Here from spectrum right-click\n\
                   • PPM correction with quick presets\n\
                   • LO offset for upconverter mode\n\
                   • Gain with fine slider and presets\n\
                   • Filter bandwidth per demod mode\n\
                   • Frequency memory (M1-M9): Alt+Shift+1-9 to save, Alt+1-9 to recall\n\
                   • Bias tee toggle (hardware support needed)\n\
                   • Direct sampling mode (HF reception without upconverter)\n\
                   • Audio controls: volume, squelch with Auto/Off\n\
                   • Signal meter with sparkline\n\n\
                   Band presets cover AM broadcast to GOES satellite downlinks.",
            highlight: Some("sdr_panel"),
            tab: Some(Tab::Sdr),
            action: None,
        },
        TutorialStep {
            title: "📊 Spectrum — Full Feature Set",
            body: "In addition to basic controls:\n\n\
                   • FFT window type: Blackman-Harris, Hamming, Hann, Kaiser, Nuttall\n\
                   • Averaging: Alpha slider or Fast/Med/Slow/XSlow presets\n\
                   • Waterfall speed: 1x-8x\n\
                   • dB range: Independent min/max sliders for spectrum and waterfall\n\
                   • Peak hold toggle (P key)\n\
                   • CSV export of spectrum data\n\
                   • PNG screenshot of waterfall\n\
                   • Frequency markers (middle-click to drop)\n\
                   • VFO BW overlay, bookmark markers on spectrum\n\n\
                   Controls: Zoom (scroll), pan (Shift+scroll), context menu (right-click).",
            highlight: Some("spectrum"),
            tab: Some(Tab::Spectrum),
            action: None,
        },
        TutorialStep {
            title: "🤖 AI Agent Integration",
            body: "The AI can control the SDR via tool calls:\n\n\
                   • tune_frequency, set_gain, set_demod, set_sample_rate\n\
                   • toggle_bias_tee, start/stop_recording\n\
                   • select_satellite, start/stop_adsb\n\
                   • set_squelch, set_volume, set_lpf_cutoff, set_ppm\n\
                   • get_status (returns full JSON state)\n\
                   • add_bookmark\n\n\
                   Supports multiple providers: OpenRouter, Anthropic, OpenAI, Groq, \
                   Mistral, Ollama (local), or custom endpoint.\n\n\
                   Configure in ⚙ Settings → AI Agent.",
            highlight: Some("ai_panel"),
            tab: Some(Tab::AiAgent),
            action: None,
        },
        TutorialStep {
            title: "✈ ADS-B + 🛸 Satellites",
            body: "ADS-B: 1090 MHz decoder. Table view with ICAO, callsign, altitude, speed, \
                   heading. Map view with position dots. Trail rendering. \
                   Planespotters API integration for aircraft model/operator lookup. \
                   Altitude and age filters. Desktop notifications for new aircraft.\n\n\
                   Satellites: TLE-based pass prediction. Doppler correction with auto-tune. \
                   Supports NOAA 15/18/19, Meteor-M2, ISS, and any satellite with TLE data. \
                   Observer location configurable in Settings.\n\n\
                   Both panels include antenna setup checklists with pass/fail gates.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "🔍 Scanner & ⏺ Recorder",
            body: "Scanner: Configurable range, step, dwell, threshold. Memory scan mode \
                   scans bookmarks instead of a range. Presets for FM, VHF, Airband, ISM, \
                   2m amateur. Histogram view. 'Hold on active' with resume delay. \
                   CSV export of scan hits.\n\n\
                   Recorder: I/Q (raw float32) and/or audio (WAV) recording. \
                   Squelch-triggered recording with configurable tail time. \
                   File browser with delete. Custom filename templates. \
                   Signal event logging for post-session analysis.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "⚙ Full Configuration",
            body: "All settings are in ⚙ Settings:\n\n\
                   • Source: Default freq, sample rate, gain\n\
                   • PPM Calibration: -100 to +100 with quick presets\n\
                   • AI Agent: All provider presets, model, temperature, system prompt\n\
                   • MQTT: Broker, topic prefix, auto-publish\n\
                   • Web Remote: HTTP server on configurable port\n\
                   • Discord: Bot token, channel, notification kind toggles\n\
                   • Appearance: Theme preset + per-color editor (31 colors)\n\
                   • Satellite: Observer lat/lon\n\
                   • User Experience: Level control, restart tutorial\n\n\
                   Export/Import for backup. Ctrl+S saves session state.",
            highlight: None,
            tab: Some(Tab::Settings),
            action: Some(("⚙ Open Settings", TutorialAction::OpenSettings)),
        },
        TutorialStep {
            title: "✅ Ready",
            body: "All features are available at this level. Switch to Clerk_Maxwell \
                   in Settings → User Experience to unlock experimental features \
                   (FFT window types, DSP chain view, debug overlays, developer console).\n\n\
                   Keyboard shortcuts: Press ? for the full reference (70+ shortcuts). \
                   💾 Ctrl+S to save config. Happy exploring!",
            highlight: None,
            tab: None,
            action: None,
        },
    ]
}

fn clerk_maxwell_steps() -> Vec<TutorialStep> {
    vec![
        TutorialStep {
            title: "👋 Clerk_Maxwell — Welcome",
            body: "Full access mode. All controls, all config, experimental features unlocked.\n\n\
                   You have access to:\n\
                   • FFT window type selection (Blackman-Harris, Hamming, Hann, Kaiser, Nuttall)\n\
                   • Raw DSP chain view with per-stage metrics\n\
                   • Debug performance overlays (FPS, frame timing)\n\
                   • Advanced FFT parameters (overlap %, zero-padding)\n\
                   • Raw I/Q hex inspector\n\
                   • Source device internals (USB buffers, transfers)\n\
                   • Signal chain block diagram\n\
                   • Developer console with diagnostics\n\
                   • Custom keybinding editor\n\n\
                   This tour is brief — you know what you're doing.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "📻 SDR Panel — Everything",
            body: "All SDR controls exposed:\n\
                   • Frequency: direct MHz input, DragValue, ± buttons, band presets, bookmark quick-tune\n\
                   • VFO A/B with swap, Set B Here, frequency memory M1-M9\n\
                   • Sample rate: 1M-2.88M buttons\n\
                   • Gain: 0-49.6 dB slider + presets\n\
                   • PPM: -100 to +100 with quick adjust\n\
                   • LO offset: upconverter mode\n\
                   • Demod: RAW, AM, FM, WFM, LSB, USB + bandwidth per mode\n\
                   • Bias tee, direct sampling toggles\n\
                   • Squelch with Auto/Off, filter LPF cutoff\n\
                   • Signal log with SNR tracking\n\
                   • Airport frequency finder with database download\n\
                   • Memory editor (M1-M9 labels)\n\n\
                   Developer: USB buffer stats, device descriptor info available in status area.",
            highlight: Some("sdr_panel"),
            tab: Some(Tab::Sdr),
            action: None,
        },
        TutorialStep {
            title: "🔧 DSP & Signal Chain",
            body: "The signal chain flows: RF → Mixer → Decimator → \
                   Band-pass Filter → Demodulator → Audio Output\n\n\
                   Experimental features unlocked:\n\
                   • FFT window: Blackman-Harris Nuttall, Hamming, Hann, Kaiser, \
                   Flat Top, Bartlett, Rectangular\n\
                   • Advanced FFT params: overlap %, zero-padding factor, \
                   resolution bandwidth display\n\
                   • DSP chain panel: see each stage's frequency response, \
                   input/output levels, CPU cost per stage\n\
                   • Raw I/Q hex viewer: inspect raw samples by address\n\n\
                   These are in the Spectrum panel's expanded controls.",
            highlight: Some("spectrum"),
            tab: Some(Tab::Spectrum),
            action: None,
        },
        TutorialStep {
            title: "⛏ Developer Tools",
            body: "Debug and diagnostic features:\n\n\
                   • Performance overlay: FPS counter, frame render time, \
                   lock contention stats, sample rate drift\n\
                   • Source internals: USB async transfer count, dropped buffers, \
                   device descriptor, temperature (if supported)\n\
                   • Signal chain diagram: live block diagram with per-block \
                   latency and throughput metrics\n\
                   • Developer console: mini REPL for internal diagnostics, \
                   log viewer with filter levels\n\
                   • Custom keybinding editor: rebind any of the 70+ shortcuts\n\n\
                   Access these from the SDR panel's advanced section or status bar debug menu \
                   (visible only at this level).",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "✈ ADS-B + 🛸 Satellites (Full)",
            body: "ADS-B: Full decoder configuration — tuner bandwidth, \
                   message rate display, raw Mode-S frame inspector. \
                   Aircraft filtering by min/max altitude, age, callsign. \
                   Trail history. Planespotters API integration.\n\n\
                   Satellites: TLE from multiple sources (Celestrak, Space-Track). \
                   Manual Doppler override. Live decode status with APT signal levels. \
                   Auto-record passes.\n\n\
                   Both have CSV export, Discord alerts, and automated scheduling.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "🔍 Scanner & ⏺ Recorder (Full)",
            body: "Scanner: Full parameter control (step, dwell, threshold). \
                   Memory scan across bookmarks. Frequency exclude list. \
                   CSV export. Histogram. Hold-on-active with configurable resume delay. \
                   Scan presets.\n\n\
                   Recorder: I/Q + audio WAV. Squelch-triggered with tail. \
                   Custom filename templates. Signal event log. \
                   Max duration. File browser with delete. \
                   Raw recording mode for post-processing.\n\n\
                   Both integrate with the scheduler for timed operations.",
            highlight: None,
            tab: None,
            action: None,
        },
        TutorialStep {
            title: "⚙ Configuration & Integrations",
            body: "All settings available:\n\n\
                   • AI: All providers, temperature, max tokens, custom system prompt\n\
                   • MQTT: Full broker config with auto-publish topics\n\
                   • Web Remote: HTTP server for browser control\n\
                   • Discord: 30+ notification kinds, per-kind enable/disable, \
                   session summaries, rate limiting\n\
                   • Theme: 5 presets + individual color editor (31 colors)\n\
                   • PPM: Fine calibration\n\
                   • Observer location for satellite predictions\n\
                   • User level: Change anytime (dev console requires this level)\n\n\
                   Export/Import JSON. Ctrl+S to save session state.",
            highlight: None,
            tab: Some(Tab::Settings),
            action: None,
        },
        TutorialStep {
            title: "🔬 Hidden Features Overview",
            body: "At Clerk_Maxwell level, you also get:\n\n\
                   • FFT window selection (Spectrum expanded controls)\n\
                   • DSP chain view with live per-stage metrics\n\
                   • Debug overlay: FPS, timing, lock stats\n\
                   • Raw I/Q hex inspector\n\
                   • Source device internals\n\
                   • Signal chain block diagram\n\
                   • Developer console\n\
                   • Custom keybinding editor\n\n\
                   These are accessed via the expanded controls in each panel \
                   or the debug menu in the status bar.\n\n\
                   Press ? for the full keyboard shortcut reference (70+ shortcuts).\
                   \n\nEnjoy full command of the radio spectrum! 📡",
            highlight: None,
            tab: None,
            action: None,
        },
    ]
}

/// Render the main tutorial overlay. Returns true when the tutorial is fully dismissed.
pub fn render_tutorial(
    state: &mut crate::user_level::TutorialState,
    shared: &std::sync::Arc<std::sync::Mutex<crate::app::SharedState>>,
    ui: &mut egui::Ui,
) -> bool {
    let mut dismissed = false;

    // If we need to ask about resume
    if state.asked_resume && state.resume_response.is_none() {
        egui::Window::new("Resume Tutorial?")
            .id(egui::Id::new("tutorial_resume"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(400.0)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(egui::RichText::new(format!(
                    "You were on step {} of the tutorial. Continue where you left off?",
                    state.step + 1
                )).size(16.0));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("▶ Resume").clicked() {
                        state.resume_response = Some(true);
                    }
                    if ui.button("🔄 Start Over").clicked() {
                        state.step = 0;
                        state.level_chosen = true;
                        state.resume_response = Some(true);
                    }
                });
            });
        return false;
    }
    if state.asked_resume && state.resume_response.is_none() {
        return false;
    }

    // Handle skip confirmation dialogs
    if state.skip_confirm_phase == 1 {
        let mut cancel = false;
        let mut skip = false;
        egui::Window::new("Skip Tutorial?")
            .id(egui::Id::new("skip_confirm_1"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(420.0)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 50),
                    "⚠ Skipping the tutorial may hide features you're not aware of.");
                ui.add_space(8.0);
                ui.label("EZ-SDR has many features that aren't obvious at first glance. \
                          The tutorial takes just a few minutes.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("◀ Stay in Tutorial").clicked() {
                        cancel = true;
                    }
                    if ui.button("Skip").clicked() {
                        skip = true;
                    }
                });
            });
        if cancel {
            state.skip_confirm_phase = 0;
        }
        if skip {
            state.skip_confirm_phase = 2;
        }
        return false;
    }

    if state.skip_confirm_phase == 2 {
        let mut cancel = false;
        let mut skip = false;
        egui::Window::new("Are You Sure?")
            .id(egui::Id::new("skip_confirm_2"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(420.0)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 80),
                    "Are you absolutely sure?");
                ui.add_space(8.0);
                ui.label("You can always restart the tutorial from ⚙ Settings → \
                          User Experience → 'Restart Tutorial'.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("◀ Stay").clicked() {
                        cancel = true;
                    }
                    if ui.button("Really Skip").clicked() {
                        skip = true;
                    }
                });
            });
        if cancel {
            state.skip_confirm_phase = 0;
        }
        if skip {
            dismissed = true;
            state.dismiss();
            if let Ok(mut s) = shared.try_lock() {
                s.config.tutorial_seen = true;
                s.config.user_level = state.level.to_str().to_string();
                s.config.save();
            }
        }
        return dismissed;
    }

    // Level selector (shown before tutorial steps)
    if !state.level_chosen {
        egui::Window::new("🎓 Welcome to EZ-SDR!")
            .id(egui::Id::new("tutorial_level_selector"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(520.0)
            .default_height(320.0)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("What's your experience level?")
                        .size(18.0).strong());
                    ui.add_space(4.0);
                    ui.label("This tailors the UI and tutorial to your needs. \
                              You can change it anytime in Settings.");
                    ui.add_space(12.0);

                    let levels = UserLevel::levels();
                    let mut level_idx = state.level as usize;
                    let slider_resp = ui.add(
                        egui::Slider::new(&mut level_idx, 0..=3)
                            .step_by(1.0)
                            .show_value(false)
                            .text("")
                            .custom_formatter(|_, _| String::new())
                    );
                    _ = slider_resp;

                    let sel = &levels[level_idx];
                    // Show tick labels in a horizontal row
                    let level_colors = [
                        egui::Color32::from_rgb(120, 200, 120),
                        egui::Color32::from_rgb(200, 200, 80),
                        egui::Color32::from_rgb(200, 150, 80),
                        egui::Color32::from_rgb(200, 100, 100),
                    ];
                    ui.columns(4, |cols| {
                        for (i, (col, lv)) in cols.iter_mut().zip(levels.iter()).enumerate() {
                            col.vertical_centered(|ui| {
                                let color = if i == level_idx { level_colors[i] } else { egui::Color32::GRAY };
                                ui.colored_label(color, egui::RichText::new(lv.label()).size(14.0).strong());
                                ui.colored_label(egui::Color32::from_gray(160),
                                    egui::RichText::new(lv.description()).size(10.0));
                            });
                        }
                    });

                    state.level = levels[level_idx];

                    ui.add_space(16.0);
                    if ui.add(egui::Button::new(egui::RichText::new(format!(
                        "🚀 Start as {} — {}",
                        sel.label(),
                        sel.description()
                    )).size(15.0)).min_size(egui::vec2(300.0, 36.0))).clicked() {
                        state.level_chosen = true;
                        state.step = 0;
                        // Save level immediately
                        if let Ok(mut s) = shared.try_lock() {
                            s.config.user_level = state.level.to_str().to_string();
                            s.config.save();
                        }
                    }
                });
            });
        return false;
    }

    // Tutorial step display
    let steps = steps_for_level(state.level);
    if state.step >= steps.len() {
        dismissed = true;
        state.dismiss();
        if let Ok(mut s) = shared.try_lock() {
            s.config.tutorial_seen = true;
            s.config.user_level = state.level.to_str().to_string();
            s.config.save();
        }
        return dismissed;
    }

    let step = &steps[state.step];

    // Set highlight and tab
    if let Some(tab) = &step.tab {
        state.tab_to_open = Some(tab.clone());
    }
    state.highlight_target = step.highlight.map(|s| s.to_string());

    egui::Window::new(format!("🎓 Tutorial — Step {}/{}", state.step + 1, steps.len()))
        .id(egui::Id::new("tutorial_step"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(520.0)
        .default_height(400.0)
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            // Progress dots
            ui.horizontal(|ui| {
                for i in 0..steps.len() {
                    if i == state.step {
                        ui.colored_label(egui::Color32::from_rgb(100, 200, 255), "●");
                    } else if i < state.step {
                        ui.colored_label(egui::Color32::from_gray(100), "○");
                    } else {
                        ui.colored_label(egui::Color32::from_gray(60), "○");
                    }
                }
            });

            ui.add_space(8.0);
            ui.label(egui::RichText::new(step.title).size(18.0).strong());
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(step.body).size(14.0));
                });

            ui.add_space(8.0);

            // Action button if present
            if let Some((btn_label, action)) = &step.action {
                if ui.add(egui::Button::new(egui::RichText::new(*btn_label).size(14.0))
                    .min_size(egui::vec2(200.0, 28.0))).clicked()
                {
                    match action {
                        TutorialAction::TuneFreq(hz) => {
                            if let Ok(mut s) = shared.try_lock() {
                                s.source.frequency_hz = *hz;
                                s.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                                s.source.gain_db = 40.0;
                                s.audio_running = true;
                                s.lpf_cutoff = 15000.0;
                                s.spectrum.zoom_reset();
                            }
                        }
                        TutorialAction::SetDemod(_mode) => {}
                        TutorialAction::StartAudio => {
                            if let Ok(mut s) = shared.try_lock() {
                                s.audio_running = true;
                            }
                        }
                        TutorialAction::OpenSettings => {
                            state.tab_to_open = Some(Tab::Settings);
                        }
                    }
                }
                ui.add_space(4.0);
            }

            // Navigation buttons
            ui.separator();
            ui.horizontal(|ui| {
                if state.step > 0 {
                    if ui.button("◀ Previous").clicked() {
                        state.step -= 1;
                        state.skip_confirm_phase = 0;
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new("◀ Previous"));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⏭ Skip Tutorial").clicked() {
                        state.skip_confirm_phase = 1;
                    }
                    if state.step + 1 < steps.len() {
                        if ui.button("Next ▶").clicked() {
                            state.step += 1;
                        }
                    } else {
                        if ui.button("✅ Finish").clicked() {
                            dismissed = true;
                            state.dismiss();
                            if let Ok(mut s) = shared.try_lock() {
                                s.config.tutorial_seen = true;
                                s.config.user_level = state.level.to_str().to_string();
                                s.config.save();
                            }
                        }
                    }
                });
            });
        });

    dismissed
}
