using System.Runtime.InteropServices;

namespace Lunalyr.Core;

internal static class LyricRules
{
    private const string Library = "lunalyr_lyric_rules";

    [DllImport(Library, EntryPoint = "ll_recording_match", CallingConvention = CallingConvention.Cdecl)]
    [DefaultDllImportSearchPaths(DllImportSearchPath.AssemblyDirectory | DllImportSearchPath.SafeDirectories)]
    internal static extern int RecordingMatch(double titleConfidence, double artistConfidence, int compareArtist,
        int lengthRequired, int compareLength, double trackDuration, double candidateDuration);

    [DllImport(Library, EntryPoint = "ll_mixed_timing_copy", CallingConvention = CallingConvention.Cdecl)]
    [DefaultDllImportSearchPaths(DllImportSearchPath.AssemblyDirectory | DllImportSearchPath.SafeDirectories)]
    internal static extern int MixedTimingCopy(double lineStart, double lineEnd, int inferredEnd, int multipleSyllables,
        double wordStart, double vocalEnd);
}
