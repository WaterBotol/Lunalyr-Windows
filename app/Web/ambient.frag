#version 300 es
precision highp float;
// GLSL port of Lunalyr's ambient shaders from adc801b, under the project licence.
out vec4 outColor;
uniform vec2 uSize;
uniform float uTime;
uniform float uFlow;
uniform float uVividness;
uniform vec3 uTintA;
uniform vec3 uTintB;
uniform vec3 uStops[5];

float remainderSigned(float a, float b) { return a - b * trunc(a / b); }
float ramp(float low, float high, float value) {
    float t = clamp((value - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Per-pixel background effects, run on the GPU via SwiftUI's `colorEffect`.
// These do work the Canvas styles cannot: raymarched geometry, escape-time
// fractals and volumetric layering, evaluated for every pixel every frame.
//
// Each takes the album's two dominant colours so the piece is tinted by the
// record rather than arriving in some fixed palette.

vec2 rotate2(vec2 p, float a) {
    float s = sin(a), c = cos(a);
    return vec2(p.x * c - p.y * s, p.x * s + p.y * c);
}

float hash21(vec2 p) {
    p = fract(p * vec2(233.34, 851.73));
    p += dot(p, p + 23.45);
    return fract(p.x * p.y);
}


// Ambient light varies only with its own continuous clock.
float ambientLight(float time) { return 0.98 + 0.025 * sin(time * 0.21); }
vec3 bloom(vec3 colour, float intensity) { return colour; }

/// Normalised, origin-centred coordinates with the aspect ratio preserved.
vec2 uvCentered(vec2 position, vec2 size) {
    return (position - 0.5 * size) / min(size.x, size.y);
}

// ---------------------------------------------------------------- plasma
// Stacked, drifting sine fields -- the smoothest of the set, so it sits
// quietly under text.
vec4 wlPlasma(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 2.2;
    float v = 0.0;
    v += sin(uv.x * 1.7 + flow * 0.42);
    v += sin(uv.y * 1.4 - flow * 0.31);
    v += sin((uv.x + uv.y) * 1.1 + flow * 0.50);
    v += sin(length(uv * 1.6) - flow * 0.55);
    v = v * 0.25;

    float band = 0.5 + 0.5 * sin(v * 3.14159 + flow * 0.2);
    vec3 colour = mix(tintA, tintB, float(band));
    float vignette = 1.0 - 0.35 * length(uvCentered(position, size));
    float intensity = (0.55 + 0.45 * band) * vignette * ambientLight(time);
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ----------------------------------------------------------- kaleidoscope
// Polar folding over a drifting noise field, producing shifting symmetry.
vec4 wlKaleido(vec2 position, vec4 current,
                                 vec2 size, float time, float flow,
                                 vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float radius = length(uv);
    float angle = atan(uv.y, uv.x);

    const float segments = 8.0;
    float wedge = 6.28318 / segments;
    angle = abs(remainderSigned(angle + flow * 0.09, wedge) - wedge * 0.5);

    vec2 folded = vec2(cos(angle), sin(angle)) * radius;
    float pattern = 0.0;
    float scale = 2.4;
    for (int i = 0; i < 4; i++) {
        pattern += sin(folded.x * scale + flow * 0.5) * cos(folded.y * scale - flow * 0.37) / scale;
        scale *= 1.9;
    }

    float intensity = clamp(0.5 + pattern * 1.6, 0.0, 1.0);
    vec3 colour = mix(tintA, tintB, float(intensity));
    float falloff = ramp(1.05, 0.15, radius);
    float lit = intensity * falloff * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ---------------------------------------------------------------- aurora
// Layered vertical curtains sheared by noise, drifting like polar light.
vec4 wlAurora(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB, vec3 music) {
    vec2 uv = position / size;
    float glow = 0.0;

    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float phase = flow * (0.16 + fi * 0.045) + fi * 2.1;
        // Each curtain is a horizontal ribbon warped by stacked sines.
        float centre = 0.5
            + sin(uv.x * 2.3 + phase) * 0.16
            + sin(uv.x * 5.1 - phase * 0.7) * 0.06
            + sin(uv.x * 3.2 + phase + fi) * music.x * 0.085
            + sin(uv.x * 6.0 - phase + fi) * music.y * 0.035;
        float thickness = 0.055 + 0.03 * sin(phase * 0.8) + music.x * 0.025;
        float band = thickness / (abs(uv.y - centre) + thickness * 0.55);
        glow += band * (0.35 + 0.25 * sin(phase));
    }

    glow = clamp(glow * 0.28, 0.0, 1.4) * ambientLight(time);
    vec3 colour = mix(tintA, tintB, float(clamp(uv.y * 1.2, 0.0, 1.0)));
    return vec4(bloom(colour * float(glow), glow), 1.0);
}

// --------------------------------------------------------------- fractal
// Animated Julia set: escape-flow iteration with a slowly orbiting seed.
vec4 wlFractal(vec2 position, vec4 current,
                                 vec2 size, float time, float flow,
                                 vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 2.6;
    uv = rotate2(uv, flow * 0.05);

    vec2 seed = vec2(0.7885 * cos(flow * 0.12), 0.7885 * sin(flow * 0.17));
    vec2 z = uv;
    int iterations = 0;
    const int maxIterations = 64;
    for (int i = 0; i < maxIterations; i++) {
        z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + seed;
        if (dot(z, z) > 16.0) break;
        iterations++;
    }

    // Smooth the banding that raw iteration counts produce.
    float smoothed = float(iterations) + 1.0 - log2(max(log2(length(z)), 1.0));
    float t = clamp(smoothed / float(maxIterations), 0.0, 1.0);
    float shade = pow(t, 0.55);
    vec3 colour = mix(tintA, tintB, float(shade));
    float lit = shade * 1.15 * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ---------------------------------------------------------------- liquid
// Metaballs: overlapping fields summed into a single flowing surface.
vec4 wlLiquid(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float field = 0.0;

    for (int i = 0; i < 7; i++) {
        float fi = float(i);
        vec2 centre = vec2(
            sin(flow * (0.21 + fi * 0.031) + fi * 1.7) * 0.62,
            cos(flow * (0.17 + fi * 0.027) + fi * 2.3) * 0.44
        );
        float radius = 0.16 + 0.07 * sin(flow * 0.3 + fi);
        field += radius * radius / max(dot(uv - centre, uv - centre), 0.0006);
    }

    // Threshold the summed field so the blobs fuse and separate.
    // A hit lowers the threshold, so the blobs visibly expand and merge.
    float threshold = 1.1;
    float surface = ramp(threshold, 3.4, field);
    float rim = ramp(3.4, 6.5, field);
    vec3 colour = mix(tintA, tintB, float(rim));
    float lit = surface * 0.95 * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ---------------------------------------------------------------- warp
// Domain-warped noise: a field displaced by another copy of itself, giving
// slow marbled turbulence.
vec4 wlWarp(vec2 position, vec4 current,
                              vec2 size, float time, float flow,
                              vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 1.8;

    vec2 q = vec2(
        sin(uv.x * 1.3 + flow * 0.21) + cos(uv.y * 1.7 - flow * 0.13),
        cos(uv.x * 1.9 - flow * 0.17) + sin(uv.y * 1.1 + flow * 0.23)
    );
    vec2 r = vec2(
        sin(uv.x * 2.1 + q.y * 1.4 + flow * 0.11),
        cos(uv.y * 2.3 + q.x * 1.2 - flow * 0.09)
    );
    float value = 0.5 + 0.5 * sin(uv.x * 1.1 + r.x * 2.2 + flow * 0.15)
                      * cos(uv.y * 1.3 + r.y * 1.8 - flow * 0.12);

    float grain = hash21(floor(position * 0.5)) * 0.03;
    vec3 colour = mix(tintA, tintB, float(clamp(value, 0.0, 1.0)));
    float lit = (0.45 + 0.65 * value + grain) * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ============================================================ additional

float fbmNoise(vec2 p) {
    float total = 0.0, amplitude = 0.5;
    for (int i = 0; i < 5; i++) {
        vec2 cell = floor(p);
        vec2 f = fract(p);
        f = f * f * (3.0 - 2.0 * f);
        float a = hash21(cell);
        float b = hash21(cell + vec2(1.0, 0.0));
        float c = hash21(cell + vec2(0.0, 1.0));
        float d = hash21(cell + vec2(1.0, 1.0));
        total += amplitude * mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
        p *= 2.02;
        amplitude *= 0.5;
    }
    return total;
}

// ---------------------------------------------------------------- nebula
// Layered fractal noise, drifting like interstellar cloud.
vec4 wlNebula(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 2.6;
    // Warping the sampleValue point by another noise field gives the billowing.
    vec2 warp = vec2(fbmNoise(uv + flow * 0.035), fbmNoise(uv.yx - flow * 0.028));
    float density = fbmNoise(uv * 1.4 + warp * 2.1 + flow * 0.02);
    float shaped = ramp(0.28, 0.86, density);

    vec3 colour = mix(tintA, tintB, float(shaped));
    float lit = (0.22 + shaped * 0.95) * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// --------------------------------------------------------------- voronoi
// Drifting cell field; the seams between cells carry the light.
vec4 wlVoronoi(vec2 position, vec4 current,
                                 vec2 size, float time, float flow,
                                 vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 4.2;
    vec2 cell = floor(uv);
    float nearest = 10.0, second = 10.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 neighbour = cell + vec2(float(x), float(y));
            vec2 seed = vec2(hash21(neighbour), hash21(neighbour + 41.7));
            // Each seed orbits its own cell so the mosaic keeps shifting.
            vec2 point = neighbour + 0.5 + 0.42 * sin(flow * 0.32 + 6.28 * seed);
            float distance = length(point - uv);
            if (distance < nearest) { second = nearest; nearest = distance; }
            else if (distance < second) { second = distance; }
        }
    }

    // The gap between the two closest seeds is the cell border.
    float edge = ramp(0.0, 0.42, second - nearest);
    float glow = 1.0 - edge;
    vec3 colour = mix(tintA, tintB, float(clamp(nearest * 0.9, 0.0, 1.0)));
    float lit = (0.16 + glow * 0.95) * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ------------------------------------------------------------------ orbs
// Soft glowing spheres drifting at different depths.
vec4 wlOrbs(vec2 position, vec4 current,
                              vec2 size, float time, float flow,
                              vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float light = 0.0;
    float tintMix = 0.0;

    for (int i = 0; i < 9; i++) {
        float fi = float(i);
        float depth = 0.4 + hash21(vec2(fi, 3.1)) * 0.9;
        vec2 centre = vec2(
            sin(flow * (0.13 + fi * 0.021) + fi * 2.4) * 0.72,
            cos(flow * (0.11 + fi * 0.019) + fi * 1.6) * 0.5
        );
        float radius = (0.05 + hash21(vec2(fi, 7.7)) * 0.11) * depth;
        float distance = length(uv - centre);
        // Inverse-square falloff reads as a genuine light source.
        float contribution = radius * radius / max(distance * distance, 0.0008);
        light += contribution;
        tintMix += contribution * hash21(vec2(fi, 11.3));
    }

    float shaped = clamp(light * 0.5, 0.0, 1.6);
    vec3 colour = mix(tintA, tintB, float(clamp(tintMix / max(light, 0.001), 0.0, 1.0)));
    float lit = shaped * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ------------------------------------------------------------------ silk
// Stacked flowing ribbons, like light across folded fabric.
vec4 wlSilk(vec2 position, vec4 current,
                              vec2 size, float time, float flow,
                              vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float sheen = 0.0;

    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        float phase = flow * (0.19 + fi * 0.028) + fi * 1.9;
        float fold = sin(uv.x * (1.7 + fi * 0.5) + phase)
                   * cos(uv.x * (0.9 + fi * 0.3) - phase * 0.6) * 0.24;
        float band = abs(uv.y - fold + (fi - 2.5) * 0.14);
        // Thin, bright highlight along each fold.
        sheen += 0.018 / (band + 0.022);
    }

    float shaped = clamp(sheen * 0.30, 0.0, 1.5);
    vec3 colour = mix(tintA, tintB, float(clamp(uv.y + 0.5, 0.0, 1.0)));
    float lit = shaped * ambientLight(time);
    return vec4(bloom(colour * float(lit), lit), 1.0);
}

// ================================================== raymarched 3D effects
//
// These march a ray through a signed-distance field, estimate a surface
// normal by sampling the field around the hit, and shade it. That is what
// separates them from the 2D pieces above: there is real geometry with
// depth, silhouettes and lighting rather than a pattern painted flat.

/// Shared shading: a key light plus a rim, tinted by the album.
vec4 shadeSurface(vec3 normal, vec3 rayDir, float depth,
                          vec3 tintA, vec3 tintB, float time) {
    vec3 key = normalize(vec3(0.6, 0.8, -0.5));
    float diffuse = max(dot(normal, key), 0.0);
    float rim = pow(1.0 - max(dot(normal, -rayDir), 0.0), 2.5);
    float fog = exp(-depth * 0.055);

    vec3 colour = mix(tintA, tintB, float(clamp(diffuse, 0.0, 1.0)));
    float intensity = (0.16 + diffuse * 0.75 + rim * 0.55) * fog * ambientLight(time);
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ---------------------------------------------------------------- waves
float waveField(vec3 p, float flow) {
    // Distance to an animated heightfield, approximated by the vertical gap.
    float h = sin(p.x * 0.9 + flow * 0.7) * 0.34
            + sin(p.z * 1.3 - flow * 0.5) * 0.26
            + sin((p.x + p.z) * 0.6 + flow * 0.3) * 0.18;
    return (p.y - h) * 0.62;
}

vec4 wlWaves(vec2 position, vec4 current,
                               vec2 size, float time, float flow,
                               vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    vec3 origin = vec3(0, 1.5, -4.0 + flow * 0.6);
    vec3 dir = normalize(vec3(uv, 1.0));

    float travelled = 0.0;
    for (int i = 0; i < 56; i++) {
        float d = waveField(origin + dir * travelled, flow);
        if (d < 0.004 || travelled > 24.0) break;
        travelled += d;
    }
    if (travelled > 24.0) return vec4(0, 0, 0, 1);

    vec3 hit = origin + dir * travelled;
    // Differentiating the height sum by hand costs three cosines; estimating
    // the same normal by sampling costs six more evaluations of the field.
    float cross = cos((hit.x + hit.z) * 0.6 + flow * 0.3) * 0.6 * 0.18;
    float dx = cos(hit.x * 0.9 + flow * 0.7) * 0.9 * 0.34 + cross;
    float dz = cos(hit.z * 1.3 - flow * 0.5) * 1.3 * 0.26 + cross;
    vec3 normal = normalize(vec3(-dx, 1.0, -dz));
    return shadeSurface(normal, dir, travelled, tintA, tintB, time);
}

// ---------------------------------------------------------------- helix
float helixField(vec3 p, float flow) {
    float best = 10.0;
    for (int strand = 0; strand < 2; strand++) {
        float offset = float(strand) * 3.14159;
        float angle = p.z * 0.9 + flow * 0.5 + offset;
        vec2 centre = vec2(cos(angle), sin(angle)) * 0.85;
        best = min(best, length(p.xy - centre) - 0.17);
    }
    return best;
}

vec4 wlHelix(vec2 position, vec4 current,
                               vec2 size, float time, float flow,
                               vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    vec3 origin = vec3(0, 0, -4.2);
    vec3 dir = normalize(vec3(uv, 1.0));

    float travelled = 0.0;
    for (int i = 0; i < 52; i++) {
        float d = helixField(origin + dir * travelled, flow);
        if (d < 0.003 || travelled > 20.0) break;
        travelled += d * 0.85;
    }
    if (travelled > 20.0) return vec4(0, 0, 0, 1);

    vec3 hit = origin + dir * travelled;
    // Pick the strand actually hit, then take its normal analytically: the
    // radial offset with the strand's own axis projected out. Sampling the
    // field six times would give the same vector for far more work.
    float a0 = hit.z * 0.9 + flow * 0.5;
    vec2 c0 = vec2(cos(a0), sin(a0)) * 0.85;
    vec2 c1 = -c0;
    vec2 centre = length(hit.xy - c0) < length(hit.xy - c1) ? c0 : c1;
    vec3 axis = normalize(vec3(-centre.y * 0.9, centre.x * 0.9, 1.0));
    vec3 radial = vec3(hit.xy - centre, 0.0);
    vec3 normal = normalize(radial - axis * dot(radial, axis));
    return shadeSurface(normal, dir, travelled, tintA, tintB, time);
}


// ===================================================== analytic 3D effects
//
// Real perspective and depth without marching a ray. A plane or a sphere can
// be intersected in closed form -- one solve, and the surface normal falls
// out of the algebra instead of costing six extra field samples. That is the
// whole difference from the two marched pieces above: the same sense of
// space for a small fraction of the per-pixel cost, which is what keeps
// these smooth on integrated GPUs.

/// Horizon-to-zenith wash, so distance falls off into colour rather than
/// into dead black.
vec3 skyWash(vec3 dir, vec3 tintA, vec3 tintB, float lift) {
    float h = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    return mix(tintB * float(0.30), tintA * float(0.13), float(h)) * float(lift);
}

/// Ray/plane in closed form. Negative when the ray never reaches it.
float planeHit(vec3 origin, vec3 dir, float height) {
    if (abs(dir.y) < 1e-4) return -1.0;
    float t = (height - origin.y) / dir.y;
    return t > 0.0 ? t : -1.0;
}

/// Ray/sphere in closed form: the near root of the quadratic, or -1.
float sphereHit(vec3 origin, vec3 dir, vec3 centre, float radius) {
    vec3 oc = origin - centre;
    float b = dot(oc, dir);
    float c = dot(oc, oc) - radius * radius;
    float disc = b * b - c;
    if (disc < 0.0) return -1.0;
    float t = -b - sqrt(disc);
    return t > 0.0 ? t : -1.0;
}

/// Distance haze towards the sky colour.
vec3 haze(vec3 colour, vec3 sky, float depth, float density) {
    return mix(colour, sky, float(1.0 - exp(-depth * density)));
}

/// Two-octave value noise for pieces that want texture but not the full
/// five-octave fbm.
float softNoise(vec2 p) {
    vec2 cell = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(cell), b = hash21(cell + vec2(1, 0));
    float c = hash21(cell + vec2(0, 1)), d = hash21(cell + vec2(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

/// Lambert key light plus rim, shared by the closed-form surfaces.
vec3 litSurface(vec3 normal, vec3 dir, vec3 tintA, vec3 tintB, float lift) {
    vec3 key = normalize(vec3(0.45, 0.75, -0.55));
    float diffuse = max(dot(normal, key), 0.0);
    float rim = pow(1.0 - max(dot(normal, -dir), 0.0), 2.5);
    vec3 colour = mix(tintB, tintA, float(diffuse));
    return colour * float((0.17 + diffuse * 0.70 + rim * 0.45) * lift);
}

// ---------------------------------------------------------------- ripple
// A lit water surface. The plane is still one solve; all the interest is in
// the height field laid over it -- three circular wave trains from separate
// origins, plus a long directional 0.0 underneath so the water has a set
// rather than looking like a pond. Because the gradient of a sum of sines is
// just the sum of the derivatives, the normal is exact and costs a handful
// of cosines instead of a sampled estimate.
vec4 wlRipple(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 dir = normalize(vec3(uv.x, uv.y + 0.30, 1.0));
    vec3 origin = vec3(0.0, 1.35, 0.0);

    // A sun in the sky, because the water needs something to glitter from.
    // Without a light source to reflect, a water shader is just a gradient.
    vec3 sun = normalize(vec3(0.22, 0.40, 1.0));
    float toSun = max(dot(dir, sun), 0.0);
    vec3 sky = skyWash(dir, tintA, tintB, lift)
              + tintA * float(pow(toSun, 7.0) * 0.35 * lift)
              + vec3(float(pow(toSun, 220.0) * 0.85 * lift));

    float t = planeHit(origin, dir, 0.0);
    if (t < 0.0) return vec4(sky, 1.0);

    vec3 hit = origin + dir * t;
    vec2 p = vec2(hit.x, hit.z);
    // Detail has to fade with distance or the far water aliases into noise.
    // This is the cheap stand-in for choosing a mip level.
    float detail = 1.0 / (1.0 + t * 0.20);

    vec2 grad = vec2(0.0);
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        vec2 centre = vec2(sin(fi * 2.4) * 3.4, 5.0 + fi * 2.8);
        vec2 d = p - centre;
        float r = length(d) + 1e-4;
        float k = 2.0 + fi * 0.75;
        // d/dr of amp*sin(kr - wt), pushed back out along the radius.
        float slope = (0.060 - fi * 0.013) * detail * k
                    * cos(r * k - flow * (1.05 + fi * 0.28)) * exp(-r * 0.11);
        grad += d / r * slope;
    }
    vec2 set = vec2(0.82, 0.57);
    grad += set * 0.05 * detail * cos(dot(p, set) * 0.75 - flow * 0.45);

    vec3 normal = normalize(vec3(-grad.x, 1.0, -grad.y));
    vec3 view = -dir;

    // Blinn-Phong. The halfway vector is what draws the long glitter streak
    // across the water instead of one round hotspot.
    vec3 halfway = normalize(sun + view);
    float facing = max(dot(normal, halfway), 0.0);
    float glitter = pow(facing, 110.0);
    float sheen = pow(facing, 16.0) * 0.16;

    // Fresnel: water at a grazing angle is a mirror, water underfoot is not.
    // This is most of what makes a surface read as wet.
    float fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(normal, view), 0.0), 5.0);
    vec3 depths = tintB * float(0.24 * lift);
    vec3 mirrored = mix(tintB * float(0.5), tintA, float(0.65)) * float(0.9 * lift);

    vec3 colour = mix(depths, mirrored, float(fresnel))
                 + vec3(float((glitter + sheen) * lift));
    colour = haze(colour, sky, t, 0.055);
    return vec4(colour, 1.0);
}

// ---------------------------------------------------------------- mirror
// A polished floor: the reflected ray is one `reflect()`, and what it
// returns is the sky, so the reflection costs a wash lookup rather than a
// second trace.
vec4 wlMirror(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 dir = normalize(vec3(uv.x, uv.y + 0.05, 1.0));
    vec3 origin = vec3(0.0, 0.9, 0.0);

    float bands = 0.5 + 0.5 * sin(dir.y * 7.0 - flow * 0.35) * exp(-abs(dir.y) * 1.4);
    vec3 sky = skyWash(dir, tintA, tintB, lift) + mix(tintA, tintB, float(bands)) * float(bands * 0.42 * lift);

    float t = planeHit(origin, dir, 0.0);
    if (t < 0.0) return vec4(sky, 1.0);

    vec3 hit = origin + dir * t;
    float wobble = sin(hit.z * 1.6 + flow * 0.8) * 0.02 + sin(hit.x * 2.1 - flow * 0.5) * 0.015;
    vec3 normal = normalize(vec3(wobble, 1.0, wobble * 0.6));
    vec3 bounce = reflect(dir, normal);
    float rippled = 0.5 + 0.5 * sin(bounce.y * 7.0 - flow * 0.35) * exp(-abs(bounce.y) * 1.4);

    vec3 colour = mix(tintB * float(0.35), tintA, float(rippled)) * float(rippled * 0.85 * lift);
    // Fresnel: a glancing ray reflects most, a steep one drinks the floor.
    float fresnel = pow(1.0 - max(-dir.y, 0.0), 4.0);
    colour = mix(colour * float(0.35), colour, float(fresnel));
    colour = haze(colour, sky, t, 0.05);
    return vec4(bloom(colour, rippled * fresnel), 1.0);
}

// -------------------------------------------------------------- wormhole
// Circular tunnel with the twist applied as a function of depth, so the
// walls appear to spiral away from the camera.
vec4 wlWormhole(vec2 position, vec4 current,
                                  vec2 size, float time, float flow,
                                  vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size) * 2.0;
    float lift = ambientLight(time);
    float r = max(length(uv), 1e-3);
    float depth = 1.0 / r + flow * 0.6;
    float around = atan(uv.y, uv.x) * 0.15915494 + depth * 0.10;

    float strands = 0.5 + 0.5 * sin(around * 25.1327 + depth * 3.0);
    float rings = 0.5 + 0.5 * sin(depth * 12.0);
    float pattern = strands * 0.55 + rings * 0.35;

    vec3 colour = mix(tintB, tintA, float(strands));
    float near = clamp(r * 1.5, 0.0, 1.0);
    float intensity = (0.08 + pattern * 0.95) * near * lift;
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ----------------------------------------------------------------- rings
// Concentric rings marching toward the camera. Each one's screen radius is
// its true radius over its depth -- the projection is the whole effect.
vec4 wlRings(vec2 position, vec4 current,
                               vec2 size, float time, float flow,
                               vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    float r = length(uv);
    float glow = 0.0;

    for (int i = 0; i < 9; i++) {
        // Each ring recycles through depth on its own offset.
        float z = fract(flow * 0.06 + float(i) * 0.1111) * 5.0 + 0.28;
        float radius = 1.15 / z;
        float band = exp(-abs(r - radius) * (55.0 / (1.0 + z)));
        glow += band * (1.0 - z * 0.18);
    }

    vec3 colour = mix(tintB, tintA, float(clamp(r * 1.4, 0.0, 1.0)));
    float intensity = clamp(glow, 0.0, 1.6) * 0.7 * lift;
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ---------------------------------------------------------------- vortex
// A spiral drain: radius and angle in one term, depth-faded toward the eye.
vec4 wlVortex(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    float r = max(length(uv), 1e-3);
    float around = atan(uv.y, uv.x);
    // log(r) makes the spiral self-similar, so it never visibly repeats.
    float spiral = sin(around * 3.0 + log(r) * 5.0 - flow * 0.9);
    float arms = 0.5 + 0.5 * spiral;

    float core = exp(-r * 3.4);
    vec3 colour = mix(tintB, tintA, float(arms));
    float intensity = (arms * 0.62 * ramp(0.0, 0.35, r) + core * 0.85) * lift;
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ----------------------------------------------------------------- orbit
// Five genuine spheres, each solved with the quadratic. Real silhouettes and
// real normals, at five intersection tests per pixel instead of a march.
vec4 wlOrbit(vec2 position, vec4 current,
                               vec2 size, float time, float flow,
                               vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 dir = normalize(vec3(uv, 1.3));
    vec3 origin = vec3(0.0, 0.0, -5.0);
    vec3 sky = skyWash(dir, tintA, tintB, lift);

    float nearest = 1e9;
    vec3 normal = vec3(0, 1, 0);
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float a = flow * 0.32 + fi * 1.2566;
        vec3 centre = vec3(cos(a) * 1.9, sin(a * 1.3) * 0.8, sin(a) * 1.9);
        float radius = 0.44 + 0.09 * sin(flow * 0.6 + fi);
        float t = sphereHit(origin, dir, centre, radius);
        if (t > 0.0 && t < nearest) {
            nearest = t;
            normal = ((origin + dir * t) - centre) / radius;
        }
    }
    if (nearest > 1e8) return vec4(sky, 1.0);

    vec3 colour = haze(litSurface(normal, dir, tintA, tintB, lift), sky, nearest, 0.045);
    return vec4(bloom(colour, 0.6), 1.0);
}

// --------------------------------------------------------------- bubbles
// Glass spheres. The shell is Fresnel only -- bright at the silhouette,
// clear through the middle -- which is why they read as hollow without any
// refraction being traced.
vec4 wlBubbles(vec2 position, vec4 current,
                                 vec2 size, float time, float flow,
                                 vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 dir = normalize(vec3(uv, 1.4));
    vec3 origin = vec3(0.0, 0.0, -4.5);
    vec3 colour = skyWash(dir, tintA, tintB, lift);

    for (int i = 0; i < 7; i++) {
        float fi = float(i);
        vec3 centre = vec3(
            sin(fi * 2.1 + flow * 0.16) * 1.9,
            fract(hash21(vec2(fi, 4.0)) + flow * 0.045) * 4.0 - 2.0,
            sin(fi * 1.3) * 1.4
        );
        float radius = 0.30 + hash21(vec2(fi, 9.0)) * 0.34;
        float t = sphereHit(origin, dir, centre, radius);
        if (t < 0.0) continue;
        vec3 normal = ((origin + dir * t) - centre) / radius;
        float fresnel = pow(1.0 - max(dot(normal, -dir), 0.0), 3.0);
        float sheen = pow(max(dot(normal, normalize(vec3(0.5, 0.8, -0.4))), 0.0), 32.0);
        colour += mix(tintA, tintB, float(fract(fi * 0.37))) * float((fresnel * 0.75 + sheen * 0.6) * lift);
    }
    return vec4(bloom(colour, 0.55), 1.0);
}

// ---------------------------------------------------------------- galaxy
// Logarithmic spiral arms seen at a tilt. Squashing y before taking the
// radius is what turns a face-on disc into one lying in perspective.
vec4 wlGalaxy(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    vec2 disc = vec2(uv.x, uv.y / 0.42);   // the tilt
    float lift = ambientLight(time);
    float r = max(length(disc), 1e-3);
    float around = atan(disc.y, disc.x);

    float arms = 0.5 + 0.5 * sin(around * 2.0 + log(r) * 4.2 - flow * 0.32);
    arms *= ramp(1.5, 0.25, r);
    float grain = softNoise(disc * 5.0 + flow * 0.05) * 0.35 + 0.65;
    float core = exp(-r * 4.5);

    vec3 colour = mix(tintB, tintA, float(arms));
    float intensity = (arms * grain * 0.7 + core * 1.1) * lift;
    return vec4(bloom(colour * float(intensity), intensity), 1.0);
}

// ---------------------------------------------------------------- strata
// Parallax layers: each sits at its own depth and scrolls at a rate
// proportional to 1/depth, which is the same relationship that makes distant
// scenery crawl past a car window.
vec4 wlStrata(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 colour = mix(tintB * float(0.28), tintA * float(0.08), float(uv.y + 0.5)) * float(lift);

    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        float depth = 1.0 + fi * 0.9;
        float scroll = flow * (0.20 / depth);
        float ridge = sin(uv.x * (1.4 + fi * 0.35) + scroll) * 0.09
                    + sin(uv.x * (3.1 - fi * 0.2) - scroll * 1.6) * 0.045;
        float top = -0.32 + fi * 0.085 + ridge;
        float mask = ramp(top + 0.012, top - 0.012, uv.y);
        vec3 band = mix(tintA, tintB, float(fi / 5.0)) * float((1.0 - fi * 0.13) * lift);
        colour = mix(colour, band, float(mask * 0.85));
    }
    return vec4(bloom(colour, 0.4), 1.0);
}

// ---------------------------------------------------------------- clouds
// Three noise layers at different depths, offset against each other. Kept to
// two octaves apiece -- the full fbm is five, and at three layers that would
// be fifteen octaves a pixel for no visible gain.
vec4 wlClouds(vec2 position, vec4 current,
                                vec2 size, float time, float flow,
                                vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 colour = mix(tintB * float(0.30), tintA * float(0.10), float(uv.y + 0.5)) * float(lift);

    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float depth = 1.0 + fi * 1.2;
        vec2 p = uv * (2.2 + fi * 1.5) + vec2(flow * (0.14 / depth), -fi * 3.7);
        float n = softNoise(p) * 0.65 + softNoise(p * 2.3 + 11.0) * 0.35;
        float body = ramp(0.48, 0.78, n) * (1.0 - fi * 0.22);
        colour += mix(tintA, tintB, float(fi * 0.4)) * float(body * 0.55 * lift);
    }
    return vec4(bloom(colour, 0.5), 1.0);
}

// -------------------------------------------------------------- curtains
// Aurora sheets standing at different distances. The vertical falloff plus
// horizontal drift is what makes them hang rather than lie flat.
vec4 wlCurtains(vec2 position, vec4 current,
                                  vec2 size, float time, float flow,
                                  vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 colour = tintB * float(0.07 * lift);

    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float depth = 1.0 + fi * 0.7;
        float sway = sin(uv.x * (2.2 + fi * 0.6) + flow * (0.34 / depth) + fi * 2.0) * 0.16;
        float centre = -0.05 + fi * 0.04 + sway;
        float sheet = exp(-abs(uv.y - centre) * (7.0 + fi * 2.2));
        // Vertical striations, so the sheet has grain instead of being a
        // smooth smear.
        float grain = 0.65 + 0.35 * sin(uv.x * 30.0 + flow * 0.5 + fi * 3.1);
        colour += mix(tintA, tintB, float(fi / 4.0)) * float(sheet * grain * (0.75 / depth) * lift);
    }
    return vec4(bloom(colour, 0.6), 1.0);
}

// --------------------------------------------------------------- ribbons
// Sine bands whose thickness and brightness both scale with 1/depth, which
// is enough to make them stack away from the eye.
vec4 wlRibbons(vec2 position, vec4 current,
                                 vec2 size, float time, float flow,
                                 vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec3 colour = tintB * float(0.06 * lift);

    for (int i = 0; i < 7; i++) {
        float fi = float(i);
        float depth = 0.6 + fi * 0.45;
        float wave = sin(uv.x * (1.8 / depth) * 2.4 + flow * (0.5 / depth) + fi) * (0.30 / depth);
        float thickness = 0.020 + 0.030 / depth;
        float band = ramp(thickness, 0.0, abs(uv.y - wave - (fi - 3.0) * 0.055));
        colour += mix(tintA, tintB, float(fi / 6.0)) * float(band * (0.9 / depth) * lift);
    }
    return vec4(bloom(colour, 0.65), 1.0);
}

// ----------------------------------------------------------------- prism
// One refraction, done honestly: the three channels are offset by different
// amounts because they bend by different amounts, which is where the fringe
// colour comes from.
vec4 wlPrism(vec2 position, vec4 current,
                               vec2 size, float time, float flow,
                               vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float lift = ambientLight(time);
    vec2 p = rotate2(uv, flow * 0.05);
    float spread = 0.055;

    vec3 colour = vec3(0.0);
    for (int c = 0; c < 3; c++) {
        float shift = (float(c) - 1.0) * spread;
        float fan = sin((p.x + shift) * 6.0 + p.y * 2.0 + flow * 0.28);
        float band = ramp(0.35, 0.95, fan) * exp(-abs(p.y) * 1.1);
        vec3 channel = c == 0 ? vec3(1.0, 0.25, 0.25)
                     : c == 1 ? vec3(0.25, 1.0, 0.45)
                              : vec3(0.35, 0.4, 1.0);
        colour += channel * float(band * 0.45);
    }
    // Pulled back toward the album so it tints rather than going full rainbow.
    colour = mix(colour, mix(tintA, tintB, float(0.5)) * float(length(vec3(colour)) * 0.6), float(0.55));
    float intensity = clamp(float(colour.r + colour.g + colour.b) * 0.4, 0.0, 1.4) * lift;
    return vec4(bloom(colour * float(lift), intensity), 1.0);
}


// ----------------------------------------------------- shared fluid field
float ambientField(vec2 p, float flow) {
    vec2 q = p + vec2(sin(p.y * 2.3 + flow * 0.13), cos(p.x * 1.9 - flow * 0.11)) * 0.24;
    float f = sin(q.x * 2.1 + q.y * 1.8 + flow * 0.17);
    f += 0.52 * sin(q.y * 3.8 - q.x * 1.2 - flow * 0.12 + f * 0.65);
    f += 0.24 * sin(q.x * 7.0 + q.y * 5.1 + flow * 0.08);
    return f / 1.76;
}

// --------------------------------------------------------------- tidal
vec4 wlTidal(vec2 position, vec4 current,
    vec2 size, float time, float flow, vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float field = ambientField(uv * 1.8, flow);
    float contour = 0.5 + 0.5 * sin(field * 3.8 + flow * 0.035);
    float body = ramp(0.18, 0.92, contour);
    float filament = exp(-pow(abs(contour - 0.61) * 10.0, 2.0));
    vec3 colour = mix(tintA * float(0.16), tintB * float(0.46), float(body));
    colour += mix(tintA, tintB, float(0.28)) * float(filament * 0.13);
    float veil = 0.86 + 0.14 * sin(uv.y * 1.2 + time * 0.07);
    float vignette = max(0.32, 1.0 - dot(uv * vec2(0.32, 0.7), uv * vec2(0.32, 0.7)));
    return vec4(colour * float(veil * vignette * 0.94), 1.0);
}

// --------------------------------------------------------- light trails
vec4 wlLightTrails(vec2 position, vec4 current,
    vec2 size, float time, float flow, vec3 tintA, vec3 tintB) {
    vec2 uv = uvCentered(position, size);
    float fog = 0.5 + 0.5 * ambientField(uv * 2.3, flow * 0.75);
    vec3 colour = mix(tintA, tintB, float(fog)) * float(0.07 + pow(fog, 2.0) * 0.19);
    vec2 p = rotate2(uv, 0.55);
    for (int i = 0; i < 3; i++) {
        float lane = float(i) - 1.0;
        float curve = lane * 0.17 + sin(p.x * 1.4 + flow * 0.18 + float(i)) * 0.095;
        curve += sin(p.x * 3.3 - flow * 0.12 + lane) * 0.027;
        float distance = abs(p.y - curve);
        float width = 0.0022;
        float core = exp(-pow(distance / width, 2.0));
        float halo = exp(-distance * 43.0) * 0.25;
        float head = fract(flow * 0.09 + float(i) * 0.29) * 6.0 - 3.0;
        float tail = exp(-max(0.0, head - p.x) * 3.0) * step(p.x, head);
        float travel = 0.16 + 0.84 * tail;
        float tip = exp(-pow((p.x - head) / 0.012, 2.0) - pow(distance / 0.006, 2.0));
        vec3 ink = mix(tintA, tintB, float(float(i) / 2.0));
        ink = mix(ink, vec3(1.0), float(0.10));
        colour += ink * float(((core + halo) * travel + tip * 0.22) * 0.53);
    }
    vec2 grid = uv * 18.0 + vec2(flow * 0.13, -flow * 0.08);
    vec2 cell = floor(grid), local = fract(grid) - 0.5;
    float star = exp(-dot(local, local) * 105.0) * step(0.955, hash21(cell));
    colour += mix(tintA, tintB, float(0.5)) * float(star * 0.055);
    return vec4(colour * float(0.96), 1.0);
}

// ======================================================= direct rendering
// The same effects drawn by Lunalyr's own Metal view instead of SwiftUI's
// `colorEffect`, so the wallpaper renders at a bounded pixel budget and is
// upscaled, rather than shading every Retina pixel. The album tint (formerly
// a `.color` blend layer) and colour richness (a `.saturation` pass) are
// applied here, in the same pass. Each fragment calls its effect directly;
// the effect is linked into the pipeline as a private function.

float wlBlendLum(vec3 c) { return dot(c, vec3(0.3, 0.59, 0.11)); }

vec3 wlClipColor(vec3 c) {
    float l = wlBlendLum(c);
    float n = min(c.r, min(c.g, c.b));
    float x = max(c.r, max(c.g, c.b));
    if (n < 0.0) c = l + (c - l) * l / max(l - n, 1e-5);
    if (x > 1.0) c = l + (c - l) * (1.0 - l) / max(x - l, 1e-5);
    return c;
}

vec3 wlEncodeSRGB(vec3 l) {
    return mix(1.055 * pow(l, vec3(1.0 / 2.4)) - 0.055, l * 12.92, lessThanEqual(l, vec3(0.0031308)));
}

vec3 wlFromOKLab(vec3 c) {
    float l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
    float m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
    float s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
    l = l * l * l; m = m * m * m; s = s * s * s;
    vec3 linear = vec3( 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
                           -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
                           -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
    return wlEncodeSRGB(clamp(linear, 0.0, 1.0));
}

// The album gradient runs corner to corner, top-leading to bottom-trailing,
// through five evenly spaced stops -- the layer that used to sit on top.
// SwiftUI blends gradient stops perceptually, so the stops arrive in OKLab.
vec3 wlAlbumGradient(vec2 position, vec2 size, vec3 stops[5]) {
    float t = clamp(dot(position, size) / max(dot(size, size), 1e-3), 0.0, 1.0) * 4.0;
    int index = min(int(t), 3);
    return wlFromOKLab(mix(stops[index], stops[index + 1], t - float(index)));
}

// `.color` blend: the gradient's hue and saturation, the effect's luminosity;
// then a luminance-preserving saturation for colour richness.
vec4 wlGrade(vec4 effect, vec2 position, vec2 size, float vividness, vec3 stops[5]) {
    vec3 lit = clamp(vec3(effect.rgb), 0.0, 1.0);
    vec3 album = wlAlbumGradient(position, size, stops);
    vec3 tinted = wlClipColor(album + (wlBlendLum(lit) - wlBlendLum(album)));
    float grey = dot(tinted, vec3(0.2126, 0.7152, 0.0722));
    return vec4(vec3(clamp(mix(vec3(grey), tinted, vividness), 0.0, 1.0)), 1.0);
}
