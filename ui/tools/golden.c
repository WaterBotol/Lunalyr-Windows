// Golden-vector generator for ui/src/timing/golden.test.ts.
//
// Build against the UNMODIFIED macOS sources (Sources/Support/LyricMotion.c,
// ConstellationGeometry.c and their headers) copied next to this file:
//   cc -O0 -ffp-contract=off -o golden golden.c LyricMotion.c ConstellationGeometry.c -lm
//   ./golden > ../src/timing/golden.fixture.json
// -ffp-contract=off keeps the arithmetic unfused, like JavaScript.
//
#include "LyricMotion.h"
#include "ConstellationGeometry.h"
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint64_t rng = 0x9E3779B97F4A7C15ull;
static uint64_t next_u64(void) { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; return rng; }
static double uni(double lo, double hi) { return lo + (hi - lo) * ((next_u64() >> 11) * (1.0 / 9007199254740992.0)); }
static void num(double v) {
    if (isnan(v)) printf("\"NaN\"");
    else if (v == 0 && signbit(v)) printf("\"-0\"");
    else if (isinf(v)) printf(v > 0 ? "\"Infinity\"" : "\"-Infinity\"");
    else printf("%.17g", v);
}
static void pose(VCWordPose p) {
    printf("["); num(p.x); printf(","); num(p.y); printf(","); num(p.scale); printf(",");
    num(p.angle); printf(","); num(p.opacity); printf(","); num(p.blur); printf("]");
}
static uint64_t fnv(const char *s, uint64_t basis) {
    uint64_t v = basis;
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) v = (v ^ (uint64_t)*p) * 1099511628211ull;
    return v;
}
static uint64_t bits(double d) { uint64_t b; memcpy(&b, &d, 8); return b; }

