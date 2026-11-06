#include "LyricRules.h"
#include <math.h>

int ll_recording_match(double title_confidence, double artist_confidence,
                       int compare_artist, int length_required, int compare_length,
                       double track_duration, double candidate_duration) {
    if (!isfinite(title_confidence) || title_confidence < 0.75) return LL_TITLE_MISMATCH;
    if (compare_artist && (!isfinite(artist_confidence) || artist_confidence < 0.65))
        return LL_ARTIST_MISMATCH;

    const int has_length = isfinite(track_duration) && isfinite(candidate_duration)
        && track_duration > 0 && candidate_duration > 0;
    if (length_required && !has_length) return LL_MISSING_IDENTITY;
    if (compare_length && has_length &&
        fabs(track_duration - candidate_duration) > fmax(3, track_duration * 0.02))
        return LL_DURATION_MISMATCH;
    return LL_RECORDING_MATCH;
}

int ll_mixed_timing_copy(double line_start, double line_end, int inferred_end,
                        int multiple_syllables, double word_start, double vocal_end) {
    if (!isfinite(line_start) || !isfinite(line_end) || !isfinite(word_start) ||
        !isfinite(vocal_end) || fabs(line_start - word_start) > 0.85 || line_end <= line_start)
        return 0;
    const double overlap = fmin(line_end, vocal_end) - fmax(line_start, word_start);
    if (overlap >= 0.12) return 1;

    // A line-only copy may end exactly where the word-timed cue starts.
    // An explicit short line or a single-syllable pickup can be a real repeat.
    return inferred_end && multiple_syllables && line_start < word_start
        && fabs(line_end - word_start) < 0.06;
}
