using System.Numerics;
using System.Runtime.InteropServices;
using NAudio.Wave;

namespace Lunalyr;

internal sealed record AudioFrame(double Bass = 0, double Mid = 0, double Treble = 0, double Level = 0,
    double Beat = 0, double Flux = 0, double[]? Spectrum = null, string? Pcm = null, int SampleRate = 0);

internal sealed class Loopback : IDisposable
{
    private const int Size = 2048;
    private readonly float[] ring = new float[Size];
    private readonly object gate = new();
    private WasapiLoopbackCapture? capture;
    private int cursor;
    private int packets;
    private long latest;
    private int rate;
    private double previous;
    private string? error;
    public string Status => error ?? (capture is null ? "Audio response is off"
        : Environment.TickCount64 - Interlocked.Read(ref latest) < 500 ? "Receiving audio" : "Waiting for system audio");

    public void Start()
    {
        if (capture is not null) return;
        error = null;
        try
        {
            var next = new WasapiLoopbackCapture();
            rate = next.WaveFormat.SampleRate;
            next.DataAvailable += Receive;
            next.RecordingStopped += Stopped;
            capture = next;
            next.StartRecording();
        }
        catch (Exception ex) when (ex is COMException or InvalidOperationException or ArgumentException)
        {
            Stop();
            error = "Audio capture unavailable. Reconnect after checking your output device.";
        }
    }

    public void Stop()
    {
        var old = capture;
        capture = null;
        if (old is not null)
        {
            old.DataAvailable -= Receive;
            old.RecordingStopped -= Stopped;
            old.Dispose();
        }
        lock (gate)
        {
            Array.Clear(ring);
            cursor = packets = 0;
            Interlocked.Exchange(ref latest, 0);
        }
        previous = 0;
        error = null;
    }

    private void Stopped(object? sender, StoppedEventArgs args)
    {
        if (args.Exception is not null) error = "Audio disconnected. Use Reconnect to open the current output.";
    }

    private void Receive(object? sender, WaveInEventArgs args)
    {
        if (sender is not WasapiLoopbackCapture source) return;
        var format = source.WaveFormat is WaveFormatExtensible extensible ? extensible.ToStandardWaveFormat() : source.WaveFormat;
        int channels = format.Channels, bytes = format.BitsPerSample / 8;
        if (channels == 0 || bytes < 2) return;
        lock (gate)
        {
            for (int frame = 0; frame + channels * bytes <= args.BytesRecorded; frame += channels * bytes)
            {
                double sum = 0;
                for (int channel = 0; channel < channels; channel++)
                {
                    int at = frame + channel * bytes;
                    sum += (bytes, format.Encoding) switch
                    {
                        (4, WaveFormatEncoding.IeeeFloat) => BitConverter.ToSingle(args.Buffer, at),
                        (4, WaveFormatEncoding.Pcm) => BitConverter.ToInt32(args.Buffer, at) / 2147483648.0,
                        (3, WaveFormatEncoding.Pcm) => ((args.Buffer[at] | args.Buffer[at + 1] << 8 | args.Buffer[at + 2] << 16) << 8 >> 8) / 8388608.0,
                        (2, WaveFormatEncoding.Pcm) => BitConverter.ToInt16(args.Buffer, at) / 32768.0,
                        _ => 0
                    };
                }
                ring[cursor++ % Size] = double.IsFinite(sum) ? (float)Math.Clamp(sum / channels, -1, 1) : 0;
                if (cursor == Size) cursor = 0;
            }
            if (args.BytesRecorded > 0) { packets++; Interlocked.Exchange(ref latest, Environment.TickCount64); }
        }
    }

    public AudioFrame Sample(bool includePcm)
    {
        if (capture is null) return new AudioFrame();
        float[] pcm = new float[Size];
        lock (gate)
        {
            if (packets == 0 || Environment.TickCount64 - Interlocked.Read(ref latest) > 220) return new AudioFrame();
            Array.Copy(ring, cursor, pcm, 0, Size - cursor);
            Array.Copy(ring, 0, pcm, Size - cursor, cursor);
        }

        var fft = pcm.Select((sample, i) => new Complex(sample * (.5 - .5 * Math.Cos(2 * Math.PI * i / (Size - 1))), 0)).ToArray();
        for (int i = 1, j = 0; i < Size; i++)
        {
            int bit = Size >> 1;
            for (; (j & bit) != 0; bit >>= 1) j ^= bit;
            j ^= bit;
            if (i < j) (fft[i], fft[j]) = (fft[j], fft[i]);
        }
        for (int len = 2; len <= Size; len *= 2)
        {
            var unit = Complex.FromPolarCoordinates(1, -2 * Math.PI / len);
            for (int start = 0; start < Size; start += len)
            {
                var phase = Complex.One;
                for (int i = 0; i < len / 2; i++, phase *= unit)
                {
                    int a = start + i, b = a + len / 2;
                    var value = fft[b] * phase;
                    fft[b] = fft[a] - value;
                    fft[a] += value;
                }
            }
        }

        double Peak(double lo, double hi)
        {
            int start = Math.Clamp((int)(lo * Size / rate), 1, Size / 2 - 1);
            int end = Math.Clamp((int)(hi * Size / rate) + 1, start + 1, Size / 2);
            double peak = 0;
            for (int i = start; i < end; i++) peak = Math.Max(peak, fft[i].Magnitude);
            return Math.Clamp(peak * 10 / Size, 0, 1);
        }

        var spectrum = Enumerable.Range(0, 48).Select(i => Peak(25 * Math.Pow(480, i / 48.0), 25 * Math.Pow(480, (i + 1) / 48.0))).ToArray();
        double level = Math.Clamp(Math.Sqrt(pcm.Sum(x => x * x) / Size) * 2.5, 0, 1);
        double flux = Math.Max(0, level - previous);
        previous = .68 * previous + .32 * level;
        string? encoded = null;
        if (includePcm)
        {
            var shorts = pcm.Select(sample => (short)Math.Round(sample * short.MaxValue)).ToArray();
            encoded = Convert.ToBase64String(MemoryMarshal.AsBytes(shorts.AsSpan()));
        }
        return new AudioFrame(Peak(25, 240), Peak(240, 2800), Peak(2800, 12000), level, Math.Min(1, flux * 5), flux,
            spectrum, encoded, rate);
    }

    public void Dispose() => Stop();
}