int main(void) {
    printf("{\n\"ease\":[");
    for (int i = -10; i <= 110; i++) { if (i > -10) printf(","); double t = i / 100.0; printf("["); num(t); printf(","); num(vc_motion_ease(t)); printf("]"); }
    printf(",[\"NaN\","); num(vc_motion_ease(NAN)); printf("],[\"Infinity\","); num(vc_motion_ease(INFINITY)); printf("]");
    printf("],\n\"handoff\":[");
    int first = 1;
    double cadences[] = {0.1, 0.4, 0.8, 2.0};
    for (int c = 0; c < 4; c++) for (int i = -2; i <= 30; i++) {
        double age = i * 0.04;
        if (!first) printf(","); first = 0;
        printf("["); num(age); printf(","); num(cadences[c]); printf(","); num(vc_motion_handoff(age, cadences[c])); printf("]");
    }
    printf(",[\"NaN\",0.4,"); num(vc_motion_handoff(NAN, 0.4)); printf("]");
    printf("],\n\"rolling\":[");
    const char *seeds[] = {"197", "12345678901234567", "18446744073709551615", "1469598103934665603", "0"};
    uint64_t seedv[] = {197ull, 12345678901234567ull, 18446744073709551615ull, 1469598103934665603ull, 0ull};
    for (int i = 0; i < 260; i++) {
        int style = (int)(next_u64() % 3);
        double rank = uni(-3, 11);
        double age = (i % 7 == 0) ? uni(-2, 0) : uni(0, 6);
        double duration = (i % 5 == 0) ? uni(0.01, 0.2) : uni(0.1, 4);
        int s = (int)(next_u64() % 5);
        double energy = (i % 3 == 0) ? 0 : uni(0, 1.2);
        int reduced = (i % 9 == 0);
        if (i) printf(",");
        printf("[%d,", style); num(rank); printf(","); num(age); printf(","); num(duration);
        printf(",\"%s\",", seeds[s]); num(energy); printf(",%d,", reduced);
        pose(vc_rolling_pose(style, rank, age, duration, seedv[s], energy, reduced)); printf("]");
    }
    // Explicit cases from motion_test.c and invalid inputs.
    printf(",[0,\"NaN\",\"Infinity\",\"NaN\",\"0\",\"NaN\",0,"); pose(vc_rolling_pose(0, NAN, INFINITY, NAN, 0, NAN, 0)); printf("]");
    printf(",[1,0,0.5,3,\"197\",1,0,"); pose(vc_rolling_pose(1, 0, .5, 3, 197, 1, 0)); printf("]");
    printf("],\n\"gesture\":[");
    for (int i = 0; i < 140; i++) {
        double age = (i % 10 == 0) ? uni(-1, 0) : uni(0, 4);
        double duration = uni(0.02, 3.5);
        int s = (int)(next_u64() % 5);
        double energy = (i % 4 == 0) ? 0 : uni(0, 1.2);
        int reduced = (i % 11 == 0);
        if (i) printf(",");
        printf("["); num(age); printf(","); num(duration); printf(",\"%s\",", seeds[s]); num(energy); printf(",%d,", reduced);
        pose(vc_vocal_gesture(age, duration, seedv[s], energy, reduced)); printf("]");
    }
    printf("],\n\"reveal\":[");
    first = 1;
    for (int i = -20; i <= 160; i += 3) {
        double t = i / 40.0; // -0.5 .. 4
        double spans[][2] = {{1, 2}, {1, 1.1}, {1, 3}, {0.5, 0.5}};
        for (int k = 0; k < 4; k++) {
            if (!first) printf(","); first = 0;
            double s = spans[k][0], e = spans[k][1];
            printf("["); num(t); printf(","); num(s); printf(","); num(e); printf(",");
            num(vc_word_reveal(t, s, e)); printf(",");
            for (int preview = 0; preview <= 1; preview++) for (int next = 0; next <= 1; next++) {
                num(vc_word_visibility(t, s, e, preview, next, 0.5)); printf(",");
            }
            num(vc_word_visibility(t, s, e, 1, 1, 1.25)); printf(",");
            num(vc_word_settle(t / 3.0)); printf("]");
        }
    }
    printf("],\n\"envelope\":[");
    double env[][4] = {{240, 42, 1.2, 0}, {240, 42, 1.38, 3.5}, {100, 30, 0.5, -20}, {80, 20, 3, 120}, {50, 10, NAN, NAN}};
    for (int i = 0; i < 5; i++) {
        VCBox b = vc_word_envelope(env[i][0], env[i][1], env[i][2], env[i][3]);
        if (i) printf(",");
        printf("["); for (int k = 0; k < 4; k++) { num(env[i][k]); printf(","); } num(b.width); printf(","); num(b.height); printf("]");
    }
    printf("],\n\"lineEnd\":[");
    double le[][4] = {{10, 14, 4, .45}, {10, 50, 4, .45}, {10, 30, 20, .5}, {10, 90, 4, NAN}, {10, 9, 4, .45}, {0, 100, 1, 2}};
    for (int i = 0; i < 6; i++) {
        if (i) printf(",");
        printf("["); for (int k = 0; k < 4; k++) { num(le[i][k]); printf(","); }
        num(vc_estimated_line_end(le[i][0], le[i][1], (int)le[i][2], le[i][3])); printf("]");
    }
    printf("],\n\"gaps\":[");
    {
        VCVocalWindow a[] = {{10, 14}, {22, 28}};
        VCVocalWindow b[] = {{0, 5}, {6, 10}};
        VCVocalWindow c[] = {{0, 8}, {4, 10}, {14, 18}};
        VCVocalWindow d[] = {{3, 4}, {4.5, 9}, {12.5, 13}, {13, 20}};
        struct { VCVocalWindow *w; int n; const char *name; } sets[] = {{a, 2, "a"}, {b, 2, "b"}, {c, 3, "c"}, {d, 4, "d"}};
        double durations[] = {0, 40, 22, 21};
        first = 1;
        for (int s = 0; s < 4; s++) for (int di = 0; di < 4; di++) for (int ti = 0; ti <= 46; ti++) {
            double time = ti * 0.9;
            double minimum = (ti % 2) ? 2.4 : 1.0;
            VCVocalGap g = {0};
            int found = vc_vocal_gap(sets[s].w, sets[s].n, time, durations[di], minimum, &g);
            if (!first) printf(","); first = 0;
            printf("[\"%s\",", sets[s].name); num(durations[di]); printf(","); num(time); printf(","); num(minimum);
            printf(",%d,", found); num(found ? g.start : 0); printf(","); num(found ? g.end : 0); printf(",%d]", found ? g.nextVocal : 0);
        }
    }
    printf("],\n\"connections\":[");
    for (int set = 0; set < 24; set++) {
        int count = 2 + (int)(next_u64() % 11);
        VCBox boxes[16];
        VCBox view = {0, 0, uni(600, 1600), uni(400, 1000)};
        for (int i = 0; i < count; i++) {
            double w = uni(30, 260), h = uni(20, 90);
            boxes[i] = (VCBox){uni(-80, view.width - w + 40), uni(-60, view.height - h + 40), w, h};
        }
        double clearance = (set % 4 == 0) ? 6 : uni(0, 14);
        VCSegment out[64];
        int n = vc_build_connections(boxes, count, view, clearance, out, 64);
        if (set) printf(",");
        printf("{\"view\":["); num(view.x); printf(","); num(view.y); printf(","); num(view.width); printf(","); num(view.height);
        printf("],\"clearance\":"); num(clearance); printf(",\"boxes\":[");
        for (int i = 0; i < count; i++) {
            if (i) printf(",");
            printf("["); num(boxes[i].x); printf(","); num(boxes[i].y); printf(","); num(boxes[i].width); printf(","); num(boxes[i].height); printf("]");
        }
        printf("],\"segments\":[");
        for (int i = 0; i < n; i++) {
            if (i) printf(",");
            printf("["); num(out[i].ax); printf(","); num(out[i].ay); printf(","); num(out[i].bx); printf(","); num(out[i].by);
            printf(",%d,%d]", out[i].from, out[i].to);
        }
        printf("]}");
    }
    printf("],\n\"fnv\":[");
    const char *texts[] = {"light", "\xE5\x85\x89", "Keep", "na\xC3\xAFve", "", "\xF0\x9F\x8C\x99", "\xD9\x85\xD8\xB3\xD8\xA7\xD8\xA1", "phrase", "the", "WWWWWWWWW"};
    double starts[] = {0, 8.2, -0.0, 10.6, 123.456, 1e-300, 3.0000000000000004, 59.99, 1e9, 0.1};
    for (int i = 0; i < 10; i++) {
        if (i) printf(",");
        uint64_t text_hash = fnv(texts[i], 1469598103934665603ull);
        uint64_t visual = fnv(texts[i], bits(starts[i]) ^ 1469598103934665603ull);
        printf("{\"text\":\"");
        for (const unsigned char *p = (const unsigned char *)texts[i]; *p; p++) printf("%c", *p);
        printf("\",\"start\":"); num(starts[i]);
        printf(",\"hash\":\"%llu\",\"visual\":\"%llu\"", (unsigned long long)text_hash, (unsigned long long)visual);
        printf(",\"metrics\":[");
        for (int index = 0; index < 4; index++) {
            uint64_t value = text_hash ^ (uint64_t)((int64_t)index * 2654435761ll);
            if (index) printf(",");
            printf("[%d,\"%llu\",%llu,%llu,%llu,%llu,%lld]", index, (unsigned long long)value,
                (unsigned long long)((value / 100) % 5), (unsigned long long)((value / 10000) % 3),
                (unsigned long long)((value / 700000) % 100), (unsigned long long)((value / 7) % 100),
                (long long)((int64_t)((value / 1000) % 9) - 4));
        }
        printf("],\"spoken\":[%llu,%llu,%llu,%llu,%llu,%llu]}", (unsigned long long)(text_hash % 3),
            (unsigned long long)((text_hash / 300) % 100), (unsigned long long)(text_hash % 4),
            (unsigned long long)(text_hash % 2), (unsigned long long)(text_hash % 1021), (unsigned long long)(visual % 997));
    }
    printf("]\n}\n");
    return 0;
}
