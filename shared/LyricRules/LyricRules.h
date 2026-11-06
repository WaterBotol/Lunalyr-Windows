#ifndef LUNALYR_LYRIC_RULES_H
#define LUNALYR_LYRIC_RULES_H

#if defined(_WIN32) && defined(LUNALYR_LYRIC_RULES_DLL)
#define LL_API __declspec(dllexport)
#else
#define LL_API
#endif

enum LLRecordingMatch {
    LL_RECORDING_MATCH = 0,
    LL_TITLE_MISMATCH = 1,
    LL_ARTIST_MISMATCH = 2,
    LL_MISSING_IDENTITY = 3,
    LL_DURATION_MISMATCH = 4
};

// Callers supply title and artist confidence after normalizing provider metadata.
LL_API int ll_recording_match(double title_confidence, double artist_confidence,
                              int compare_artist, int length_required, int compare_length,
                              double track_duration, double candidate_duration);

// The caller has already established that the neighbouring cues have matching
// text and that only the second cue has native word timing.
LL_API int ll_mixed_timing_copy(double line_start, double line_end, int inferred_end,
                               int multiple_syllables, double word_start, double vocal_end);

#endif
