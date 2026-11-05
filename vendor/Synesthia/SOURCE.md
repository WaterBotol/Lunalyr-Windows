# Synesthia visualisers

Source: https://github.com/jonjaques/synesthia
Pinned commit: 05144bf0f03eea5cc4af8135e9d71553f1ac9202
Copyright (c) 2026 Jon Jaques. MIT licence, bundled in Sources/Resources/Synesthia-LICENSE.txt.

Reused source: the audio analyser, VizUniforms definition, Aurora and Spectrum Tunnel shaders, and GPU resolution ladder/timer. The render host is adapted to Lunalyr’s scheduled wallpaper frames, existing capture, preferences and desktop windows. No Synesthia icon, wordmark, name branding, recordings or demo music is bundled.

Local changes: Swift 5 compatibility; native PCM decoder input; FFT setup cleanup; bounded sample-rate input; no kick-driven tunnel camera jumps; aurora curvature uses a smoothed spectrum instead of unstable waveform phase; gentler transient scaling and desktop defaults. The Silk Ribbons variation uses the same aurora renderer with restrained settings.

Analyser changes (November 2026): the 64 bands are log-spaced in real Hz (30 Hz to 16 kHz) and integrate a 4× zero-padded spectrum between their edges, instead of forcing at least one 23 Hz bin per band, which had left the low half of the axis linear; the named components and bass/mid/treble are derived from their Hz ranges; the window lasts about 43 ms at every sample rate (2048 samples at 48 kHz, 4096 at 96 kHz) and every envelope uses a time constant in seconds, so the look no longer changes with the device's rate; kicks and hi-hats come from a time-domain onset path on ~5 ms blocks with their own timestamps; and each pass is kept with its capture time so the visuals can follow the output device's latency. The capture path no longer allocates or copies on write.

The imported shader comments retain their procedural-art references. Everything is rendered locally, with no browser or remote assets.
