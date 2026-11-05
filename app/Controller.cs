using System.Diagnostics;
using System.Text.Json;
using Lunalyr.Core;
using Microsoft.Web.WebView2.Core;
using Microsoft.Win32;

namespace Lunalyr;

internal sealed class Controller : ApplicationContext
{
    private readonly System.Windows.Forms.Timer poll = new() { Interval = 500 };
    private readonly System.Windows.Forms.Timer frames = new() { Interval = 66 };
    private readonly NotifyIcon trayIcon = new() { Visible = true, Text = "Lunalyr", Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath) };
    private readonly ContextMenuStrip menu = new();
    private readonly Media media = new();
    private readonly Loopback audio = new();
    private readonly HttpClient http = Program.NewHttp();
    private readonly LyricLookup lookup;
    private readonly string root;
    private readonly string mediaFolder;
    private readonly string lyricsFolder;
    private readonly string? smoke;
    private readonly string settingsFile;
    private readonly string choicesFile;
    private readonly Dictionary<string, Choice> choices;
    private readonly Dictionary<string, (Surface Surface, Rectangle Bounds)> desktop = [];
    private readonly Dictionary<string, Lyrics[]> cache = [];
    private readonly HashSet<string> blocked = [];
    private readonly System.Runtime.CompilerServices.ConditionalWeakTable<Lyrics, Lyrics> readings = new();
    private readonly System.Diagnostics.Stopwatch clock = System.Diagnostics.Stopwatch.StartNew();
    private CoreWebView2Environment? environment;
    private Surface? settings;
    private Surface? tray;
    private Surface? onboarding;
    private Preferences preferences;
    private Playback playback = new();
    private Lyrics[] candidates = [];
    private CancellationTokenSource? request;
    private bool polling;
    private bool syncingDesktop;
    private bool locked;
    private bool shuttingDown;
    private bool audioWanted;
    private long lastDesktopTry = -10_000;
    private string notice = "";
    private string lookupStatus = "";
    public int ExitCode { get; private set; }

    internal Controller(string data, string? smokeDirectory, bool silent)
    {
        root = data;
        smoke = smokeDirectory;
        mediaFolder = Path.Combine(root, "Media");
        lyricsFolder = Path.Combine(root, "Lyrics");
        Directory.CreateDirectory(mediaFolder);
        Directory.CreateDirectory(lyricsFolder);
        settingsFile = Path.Combine(root, "settings.json");
        choicesFile = Path.Combine(root, "song-choices.json");
        preferences = Store.Read(settingsFile, new Preferences());
        choices = Store.Read(choicesFile, new Dictionary<string, Choice>());
        lookup = new LyricLookup(http);

        menu.Items.Add("Settings…", null, (_, _) => ShowSettings());
        menu.Items.Add("Show onboarding…", null, (_, _) => ShowOnboarding());
        menu.Items.Add("Quit Lunalyr", null, (_, _) => ExitThread());
        trayIcon.ContextMenuStrip = menu;
        trayIcon.MouseClick += (_, args) => { if (args.Button == MouseButtons.Left) ShowTray(); };
        poll.Tick += async (_, _) => await Tick();
        frames.Tick += (_, _) => FeedAudio();
        SystemEvents.SessionSwitch += SessionSwitch;

        var launch = new System.Windows.Forms.Timer { Interval = 1 };
        launch.Tick += async (_, _) =>
        {
            launch.Dispose();
            try
            {
                environment = await CoreWebView2Environment.CreateAsync(userDataFolder: Path.Combine(root, "WebView2"));
                settings = await Make("settings");
                settings.Hotkey += Hotkey;
                RegisterHotkeys(settings);
                if (smoke is not null) await RunSmoke();
                else
                {
                    if (!preferences.Onboarded) ShowOnboarding();
                    else if (!silent) ShowSettings();
                    poll.Start();
                    frames.Start();
                    await Tick();
                }
            }
            catch (Exception error)
            {
                Log(error.ToString());
                if (smoke is null) MessageBox.Show(error is WebView2RuntimeNotFoundException
                    ? "Install the Microsoft Edge WebView2 Runtime and reopen Lunalyr." : error.Message, "Lunalyr");
                ExitCode = 1;
                ExitThread();
            }
        };
        launch.Start();
    }

    private async Task<Surface> Make(string role)
    {
        var view = new Surface(role);
        view.Message += Receive;
        _ = view.Handle;
        try { await view.Open(environment!, mediaFolder); return view; }
        catch { view.Dispose(); throw; }
    }

    private void ShowSettings()
    {
        if (settings is null || settings.IsDisposed) return;
        if (tray is not null) tray.Hide();
        if (!settings.Visible)
        {
            var screen = Screen.FromPoint(Cursor.Position).WorkingArea;
            settings.Size = new Size(Math.Min(1120, screen.Width), Math.Min(800, screen.Height));
            settings.Location = new Point(screen.X + (screen.Width - settings.Width) / 2, screen.Y + (screen.Height - settings.Height) / 2);
            settings.FormClosing -= HideWindow;
            settings.FormClosing += HideWindow;
            settings.Show();
        }
        if (settings.WindowState == FormWindowState.Minimized) settings.WindowState = FormWindowState.Normal;
        settings.Activate();
    }

    private async void ShowTray()
    {
        if (environment is null || shuttingDown) return;
        if (tray?.Visible == true) { tray.Hide(); return; }
        try
        {
            if (tray is null)
            {
                tray = await Make("tray");
                tray.FormClosing += HideWindow;
                tray.Deactivate += (_, _) => tray.Hide();
            }
            var area = Screen.FromPoint(Cursor.Position).WorkingArea;
            int width = Math.Min(344, area.Width), height = Math.Min(572, area.Height);
            tray.Bounds = new Rectangle(Math.Clamp(Cursor.Position.X - width / 2, area.Left, area.Right - width),
                Math.Clamp(Cursor.Position.Y - height - 8, area.Top, area.Bottom - height), width, height);
            tray.Show();
            tray.Activate();
        }
        catch (Exception error) { Log(error.Message); }
    }

    private async void ShowOnboarding()
    {
        if (environment is null || shuttingDown) return;
        try
        {
            onboarding ??= await Make("onboarding");
            onboarding.FormClosing -= HideWindow;
            onboarding.FormClosing += HideWindow;
            var area = Screen.FromPoint(Cursor.Position).WorkingArea;
            onboarding.ClientSize = new Size(Math.Min(640, area.Width - 32), Math.Min(540, area.Height - 60));
            onboarding.Location = new Point(area.X + (area.Width - onboarding.Width) / 2, area.Y + (area.Height - onboarding.Height) / 2);
            onboarding.Show();
            onboarding.Activate();
        }
        catch (Exception error) { Log(error.Message); }
    }

    private void HideWindow(object? sender, FormClosingEventArgs args)
    {
        if (shuttingDown) return;
        args.Cancel = true;
        (sender as Form)?.Hide();
    }

    private void SessionSwitch(object sender, SessionSwitchEventArgs args)
    {
        if (args.Reason is not (SessionSwitchReason.SessionLock or SessionSwitchReason.SessionUnlock)) return;
        if (settings is { IsDisposed: false, IsHandleCreated: true } view)
            view.BeginInvoke(() => { locked = args.Reason == SessionSwitchReason.SessionLock; _ = UpdateDesktop(); });
    }

    private async Task Tick()
    {
        if (polling || shuttingDown) return;
        polling = true;
        try
        {
            var previous = playback;
            try { playback = await media.Read(preferences.Player); }
            catch (Exception error) when (error is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
            {
                notice = "Windows media sessions are unavailable. Restart the player or Lunalyr.";
                playback = new Playback();
            }
            if (shuttingDown) return;
            if (previous.Track?.Key != playback.Track?.Key) ReloadLyrics();
            SendAll("playback", new { playback, players = media.Players });
            await UpdateDesktop();
            var power = SystemInformation.PowerStatus.PowerLineStatus;
            bool lowPower = preferences.LowPower || (preferences.RespectBattery && power == PowerLineStatus.Offline);
            bool wantsAudio = !locked && !lowPower && preferences.Audio
                && (preferences.Background == "visualiser" || preferences.TextColour == "musicReactive");
            if (audioWanted != wantsAudio)
            {
                audioWanted = wantsAudio;
                if (wantsAudio) audio.Start();
                else { audio.Stop(); SendAll("audio", new AudioFrame()); }
            }
            SendAll("status", new { notice, audio = audio.Status, lowPower, desktop = desktop.Count, lookup = lookupStatus });
        }
        catch (Exception error) { Log(error.ToString()); }
        finally { polling = false; }
    }

    private async Task UpdateDesktop()
    {
        if (syncingDesktop || shuttingDown) return;
        syncingDesktop = true;
        try
        {
            bool enabled = preferences.WallpaperEnabled && preferences.Background != "desktop";
            var screens = Screen.AllScreens;
            var shell = enabled ? Desktop.Discover(false) : null;
            if (enabled && shell is null && Environment.TickCount64 - lastDesktopTry > 5_000)
            {
                lastDesktopTry = Environment.TickCount64;
                shell = Desktop.Discover(true);
            }
            foreach (string name in desktop.Keys.ToArray())
            {
                var entry = desktop[name];
                var screen = screens.FirstOrDefault(item => item.DeviceName == name);
                if (shell is null || screen?.Bounds != entry.Bounds || entry.Surface.IsDisposed
                    || !Desktop.Exists(entry.Surface.Handle) || Desktop.Parent(entry.Surface.Handle) != shell.Value.Parent)
                {
                    desktop.Remove(name);
                    entry.Surface.Dispose();
                }
            }
            if (shell is null) return;
            foreach (var screen in screens)
            {
                bool show = !locked && (!preferences.PauseCovered || !Desktop.Covered(screen.Bounds));
                if (!desktop.TryGetValue(screen.DeviceName, out var entry))
                {
                    if (!show) continue;
                    var surface = await Make("wallpaper");
                    if (shuttingDown) { surface.Dispose(); return; }
                    if (!Desktop.Attach(surface.Handle, shell.Value, screen.Bounds)) { surface.Dispose(); continue; }
                    desktop[screen.DeviceName] = entry = (surface, screen.Bounds);
                    Log("Attached wallpaper on " + screen.DeviceName);
                }
                if (show != entry.Surface.Visible) entry.Surface.Visible = show;
                entry.Surface.Send("covered", !show);
            }
        }
        finally { syncingDesktop = false; }
    }

    private Choice CurrentChoice() => playback.Track is { } track && choices.TryGetValue(track.Key, out var choice) ? choice : new Choice();

    private void ReloadLyrics()
    {
        request?.Cancel();
        candidates = [];
        lookupStatus = playback.Track is null ? "" : "Looking for lyrics…";
        SendLyrics();
        if (playback.Track is not { } track) return;
        request = new CancellationTokenSource();
        _ = FindLyrics(track, request, preferences);
    }

    private async Task FindLyrics(Track track, CancellationTokenSource run, Preferences options)
    {
        var token = run.Token;
        try
        {
            var choice = CurrentChoice();
            string cacheKey = track.Key + "\n" + JsonSerializer.Serialize(new
            {
                options.LyricsFolder, options.Sources, options.DisabledSources, options.PreferWords, options.LookupPolicy
            }, Store.Json);
            if (choice.Path is { } name && Path.GetFileName(name) == name)
            {
                var imported = LyricLookup.ReadFile(Path.Combine(lyricsFolder, name), track.Duration, "import");
                if (imported is not null) { candidates = [imported]; lookupStatus = ""; SendLyrics(); return; }
            }
            if (!cache.TryGetValue(cacheKey, out var found))
            {
                found = await lookup.Find(track, options, lyricsFolder, token, foundSoFar =>
                {
                    if (token.IsCancellationRequested || shuttingDown) return;
                    candidates = foundSoFar;
                    lookupStatus = "";
                    SendLyrics();
                });
                if (token.IsCancellationRequested || shuttingDown) return;
                if (found.Length > 0)
                {
                    if (cache.Count >= 80) cache.Remove(cache.Keys.First());
                    cache[cacheKey] = found;
                }
            }
            candidates = found;
            lookupStatus = candidates.Length > 0 ? "" : "No matching lyrics";
            SendLyrics();
        }
        catch (OperationCanceledException) when (token.IsCancellationRequested) { }
        catch (Exception error)
        {
            Log(error.Message);
            if (!token.IsCancellationRequested) { lookupStatus = "Lyrics could not be loaded"; SendLyrics(); }
        }
        finally { if (ReferenceEquals(request, run)) request = null; run.Dispose(); }
    }

    private void SendLyrics() => SendAll("lyrics", LyricState());

    private object LyricState()
    {
        var choice = CurrentChoice();
        var selected = candidates.FirstOrDefault(row => row.Source == choice.Source) ?? candidates.FirstOrDefault();
        if (selected is not null && preferences.Reading != "off") selected = readings.GetValue(selected, WindowsReadings.Add);
        return new { selected, offset = choice.Offset, saved = choice.Path is not null || choice.Source is not null,
            sources = candidates.Select(row => new { row.Source, row.Quality }), lookup = lookupStatus };
    }

    private IEnumerable<Surface> Surfaces() => new[] { settings, tray, onboarding }.OfType<Surface>().Concat(desktop.Values.Select(value => value.Surface));
    private void SendAll<T>(string type, T data)
    {
        foreach (var surface in Surfaces()) surface.Send(type, data);
    }

    private void FeedAudio()
    {
        if (!audioWanted) return;
        bool pcm = preferences.Visualiser is "wavefield" or "wavefieldSphere" or "wavefieldOriginal" or "wavecore";
        SendAll("audio", audio.Sample(pcm));
    }

    private void RegisterHotkeys(Surface view)
    {
        (int Id, uint Key, string Label)[] keys = [(1, 0x4C, "L"), (2, 0x50, "P"), (3, 0xBC, ","), (4, 0xBE, ".")];
        foreach (var key in keys)
            if (!Desktop.Hotkey(view.Handle, key.Id, key.Key)) blocked.Add(key.Label);
        if (blocked.Count > 0) notice = "Some shortcuts are in use: " + string.Join(", ", blocked);
    }

    private void Hotkey(int id)
    {
        if (id == 1) Apply(preferences with { LyricsVisible = !preferences.LyricsVisible });
        if (id == 2) Apply(preferences with { WallpaperEnabled = !preferences.WallpaperEnabled });
        if (id is 3 or 4) ChangeChoice(choice => choice with { Offset = Math.Clamp(choice.Offset + (id == 3 ? -.1 : .1), -10, 10) });
    }

    private async void Receive(Surface view, JsonElement message)
    {
        try
        {
            if (message.ValueKind != JsonValueKind.Object || !message.TryGetProperty("type", out var kind)
                || kind.ValueKind != JsonValueKind.String) return;
            message.TryGetProperty("value", out var value);
            string? command = kind.GetString();
            if (command == "ready")
            {
                view.Send("preferences", preferences);
                view.Send("playback", new { playback, players = media.Players });
                view.Send("lyrics", LyricState());
                return;
            }
            if (view.Role == "wallpaper") return;
            switch (command)
            {
                case "preferences":
                    var incoming = value.Deserialize<Preferences>(Store.Json);
                    if (incoming is not null) Apply(incoming with { Wallpaper = preferences.Wallpaper, LyricsFolder = preferences.LyricsFolder });
                    break;
                case "control": await media.Control(value.GetString() ?? ""); await Tick(); break;
                case "offset":
                    if (value.TryGetDouble(out double seconds) && double.IsFinite(seconds))
                        ChangeChoice(choice => choice with { Offset = Math.Clamp(seconds, -10, 10) });
                    break;
                case "source":
                    string? source = value.ValueKind == JsonValueKind.String ? value.GetString() : null;
                    if (source is null || candidates.Any(row => row.Source == source))
                    {
                        ChangeChoice(choice => choice with { Source = source, Path = null });
                        if (source is null) ReloadLyrics();
                    }
                    break;
                case "refresh": cache.Clear(); ReloadLyrics(); break;
                case "wallpaper": ImportWallpaper(view); break;
                case "lyricsFile": ImportLyrics(view); break;
                case "lyricsFolder": PickLyricsFolder(view); break;
                case "onboarding": ShowOnboarding(); break;
                case "settings": ShowSettings(); if (value.ValueKind == JsonValueKind.String) settings?.Send("page", value.GetString()); break;
                case "done": Apply(preferences with { Onboarded = true }); onboarding?.Hide(); ShowSettings(); break;
                case "reconnect": if (audioWanted) { audio.Stop(); audio.Start(); } break;
                case "copyText": if (value.GetString() is { Length: > 0 and <= 12000 } text) Clipboard.SetText(text); break;
                case "copyCard":
                    CopyCard(value.GetString());
                    view.Send("notice", "Animated lyric card copied.");
                    break;
                case "releases": Process.Start(new ProcessStartInfo("https://github.com/WaterBotol/Lunalyr-Windows/releases") { UseShellExecute = true }); break;
                case "quit": ExitThread(); break;
            }
        }
        catch (Exception error)
        {
            Log(error.Message);
            view.Send("notice", error.Message);
        }
    }

    private void Apply(Preferences next)
    {
        next = next with
        {
            Fps = next.Fps is 15 or 30 or 60 ? next.Fps : 30,
            Sources = next.Sources.Where(LyricLookup.Names.ContainsKey).Distinct().ToArray(),
            DisabledSources = next.DisabledSources.Where(LyricLookup.Names.ContainsKey).Distinct().ToArray()
        };
        var previous = preferences;
        if (previous.Startup != next.Startup)
        {
            using var run = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run");
            if (next.Startup) run.SetValue("LunalyrWindows2", $"\"{Application.ExecutablePath}\" --autostart");
            else run.DeleteValue("LunalyrWindows2", false);
        }
        Store.Write(settingsFile, next);
        preferences = next;
        SendAll("preferences", preferences);
        if (previous.Player != next.Player) _ = Tick();
        if (previous.Reading != next.Reading) SendLyrics();
        if (previous.LyricsFolder != next.LyricsFolder || previous.PreferWords != next.PreferWords
            || previous.LookupPolicy != next.LookupPolicy || !previous.Sources.SequenceEqual(next.Sources)
            || !previous.DisabledSources.SequenceEqual(next.DisabledSources)) ReloadLyrics();
        if (!next.Audio && audioWanted) { audioWanted = false; audio.Stop(); SendAll("audio", new AudioFrame()); }
        _ = UpdateDesktop();
    }

    private void ChangeChoice(Func<Choice, Choice> update)
    {
        if (playback.Track is not { } track) return;
        choices[track.Key] = update(CurrentChoice());
        Store.Write(choicesFile, choices);
        SendLyrics();
    }

    private void ImportLyrics(Surface owner)
    {
        if (playback.Track is not { } track) { owner.Send("notice", "Play a song before importing lyrics."); return; }
        using var dialog = new OpenFileDialog { Filter = "Lyrics|*.lrc;*.ttml;*.xml;*.yrc;*.txt;*.yaml;*.yml", CheckFileExists = true };
        if (dialog.ShowDialog(owner) != DialogResult.OK) return;
        if (LyricLookup.ReadFile(dialog.FileName, track.Duration) is null) throw new FormatException("This file has no readable lyric lines.");
        string name = Guid.NewGuid().ToString("N") + Path.GetExtension(dialog.FileName).ToLowerInvariant();
        File.Copy(dialog.FileName, Path.Combine(lyricsFolder, name));
        if (playback.Track?.Key != track.Key)
        {
            owner.Send("notice", "The song changed. The imported file was saved without attaching it.");
            return;
        }
        ChangeChoice(choice => choice with { Path = name, Source = "import" });
        ReloadLyrics();
    }

    private void PickLyricsFolder(Surface owner)
    {
        using var dialog = new FolderBrowserDialog { Description = "Choose a local lyric export folder" };
        if (dialog.ShowDialog(owner) == DialogResult.OK) { cache.Clear(); Apply(preferences with { LyricsFolder = dialog.SelectedPath }); }
    }

    private void ImportWallpaper(Surface owner)
    {
        using var dialog = new OpenFileDialog { Filter = "Images and videos|*.png;*.jpg;*.jpeg;*.webp;*.gif;*.mp4;*.webm", CheckFileExists = true };
        if (dialog.ShowDialog(owner) != DialogResult.OK) return;
        string extension = Path.GetExtension(dialog.FileName).ToLowerInvariant();
        if (extension is not (".png" or ".jpg" or ".jpeg" or ".webp" or ".gif" or ".mp4" or ".webm"))
            throw new FormatException("Choose a supported image or video.");
        if (new FileInfo(dialog.FileName).Length > 500_000_000) throw new FormatException("Choose a wallpaper under 500 MB.");
        string name = Guid.NewGuid().ToString("N") + extension;
        File.Copy(dialog.FileName, Path.Combine(mediaFolder, name));
        Apply(preferences with { Wallpaper = name, Background = "custom" });
    }

    private void CopyCard(string? base64)
    {
        if (base64 is null || base64.Length > 12_000_000) throw new FormatException("The card exceeds the clipboard size limit.");
        byte[] bytes = Convert.FromBase64String(base64);
        if (bytes.Length < 7 || !bytes.AsSpan(0, 6).SequenceEqual("GIF89a"u8)) throw new FormatException("The card is not a GIF.");
        var folder = Path.Combine(root, "Cards");
        Directory.CreateDirectory(folder);
        foreach (string stale in Directory.EnumerateFiles(folder, "*.gif").Where(path => File.GetLastWriteTimeUtc(path) < DateTime.UtcNow.AddDays(-7)))
            File.Delete(stale);
        string path = Path.Combine(folder, "Lunalyr-" + Guid.NewGuid().ToString("N") + ".gif");
        File.WriteAllBytes(path, bytes);
        var data = new DataObject();
        data.SetFileDropList(new System.Collections.Specialized.StringCollection { path });
        using var stream = new MemoryStream(bytes);
        using var image = Image.FromStream(stream);
        using var bitmap = new Bitmap(image);
        data.SetImage(bitmap);
        Clipboard.SetDataObject(data, true, 4, 100);
    }

    private async Task RunSmoke()
    {
        Directory.CreateDirectory(smoke!);
        ShowSettings();
        preferences = preferences with { Onboarded = true, PauseCovered = false };
        for (int i = 0; !settings!.Ready && i < 120; i++) await Task.Delay(100);
        if (!settings!.Ready) throw new InvalidOperationException("Settings did not start its WebView.");
        using var testPlayer = new Windows.Media.Playback.MediaPlayer();
        testPlayer.CommandManager.IsEnabled = false;
        var controls = testPlayer.SystemMediaTransportControls;
        controls.IsEnabled = controls.IsPlayEnabled = controls.IsPauseEnabled = true;
        controls.DisplayUpdater.Type = Windows.Media.MediaPlaybackType.Music;
        controls.DisplayUpdater.MusicProperties.Title = "Golden Hour Static";
        controls.DisplayUpdater.MusicProperties.Artist = "Lunar Tides";
        controls.DisplayUpdater.Update();
        controls.PlaybackStatus = Windows.Media.MediaPlaybackStatus.Playing;
        controls.UpdateTimelineProperties(new Windows.Media.SystemMediaTransportControlsTimelineProperties
        {
            StartTime = TimeSpan.Zero, MinSeekTime = TimeSpan.Zero,
            EndTime = TimeSpan.FromSeconds(45), MaxSeekTime = TimeSpan.FromSeconds(45), Position = TimeSpan.FromSeconds(5.6)
        });

        for (int i = 0; i < 24; i++)
        {
            var live = await media.Read("");
            if (live.Track?.Title == "Golden Hour Static" && live.Track.Artist == "Lunar Tides" && live.Playing)
            {
                playback = live with { Playing = false };
                break;
            }
            await Task.Delay(250);
        }
        if (playback.Track is null) throw new InvalidOperationException("Windows did not report the test media session.");
        Log($"Windows media session read: {playback.Track.Title}, {playback.Position:F2}s.");
        const string fixture = "[00:00.00]<00:00.00>Every <00:02.00>word, <00:04.00>in time\n[00:08.00]with the music";
        candidates = [new Lyrics("local", LyricParser.Lrc(fixture, 45))];
        SendAll("playback", new { playback, players = new[] { "Fixture" } });
        SendLyrics();
        await UpdateDesktop();
        if (desktop.Count == 0) throw new InvalidOperationException("Wallpaper did not attach to Explorer.");
        for (int i = 0; desktop.Values.Any(entry => !entry.Surface.Ready) && i < 120; i++) await Task.Delay(100);
        if (desktop.Values.Any(entry => !entry.Surface.Ready)) throw new InvalidOperationException("Wallpaper WebView did not start.");
        await Task.Delay(1800);
        await settings.SavePreview(Path.Combine(smoke!, "settings.png"));
        settings.Hide();
        await Task.Delay(650);
        var wallpaper = desktop.Values.First().Surface;
        await wallpaper.SavePreview(Path.Combine(smoke!, "wallpaper.png"));
        var bounds = SystemInformation.VirtualScreen;
        using var screenshot = new Bitmap(bounds.Width, bounds.Height);
        using var graphics = Graphics.FromImage(screenshot);
        graphics.CopyFromScreen(bounds.Left, bounds.Top, 0, 0, bounds.Size);
        screenshot.Save(Path.Combine(smoke!, "desktop.png"), System.Drawing.Imaging.ImageFormat.Png);
        Log("SMOKE OK: settings, wallpaper, fixture lyric timeline");
        ExitThread();
    }

    private void Log(string message) => File.AppendAllText(Path.Combine(smoke ?? root, "lunalyr.log"),
        DateTimeOffset.Now.ToString("O") + " " + message + Environment.NewLine);

    protected override void ExitThreadCore()
    {
        shuttingDown = true;
        poll.Stop();
        frames.Stop();
        request?.Cancel();
        SystemEvents.SessionSwitch -= SessionSwitch;
        trayIcon.Visible = false;
        if (settings is { IsHandleCreated: true })
            foreach (int id in new[] { 1, 2, 3, 4 }) Desktop.RemoveHotkey(settings.Handle, id);
        foreach (var surface in Surfaces().ToArray()) surface.Dispose();
        trayIcon.Dispose();
        menu.Dispose();
        poll.Dispose();
        frames.Dispose();
        audio.Dispose();
        http.Dispose();
        base.ExitThreadCore();
    }
}
