# Fosfora Aurora

Source: https://github.com/kevinraymond/fosfora
Pinned commit: 2e85d3babca5b7f27426e211e58fe3c831fb667f
Copyright (c) 2025-2026 Kevin Raymond. Reused under the MIT licence option.
The complete licence is bundled in Sources/Resources/Fosfora-LICENSE.txt.

The original assets/shaders/aurora.wgsl and assets/effects/aurora.pfx are retained here for provenance. The seven frequency-curtain equations, colour array and Gaussian glow are ported into fosforaAuroraFragment in Sources/Visualizers/FosforaAurora.metal. The host remains Lunalyr's existing Metal surface, local FFT, capture, frame limit and power policy; no Fosfora executable, engine, assets or audio are bundled.

Local adaptations: existing procedural noise replaces fosfora_noise2; Lunalyr's integrated clock supplies travel; soft-knee sensitivity, per-section spectral bending, desktop spacing, saturation and tone mapping are applied. Global onset/beat flashes and shimmer are omitted. Each ribbon reads its own register of Lunalyr's analyser, so neighbouring ribbons no longer share frequencies: sub-bass 30–60 Hz, bass 60–150 Hz, low-mid 150–400 Hz, mid 400–1200 Hz, high-mid 1.2–3 kHz, presence 3–6 kHz and air 10–16 kHz. This is an adapted native option, not the original application's complete output.
