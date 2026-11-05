#version 300 es
precision highp float;
// GLSL port of Lunalyr's music shaders from adc801b. Licence notices in Web/licenses.
out vec4 outColor;

float ramp(float low, float high, float value) {
    float t = clamp((value - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

struct VizUniforms {
    vec2 resolution;
    float time;
    float dt;
    float bass;
    float mid;
    float treble;
    float level;
    float beat;
    float sensitivity;
    float speed;
    float palette;
    float p[16];
    float subBass;
    float lowMid;
    float highMid;
    float presence;
    float air;
    float trebleBeat;
    float flux;
    float spectralCentre;
    float beatHit;        // 1 on the frame a kick was detected, else 0
    float trebleHit;      // 1 on the frame a hi-hat/snare transient landed
    float beatCount;      // kicks counted since the source started (= kickOnsets)
    float frame;          // frames rendered since launch
    float reduceMotion;   // 1 when the system Reduce Motion setting is on
    float intro;          // 0 → 1 ramp after this visualizer was built
    // Sound types at the displayed time (SoundTypeDetector; see VisualizerUniforms.swift).
    float soundKick;      // kick envelope 0..1 (attack 10 ms, release 0.26 s; × Punch; calm: slow, ≤ 0.5)
    float soundSnare;     // snare/clap envelope 0..1 (release 0.22 s)
    float soundHat;       // hi-hat envelope 0..1 (release 0.15 s)
    float soundBass;      // sustained bass line 0..1 (kick tails held out)
    float soundVocal;     // lead voice / melodic lead 0..1
    float soundPad;       // sustained harmonic bed 0..1
    float kickOnsets;     // +1 per kick; integer-valued, wraps at 2^24 → uint(u.kickOnsets)
    float snareOnsets;
    float hatOnsets;
    float kickAge;        // seconds since the newest kick at the displayed time (60 = none)
    float snareAge;
    float hatAge;
};

// Every scene returns through this. Buffer 5 holds the photosensitivity
// limiter's gain (ExposureLimiter.metal): 1 unless Reduce Motion or Dim
// Flashing Lights is on and the mean brightness is rising too fast.
vec4 limitedExposure(vec3 colour, float exposure) {
    return vec4(colour * exposure, 1.0);
}

// Colour richness for scenes that are already in their final colours: the
// same luma/chroma mix the older scenes apply before tone mapping.
vec3 richness(vec3 colour, float amount) {
    float luma = dot(colour, vec3(0.299, 0.587, 0.114));
    return max(mix(vec3(luma), colour, amount), 0.0);
}

// Recolour the surface while keeping the shader's light and shadow. The
// spatial ramp gives separate parts of each ribbon, field or sphere their own ink.
vec3 albumTint(vec3 scene, vec2 uv, vec4 stops[5]) {
    if (stops[0].w == 0.0) return scene;
    const vec3 weights = vec3(0.30, 0.59, 0.11);
    float luma = dot(clamp(scene, 0.0, 1.0), weights);
    float position = clamp(0.66 * uv.x + 0.34 * (1.0 - uv.y), 0.0, 1.0) * 4.0;
    int segment = min(3, int(position));
    vec3 color = mix(stops[segment].rgb, stops[segment + 1].rgb, position - float(segment));
    vec3 chroma = (color - dot(color, weights)) * stops[0].w;
    float low = min(chroma.r, min(chroma.g, chroma.b));
    float high = max(chroma.r, max(chroma.g, chroma.b));
    float scale = min(1.0, min(luma / max(0.0001, -low), (1.0 - luma) / max(0.0001, high)));
    return clamp(luma + chroma * scale, 0.0, 1.0);
}
uniform VizUniforms u;
uniform vec4 album[5];
uniform float bands[64];
uniform float wave[256];
uniform float exposure;

// ---------------------------------------------------------------- helpers

// Cosine palette (Íñigo Quílez): a + b*cos(2π(c*t + d)) per channel maps any
// scalar t to a smooth cyclic gradient. `which` is u.palette (0 Prism,
// 1 Ember, 2 Ocean, 3 Violet, 4 Mono).
vec3 cosPalette(float t, float which) {
    vec3 a, b, c, d;
    int p = int(which + 0.5);
    if (p == 1)      { a = vec3(0.55, 0.25, 0.12); b = vec3(0.45, 0.35, 0.20); c = vec3(1.0); d = vec3(0.00, 0.12, 0.25); } // Ember
    else if (p == 2) { a = vec3(0.10, 0.35, 0.45); b = vec3(0.25, 0.35, 0.45); c = vec3(1.0); d = vec3(0.00, 0.10, 0.20); } // Ocean
    else if (p == 3) { a = vec3(0.40, 0.20, 0.50); b = vec3(0.50, 0.30, 0.50); c = vec3(1.0); d = vec3(0.80, 0.90, 0.30); } // Violet
    else if (p == 4) { a = vec3(0.60);             b = vec3(0.40);             c = vec3(1.0); d = vec3(0.0);              } // Mono
    else             { a = vec3(0.50);             b = vec3(0.50);             c = vec3(1.0); d = vec3(0.00, 0.33, 0.67); } // Prism
    return a + b * cos(6.28318 * (c * t + d));
}

// Samples the 64-band spectrum at a continuous position x in 0...1, linearly
// interpolating between neighboring bands so sweeps look smooth.
float bandAt(float bands[64], float x) {
    float f = clamp(x, 0.0, 0.9999) * 63.0;
    int i = int(f);
    return mix(bands[i], bands[min(i + 1, 63)], fract(f));
}

// Soft knee: retain variation through the full sensitivity range.
float visualEnergy(float value, float sensitivity) {
    float x = max(value, 0.0) * sqrt(max(sensitivity, 0.3));
    return x / (0.65 + x);
}

// The horizontal coordinate is a fixed logarithmic frequency axis. Smooth
// neighbours without scrolling the bins or turning them into sharp bars.
float spectrumAlongRibbon(float bands[64], float x, float sensitivity) {
    float energy = (bandAt(bands, x - 1.0 / 63.0) + 2.0 * bandAt(bands, x)
                  + bandAt(bands, x + 1.0 / 63.0)) * 0.25;
    return visualEnergy(energy, sensitivity);
}

// Samples the 256-point waveform at x in 0...1, averaged with its neighbors
// to soften single-sample spikes.
float waveAt(float wave[256], float x) {
    int i = int(clamp(x, 0.0, 0.9999) * 255.0);
    int a = max(i - 1, 0), b = min(i + 1, 255);
    return (wave[a] + wave[i] + wave[b]) / 3.0;
}

// ---------------------------------------------------------------- 2D noise

// Cheap 2D → pseudo-random hash in 0...1. Not statistically great, but fast
// and stable per input — used to scatter stars/particles deterministically.
float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

// Value noise: random values at integer grid points, smoothly interpolated
// between them. Gives soft, organic variation rather than white noise.
float vnoise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Fractal Brownian motion: several octaves of noise, each at double the
// frequency and half the amplitude of the last. The classic recipe for
// clouds, smoke, and haze.
float fbm(vec2 p) {
    float v = 0.0, amp = 0.5;
    for (int i = 0; i < 4; i++) {
        v += amp * vnoise(p);
        p *= 2.03;
        amp *= 0.5;
    }
    return v;
}

// ---------------------------------------------------------------- 3D noise

// 3D value-noise stack standing in for Shadertoy's RGBA-noise texture (the
// Spectrum Tunnel's rock, steam and glitter are carved from it).
float hash31(vec3 p) {
    p = fract(p * vec3(443.897, 441.423, 437.195));
    p += dot(p, p.yzx + 19.19);
    return fract((p.x + p.y) * p.z);
}

float vnoise3(vec3 x) {
    vec3 i = floor(x), f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    float n000 = hash31(i);
    float n100 = hash31(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash31(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash31(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash31(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash31(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash31(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash31(i + vec3(1.0, 1.0, 1.0));
    return mix(mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
               mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y), f.z);
}

// Balanced three-octave fbm, ~0…0.875 — used for color/steam variation.
float fbm3(vec3 p) {
    float v = 0.0, amp = 0.5;
    for (int i = 0; i < 3; i++) {
        v += amp * vnoise3(p);
        p *= 2.03;
        amp *= 0.5;
    }
    return v;
}

// Demoscene-style fbm (after "Sailing Beyond"): heavily weighted first
// octave with frequency *tripling* per octave, ~0…0.33. Shaped with a power
// curve at the call site it makes terraced, cliff-like billows rather than
// soft lumps.
float fbmSharp(vec3 p) {
    float r = 0.0, w = 1.0, s = 1.0;
    for (int i = 0; i < 3; i++) {
        w *= 0.25;
        s *= 3.0;
        r += w * vnoise3(p * s);
    }
    return r;
}

vec4 colourStrataFragment(vec2 texturePoint) {
    vec2 uv = texturePoint;
    float x = uv.x - 0.5;
    float y = uv.y;
    float time = u.time * u.speed;
    vec3 energy = max(vec3(u.bass, u.mid, u.treble), vec3(0.0)) * u.sensitivity;
    vec3 response = energy / (0.56 + energy);

    // Each register owns a different edge. Their boundaries share a slow
    // drift, with enough overlap to read as a continuous dyed horizon.
    float lowEdge = 0.20 + 0.21 * response.x + 0.095 * sin(x * 3.2 + time * 0.38)
        + 0.031 * cos(x * 8.0 - time * 0.57);
    float middleEdge = 0.50 + 0.065 * sin(x * 4.7 - time * 0.29)
        + (0.018 + 0.14 * response.y) * cos(x * 5.6 + time * 0.47);
    float topEdge = 0.73 + 0.09 * cos(x * 3.5 + time * 0.36)
        + 0.026 * sin(x * 7.3 - time * 0.61);
    float topFeather = 0.065 + 0.19 * response.z;

    float lower = 1.0 - ramp(lowEdge - 0.105, lowEdge + 0.13, y);
    float middle = 1.0 - ramp(middleEdge - 0.084, middleEdge + 0.108, y);
    float upper = ramp(topEdge - topFeather, topEdge + topFeather, y);
    float middleLight = middle * (1.0 - lower);
    float edgeGlow = 1.0 - ramp(0.025, 0.19, abs(y - middleEdge));
    vec2 centre = vec2(x * 1.65, (y - 0.5) * 1.55);
    float contrast = mix(0.61, 1.0, ramp(0.06, 0.72, length(centre)));

    vec3 colour = vec3(0.067, 0.088, 0.135);
    colour += vec3(0.22, 0.092, 0.21) * lower;
    colour += vec3(0.20, 0.25, 0.30) * middleLight;
    colour += vec3(0.080, 0.17, 0.22) * upper;
    colour += vec3(0.088, 0.091, 0.10) * edgeGlow;
    // Colour richness applies with or without artwork (see Star Trails).
    return limitedExposure(albumTint(richness(colour * contrast, u.p[15]), uv, album), exposure);
}


float inkCoverage(vec2 point, vec2 centre, vec2 radii, float slant) {
    vec2 local = point - centre;
    vec2 bent = vec2(local.x + slant * local.y, local.y + 0.15 * local.x * local.x);
    float edge = length(bent / radii);
    return 1.0 - ramp(0.89, 1.10, edge);
}

vec3 inkColour(vec3 colour, float vividness) {
    float grey = dot(colour, vec3(0.30, 0.59, 0.11));
    return clamp(mix(vec3(grey), colour, vividness), 0.0, 1.0);
}

vec4 overprintFragment(vec2 texturePoint) {
    vec2 point = texturePoint - 0.5;
    float time = u.time * u.speed;
    vec3 energy = max(vec3(u.bass, u.mid, u.treble), vec3(0.0)) * u.sensitivity;
    vec3 response = energy / (0.54 + energy);

    float first = inkCoverage(point, vec2(-0.37 + 0.056 * sin(time * 0.49), -0.24),
        vec2(0.58 + 0.17 * response.x, 0.38 + 0.21 * response.x), -0.50);
    float second = inkCoverage(point, vec2(0.38 - 0.30 * response.y, 0.18 + 0.08 * cos(time * 0.57)),
        vec2(0.48, 0.43), 0.45);
    float third = inkCoverage(point, vec2(0.10 + 0.055 * cos(time * 0.34), -0.55),
        vec2(0.78, 0.40), 0.18);

    // Register the ink plate colours to the artwork and let overlaps darken
    // softly, like ink on coloured stock.
    bool hasAlbum = album[0].w > 0.0;
    // Colour richness also applies to the built-in inks when there is no
    // artwork (album[0].w carries the same value when there is).
    float vividness = u.p[15];
    vec3 stock = hasAlbum ? album[2].rgb : vec3(0.36, 0.32, 0.62);
    vec3 left = inkColour(hasAlbum ? album[0].rgb : vec3(0.42, 0.30, 0.71), vividness);
    vec3 right = inkColour(hasAlbum ? album[4].rgb : vec3(0.22, 0.60, 0.62), vividness);
    vec3 bottom = inkColour(hasAlbum ? album[1].rgb : vec3(0.69, 0.34, 0.46), vividness);
    vec3 colour = vec3(0.026) + stock * 0.11;
    colour += first * left * 0.33 + second * right * 0.31 + third * bottom * 0.16;
    colour *= 1.0 - 0.18 * first * second;

    // A stationary dot screen sits near the plate edges. Treble strengthens
    // the impression without moving the dots or changing the whole field.
    float aspect = u.resolution.x / max(1.0, u.resolution.y);
    vec2 paperPoint = vec2(point.x * aspect, point.y);
    vec2 screen = vec2(0.94 * paperPoint.x - 0.34 * paperPoint.y,
                          0.34 * paperPoint.x + 0.94 * paperPoint.y);
    float dotDistance = length(fract(screen * 76.0) - 0.5);
    float antialias = clamp(fwidth(dotDistance) * 0.65, 0.022, 0.20);
    float dots = 1.0 - ramp(0.29 - antialias, 0.29 + antialias, dotDistance);
    float edges = max(4.0 * first * (1.0 - first), 4.0 * second * (1.0 - second));
    colour *= 1.0 - edges * dots * (0.10 + 0.45 * response.z);

    float centreShade = mix(0.63, 1.0, ramp(0.06, 0.60, length(point)));
    return limitedExposure(colour * centreShade, exposure);
}

// Aurora Curtains. Lunalyr's own scene rather than part of the Synesthia
// import; its palette and spectrum helpers come from VisualizerShaderCommon.h,
// which carries that notice.


// GPU Aurora's flowing material, with a frequency axis along every curtain.
vec4 curtainAuroraFragment(vec2 texturePoint) {
    vec2 uv = texturePoint;
    float local = spectrumAlongRibbon(bands, uv.x, u.sensitivity);
    float glow = 0.0;
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float phase = u.time * u.speed * (0.5 + fi * 0.14) + fi * 2.1;
        // Each line favours a different register while keeping the complete
        // bass-left/treble-right axis. Notes affect their own section, never
        // a whole-frame translation or a global brightness pulse.
        float emphasis = 0.45 + 0.55 * exp(-pow((uv.x - fi / 4.0) * 2.8, 2.0));
        float response = local * emphasis;
        float centre = 0.5 + sin(uv.x * 2.3 + phase) * 0.16
            + sin(uv.x * 5.1 - phase * 0.7) * 0.06
            + sin(uv.x * (5.0 + fi) + phase + fi) * response * 0.17;
        float thickness = 0.018 + 0.009 * (1.0 + sin(phase * 0.8)) + 0.045 * response;
        float band = thickness / (abs(uv.y - centre) + thickness * 0.55);
        glow += band * (0.15 + 0.10 * sin(phase) + 1.15 * response);
    }
    glow = clamp(glow * 0.28, 0.0, 1.7);
    vec3 colour = mix(cosPalette(0.16, u.palette), cosPalette(0.7, u.palette), clamp(uv.y * 1.2, 0.0, 1.0));
    vec3 col = colour * glow;
    float luma = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(luma), col, u.p[15]), 0.0);
    col = col / (1.0 + col);
    return limitedExposure(albumTint(col, texturePoint, album), exposure);
}

// Ported from Fosfora Aurora, copyright (c) 2025-2026 Kevin Raymond, MIT.
// Original WGSL, pinned revision and local adaptations: ThirdParty/Fosfora.
// Complete notice is bundled as Resources/Fosfora-LICENSE.txt.

// The shared value noise in VisualizerShaderCommon.h replaces fosfora_noise2.


// Fosfora's option slots: the 60-150 Hz and 400-1200 Hz registers, so no
// ribbon contains its neighbours' frequencies (u.bass and u.mid are broad).
const int kFosforaBass = 3;
const int kFosforaMid = 4;

vec4 fosforaAuroraFragment(vec2 texturePoint) {
    vec2 uv = texturePoint;
    vec2 p = (uv - 0.5) * vec2(u.resolution.x / max(u.resolution.y, 1.0), 1.0);
    float t = u.time * u.speed;
    float travel = t * 1.3; // The containing clock already integrates Motion speed.
    float registers[7] = float[7](u.subBass, u.p[kFosforaBass], u.lowMid, u.p[kFosforaMid],
        u.highMid, u.presence, u.air);
    vec3 colours[7] = vec3[7](vec3(0.8, 0.1, 0.2), vec3(1.0, 0.4, 0.1), vec3(1.0, 0.8, 0.2),
        vec3(0.2, 1.0, 0.3), vec3(0.1, 0.7, 1.0), vec3(0.3, 0.3, 1.0), vec3(0.7, 0.2, 1.0));
    vec3 col = vec3(0.002, 0.003, 0.008);
    for (int i = 0; i < 7; i++) {
        float fi = float(i);
        float bandValue = visualEnergy(registers[i], u.sensitivity);
        float section = spectrumAlongRibbon(bands, (fi + uv.x) / 7.0, u.sensitivity);
        float phase = travel * (0.8 + fi * 0.1);
        float curve = sin(p.x * (2.0 + fi * 0.5) + phase) * 0.05
            + sin(p.x * (2.0 + fi * 0.5) * 2.3 + phase * 0.7) * 0.03;
        float ripple = (vnoise(vec2(p.x * 3.0 + phase * 0.5, fi * 2.0 + t * 0.1)) - 0.5) * 0.08;
        float dy = p.y - (fi - 3.0) / 7.0 * 0.85 - curve - ripple
            - sin(uv.x * 9.0 + phase) * section * 0.045;
        float width = 0.014 * (1.0 + 1.5 * bandValue + section);
        float glow = exp(-dy * dy / (width * width));
        col += colours[i] * glow * (0.06 + 1.7 * bandValue + 0.8 * section);
    }
    // Keep Fosfora's frequency curtains; omit its global onset/beat flashes
    // and shimmer so the material stays calm behind lyrics.
    float luma = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(luma), col, u.p[15]), 0.0);
    return limitedExposure(albumTint(col / (1.0 + col), texturePoint, album), exposure);
}

// Adapted from Synesthia by Jon Jaques, MIT licence.
// Pinned source and local changes: ThirdParty/Synesthia/SOURCE.md.
// Copyright (c) 2026 Jon Jaques. See Resources/Synesthia-LICENSE.txt.

// Spectral Aurora and Silk Ribbons both draw with auroraFragment: Silk
// Ribbons is the same renderer with restrained settings (its palette and
// option slots are set in AudioVisualizerView.Coordinator.uniforms).


// ---------------------------------------------------------------- Aurora
//
// A night-sky scene built in layers, back to front: gradient sky, a star
// field that flashes with the percussion, haze, ground fog, and up to eight
// aurora ribbons — one per instrument group, sub-bass at the bottom of the
// sky through air at the top, each with its own motion personality and each
// shaded along its length by the fine spectrum inside its own range.

// The aurora's option slots, in descriptor order.
const int kAuroraRibbons = 0;
const int kAuroraHeight = 1;
const int kAuroraStars = 2;

// One layer of the aurora's star field: a jittered grid where ~10% of cells
// hold a star, each with its own size, tint, twinkle phase, and flash
// gates. The air band sets the ambient twinkle level; every hi-hat lights a
// different constellation and every kick a different (larger) one, so the
// sky visibly plays the drum kit.
vec3 auroraStarLayer(vec2 sp, float scale, VizUniforms u) {
    vec2 g = sp * scale;
    vec2 id = floor(g);
    float h1 = hash21(id);
    if (h1 < 0.90) { return vec3(0.0); }
    vec2 offs = vec2(hash21(id + 7.1), hash21(id + 3.7)) - 0.5;
    vec2 f = fract(g) - 0.5 - offs * 0.7;
    float d = length(f);
    float h2 = hash21(id + 11.3);
    float h3 = hash21(id + 17.9);
    float h4 = hash21(id + 23.7);
    float core = exp(-d * d * (150.0 - 90.0 * h2));
    // 4-point diffraction cross on the brightest few
    float sparkle = (exp(-abs(f.x) * 42.0) * exp(-abs(f.y) * 8.0)
                   + exp(-abs(f.y) * 42.0) * exp(-abs(f.x) * 8.0))
                  * max(h1 - 0.975, 0.0) * 40.0;
    float twinkle = 0.45 + 0.55 * sin(u.time * (2.0 + 4.0 * h3) + h3 * 40.0);
    // two independent, periodically reshuffled gates: each hi-hat lights a
    // different constellation, each kick a different (larger) one
    float gateHat = step(0.55, fract(h4 + floor(u.time * 1.5) * 0.618));
    float gateKick = step(0.45, fract(h4 * 3.7 + floor(u.time * 2.0) * 0.618));
    float amp = (0.30 + 0.70 * twinkle) * (0.30 + 0.70 * clamp(u.air * u.sensitivity, 0.0, 1.0))
              + gateHat * u.trebleBeat * 1.3
              + gateKick * u.beat * 1.2;
    vec3 tint = mix(vec3(1.0), cosPalette(h2, u.palette), 0.35);
    return tint * (core + sparkle) * amp * (h1 - 0.90) * 10.0;
}

vec4 auroraFragment(vec2 texturePoint) {
    vec2 uv = texturePoint;
    float aspect = u.resolution.x / max(u.resolution.y, 1.0);
    vec2 suv = vec2(uv.x * aspect, uv.y);
    float starsAmt = u.p[kAuroraStars];

    // Night-sky gradient, tinted faintly by the palette and the spectral
    // spectralCentre so the sky's mood follows the music's brightness.
    vec3 col = mix(vec3(0.008, 0.010, 0.026), vec3(0.016, 0.026, 0.050), uv.y);
    col += cosPalette(0.62 + u.spectralCentre * 0.3, u.palette) * 0.012;

    // the star field (see auroraStarLayer): three depths of sky
    vec3 stars = auroraStarLayer(suv, 40.0, u)
                 + auroraStarLayer(suv + 5.3, 75.0, u) * 0.6
                 + auroraStarLayer(suv + 9.1, 140.0, u) * 0.35;
    col += stars * starsAmt * ramp(0.16, 0.50, uv.y);

    // drifting haze that brightens with the low end, tinted by brightness
    float haze = fbm(uv * 3.0 + vec2(u.time * 0.05 * u.speed, 0.0));
    col += cosPalette(0.6 + u.spectralCentre * 0.3 + u.time * 0.01, u.palette) * haze
         * (0.03 + 0.10 * u.bass * u.sensitivity);

    // ground fog breathing with the sub-bass
    float fog = fbm(vec2(uv.x * 4.0 + u.time * 0.08 * u.speed, uv.y * 9.0));
    col += cosPalette(0.05, u.palette) * fog * ramp(0.30, 0.0, uv.y)
         * (0.04 + 0.30 * u.subBass * u.sensitivity);

    int layers = clamp(int(u.p[kAuroraRibbons] + 0.5), 2, 8);
    float height = u.p[kAuroraHeight];

    // One ribbon per instrument group, low to high: sub-bass at the bottom
    // of the sky through air at the top. Each has its own motion
    // personality — lows are fat and slow and jump on the kick, mids carry
    // the waveform (melody and vocals) and answer onsets, highs are thin
    // and fast and shiver on hi-hats — and each is shaded along its length
    // by the fine spectrum *inside* its own frequency range, so every
    // ribbon is a small equalizer for its register.
    float comps[8] = float[8](u.subBass, u.bass, u.lowMid, u.mid,
                      u.highMid, u.presence, u.treble, u.air);
    for (int i = 0; i < 8; i++) {
        if (i >= layers) break;
        float fi = float(i) / float(max(layers - 1, 1));
        int cIdx = int(fi * 7.0 + 0.5);
        float reg = float(cIdx) / 7.0;
        float compE = visualEnergy(comps[cIdx], u.sensitivity);
        // the fine spectrum inside this register, spread across the screen
        float localE = spectrumAlongRibbon(bands, (float(cIdx) + clamp(uv.x, 0.0, 1.0)) / 8.0, u.sensitivity);
        // transient personality: lows jump on the kick, mids on onsets,
        // highs shiver on hi-hat transients
        float transient = u.beat * (1.0 - ramp(0.15, 0.45, reg))
                        + u.flux * exp(-pow((reg - 0.5) * 3.5, 2.0))
                        + u.trebleBeat * ramp(0.55, 0.85, reg);
        // mids carry the waveform; lows and highs mostly ignore it
        float wWeight = exp(-pow((reg - 0.5) * 2.4, 2.0));
        float w = (localE - 0.3) * 0.8; // Each section bends with frequencies inside this ribbon's own register.
        // low ribbons undulate slowly and widely, high ribbons shiver fast
        float sway = (0.065 * (1.0 - reg) + 0.018 + 0.055 * compE)
                   * sin(uv.x * (4.0 + reg * 14.0)
                         + u.time * u.speed * (0.4 + 1.6 * reg) * (0.6 + fi * 0.8));
        float ripple = 0.014 * reg
                     * sin(uv.x * (30.0 + 60.0 * reg) + u.time * u.speed * (2.0 + 3.0 * reg))
                     * clamp(u.highMid * u.sensitivity, 0.0, 1.2);
        float yC = 0.5
                 + (fi - 0.5) * 0.58
                 + w * height * wWeight * (0.5 + 0.8 * compE)
                 + sway + ripple
                 + 0.04 * transient * (1.0 - 0.5 * reg);
        // thickness: fat lows, thin highs, detailed by the local spectrum,
        // puffed by this register's transients
        float thickness = (0.009 + 0.045 * compE) * (1.6 - 1.1 * reg)
                        * (0.6 + 0.9 * localE) * (1.0 + 0.5 * transient);
        // t/(|dy|+t) is a soft glow profile: 1 on the center line, falling
        // off hyperbolically; the pow sharpens its core.
        float glowR = thickness / (abs(uv.y - yC) + thickness);
        glowR = pow(glowR, 1.4);
        // vertical curtain rays driven by the presence band
        float rays = 0.85 + 0.30 * sin(uv.x * (90.0 + fi * 20.0) + u.time * u.speed * (1.5 + fi))
                   * clamp(u.presence * u.sensitivity * 1.5, 0.0, 1.0);
        col += cosPalette(fi * 0.85 + u.spectralCentre * 0.2 + u.time * 0.015, u.palette)
             * glowR * rays * (0.10 + 0.85 * compE + 0.30 * localE) * (1.0 + 0.45 * transient);
    }

    // beat lift + onset shimmer
    col *= 1.0 + 0.18 * u.beat + 0.10 * u.flux;

    // vibrance, Reinhard tone mapping, and a gentle S-curve — the same
    // finishing chain as the tunnel, so the ribbons saturate instead of
    // washing out
    float luma = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(luma), col, 1.35 * u.p[15]), 0.0);
    col = col / (1.0 + col);
    col = col * col * (3.0 - 2.0 * col);
    return limitedExposure(albumTint(col, texturePoint, album), exposure);
}

// Adapted from Synesthia by Jon Jaques, MIT licence.
// Pinned source and local changes: ThirdParty/Synesthia/SOURCE.md.
// Copyright (c) 2026 Jon Jaques. See Resources/Synesthia-LICENSE.txt.


// ---------------------------------------------------------------- Spectrum Tunnel
//
// A raymarched flight through a winding cave. Unlike the other fragment
// shaders (which fake depth with a polar mapping), this one marches an
// actual 3D signed-distance field: the tunnel is a bore of varying radius
// around a centerline that snakes through space, so bends really occlude —
// the far end disappears behind the curve of the wall and you never see the
// end. The camera rides the centerline. Adapted from two Shadertoy
// references: XtdGR7 (the winding SDF tunnel) and the "Hyper Tunnel" from
// the demo "Sailing Beyond" (crevice lighting from the iteration count,
// power-shaped fbm crags, the volumetric steam pass, and the audio-pumped
// exposure). Their texture lookups are replaced with procedural 3D value
// noise, so no assets are needed.
//
// The audio is carved into the geometry itself: each angular lane of the
// wall reads one slice of the 64-band spectrum (bass at the floor, treble
// overhead, mirror-folded so there is no seam) and loud bands physically
// bulge the rock inward; the bass swells the whole bore; spiral ridges
// corkscrew past at a rate set by the Twist slider with depth from the
// low-mids; the waveform rings the bore along its length; every kick throws
// a wall of light up the tunnel toward the camera; the mids, presence and
// onsets pump the steam; and the scene's exposure breathes with the track's
// overall loudness.

// The tunnel's option slots, in the order TunnelVisualizer.descriptor
// declares them (`u.p[kTunnelGlow]` reads the Glow slider).
const int kTunnelTwist = 0;
const int kTunnelGlow = 1;
const int kTunnelBend = 2;
const int kTunnelPulse = 3;
const int kTunnelRipple = 4;
const int kTunnelFog = 5;

// hash31, vnoise3, fbm3 and fbmSharp (the 3D value noise standing in for
// Shadertoy's noise texture) are in VisualizerShaderCommon.h.

// Where the tunnel's centerline sits at distance z: layered incommensurate
// sines, so the path winds forever without repeating. The amplitude is on
// the order of the bore radius — that is what makes curves actually occlude
// instead of just leaning.
vec2 tunnelCenter(float z, float bend) {
    return vec2(sin(z * 0.31) + 0.6 * sin(z * 0.17 + 4.0),
                  cos(z * 0.23) + 0.6 * sin(z * 0.13 + 1.0)) * 0.55 * bend;
}

// Mirror-folded angle around the bore (0 = floor, 1 = ceiling): band 0 and
// band 63 never sit next to each other, so the spectrum wall has no seam.
// The +0.25 phase puts the fold's minimum at angle -π/2 (straight down), so
// the bass really is at the floor and the treble overhead as documented —
// +0.75 (the obvious other choice) flips the whole spectrum upside down.
float tunnelFold(float angle) {
    return 1.0 - abs(fract(angle / 6.28318 + 0.25) * 2.0 - 1.0);
}

// The signed-distance field: positive inside the air of the tunnel, zero at
// the wall. Everything audible deforms it — see the section comment.
float tunnelMap(vec3 pos, VizUniforms u,
                       float bands[64], float wave[256]) {
    vec2 xy = pos.xy - tunnelCenter(pos.z, u.p[kTunnelBend]);
    float angle = atan(xy.y, xy.x);
    float e = visualEnergy(bandAt(bands, tunnelFold(angle)), u.sensitivity);
    // spiral ridges spinning past: rate from the Twist slider, depth from
    // the low-mids
    float arms = sin(angle * 3.0 - pos.z * 0.4 + u.time * u.speed * (0.5 + 1.0 * u.p[kTunnelTwist]))
               * (0.05 + 0.25 * visualEnergy(u.lowMid, u.sensitivity));
    // power-shaped noise carves terraced crags into the rock
    float crags = pow(fbmSharp(pos * 0.55) * 3.3, 1.3);
    // the bore's own slow, irregular breathing: a drifting noise field, so
    // caverns open and close instead of repeating like a sine would
    float cavern = vnoise3(vec3(1.7, 2.9, pos.z * 0.06) + vec3(0.0, u.time * 0.05, 0.0)) - 0.5;
    float r = 1.1
        + 0.5 * cavern
        + 0.45 * visualEnergy(u.bass, u.sensitivity)         // bass opens the bore without clipping at high gain
        + u.p[kTunnelRipple] * (0.42 * crags - 0.30                       // crags (Ripple slider)
                  + 0.08 * visualEnergy(waveAt(wave, fract(pos.z * 0.10)), u.sensitivity))
        + arms
        - 0.48 * e * e;                                     // each angular frequency lane shapes its wall
    // Floor the bore radius so a worst-case pile-up of carvings can never
    // close the tunnel over the camera (which rides the centerline).
    r = max(r, 0.30);
    return r - length(xy);
}

vec4 tunnelFragment(vec2 texturePoint) {
    float glow = u.p[kTunnelGlow];
    float pulse = u.p[kTunnelPulse];
    float fogAmt = u.p[kTunnelFog];

    // Square-space pixel coordinates (y half-height = 1).
    vec2 suv = texturePoint * 2.0 - 1.0;
    suv.x *= u.resolution.x / max(u.resolution.y, 1.0);

    // The camera rides the centerline, aimed at the centerline further on
    // (that look-ahead is what keeps the mouth centered while the far end
    // swings). The kick surges it forward; a slow continuous roll keeps the
    // flight alive.
    float camZ = u.time * u.speed * 2.2; // Continuous travel; no kick-driven camera jumps.
    vec3 ro = vec3(tunnelCenter(camZ, u.p[kTunnelBend]), camZ);
    vec3 ta = vec3(tunnelCenter(camZ + 2.0, u.p[kTunnelBend]), camZ + 2.0);
    float rollA = u.time * 0.1 + 0.6 * sin(u.time * 0.3);
    vec3 fw = normalize(ta - ro);
    vec3 right = normalize(cross(fw, vec3(sin(rollA), cos(rollA), 0.0)));
    vec3 up = cross(right, fw);
    vec3 rd = normalize(suv.x * right + suv.y * up + 1.4 * fw);

    // Sphere-trace to the wall, counting steps: grazing rays rack up
    // iterations, and that count *is* the lighting — crevices, crags and
    // silhouettes glow with no normal computation at all (the demoscene
    // trick).
    float t = 0.0;
    vec3 p = ro;
    float steps = 0.0;
    for (int i = 0; i < 60; i++) {
        float d = tunnelMap(p, u, bands, wave);
        if (d < 0.0025 || t > 26.0) break;
        t += max(d * 0.8, 0.004);
        p = ro + rd * t;
        steps += 1.0;
    }

    // Recover the wall's slice of the spectrum.
    vec2 xy = p.xy - tunnelCenter(p.z, u.p[kTunnelBend]);
    float fold = tunnelFold(atan(xy.y, xy.x));
    float e = visualEnergy(bandAt(bands, fold), u.sensitivity);

    float crevice = min(0.85, steps / 42.0) + 0.06;

    // Mineral coloring: the same noise family that carved the rock steers
    // the hue on top of the bass→treble floor-to-ceiling gradient, so
    // colored strata run through the walls.
    vec3 wallColor = cosPalette(0.06 + fold * 0.45 + 1.1 * fbm3(p * 0.35)
                                  + u.spectralCentre * 0.30, u.palette);
    vec3 col = wallColor * (0.35 + 3.0 * e * e) * crevice * glow;

    // ribs streaming past with the high-mids
    float segF = fract(p.z * 0.9);
    float rib = ramp(0.10, 0.0, min(segF, 1.0 - segF));
    col += cosPalette(0.5 + fold * 0.3, u.palette) * rib * crevice
         * (0.08 + 1.0 * visualEnergy(u.highMid, u.sensitivity)) * glow;

    // glitter veins answering hi-hats and air: soft pow-shaped noise spots
    // tinted by the palette, twinkling as they stream past (hard hash cells
    // here read as flat gray confetti — never again)
    float sp = vnoise3(p * 13.0 + vec3(0.0, 0.0, u.time * 0.6));
    float glit = pow(max(sp - 0.72, 0.0) * 3.5, 3.0);
    col += cosPalette(0.85 + fold * 0.3 + u.spectralCentre * 0.2, u.palette) * glit
         * (0.5 + 0.5 * sin(u.time * 12.0 + p.z * 3.0))
         * (1.4 * u.trebleBeat + 0.8 * u.air * u.sensitivity) * 2.0;

    // every kick throws a wall of light up the tunnel toward the camera
    // (beat 1 → 0 maps far → here) — the Pulse slider scales it
    float lightZ = camZ + 1.5 + u.beat * 10.0;
    float pulseGlow = exp(-abs(p.z - lightZ) * 0.6) * u.beat * pulse;
    col += cosPalette(0.1 + u.spectralCentre * 0.3, u.palette) * pulseGlow * (1.5 + 1.0 * u.bass);

    // distance dissolves into palette haze pulsing with the low end (Fog
    // slider) — the far end of the cave is never a picture, just glow
    float atten = 1.0 / (1.0 + t * t * 0.010 * fogAmt);
    vec3 fogColor = cosPalette(0.6 + u.spectralCentre * 0.25 + u.time * 0.01, u.palette)
                    * (0.06 + 0.45 * u.bass * u.sensitivity + 0.20 * u.flux);
    col = col * atten + fogColor * (1.0 - atten);

    // volumetric steam (after "Sailing Beyond"): march back from the wall
    // toward the camera stacking noise, cube it, and let the mids, the
    // presence band (vocals) and onsets pump its brightness
    float f = 0.0;
    float distC = t;
    for (int i = 0; i < 16; i++) {
        vec3 sp = ro + rd * distC;
        f += vnoise3(sp * 0.9 + vec3(0.0, 0.0, -u.time * 0.30)) * 0.045;
        distC -= max(t * 0.06, 0.25);
        if (distC < 0.4) break;
    }
    vec3 steamColor = cosPalette(0.55 + u.spectralCentre * 0.3 + u.time * 0.008, u.palette);
    col += steamColor * pow(abs(f * 1.5), 3.0)
         * (0.35 + 1.6 * clamp(u.mid * u.sensitivity, 0.0, 1.0)
            + 1.0 * clamp(u.presence * u.sensitivity, 0.0, 1.2) + 0.8 * u.flux);

    // the whole scene breathes with the track's loudness (the demoscene
    // audio-exposure trick, driven by our smoothed RMS level)
    col *= (0.50 + 0.85 * u.level * u.sensitivity) * (1.0 + 0.20 * u.flux) * 1.6;

    // vignette, then vibrance: push saturation while still HDR, Reinhard
    // tone map (c/(1+c)) so the additive pile-up rolls smoothly into 0...1
    // instead of clipping at white, and finish with a gentle S-curve — the
    // deepened shadows and richer primaries keep the cave from going muddy
    col *= ramp(4.5, 0.8, dot(suv, suv));
    float luma = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(luma), col, 1.5 * u.p[15]), 0.0);
    col = col / (1.0 + col);
    col = col * col * (3.0 - 2.0 * col);
    return limitedExposure(albumTint(col, texturePoint, album), exposure);
}
