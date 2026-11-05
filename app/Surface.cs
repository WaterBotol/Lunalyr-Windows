using System.Text.Json;
using Lunalyr.Core;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.WinForms;

namespace Lunalyr;

internal sealed class Surface : Form
{
    public const string Origin = "https://lunalyr.local/";
    private readonly WebView2 web = new() { Dock = DockStyle.Fill, DefaultBackgroundColor = Color.FromArgb(24, 24, 27) };
    private readonly bool wallpaper;
    private bool ready;
    public event Action<Surface, JsonElement>? Message;
    public event Action<int>? Hotkey;
    public bool Ready => ready;

    public Surface(string role)
    {
        Role = role;
        wallpaper = role == "wallpaper";
        Text = "Lunalyr";
        BackColor = Color.FromArgb(24, 24, 27);
        FormBorderStyle = wallpaper || role == "tray" ? FormBorderStyle.None : FormBorderStyle.Sizable;
        ShowInTaskbar = !wallpaper && role != "tray";
        StartPosition = FormStartPosition.Manual;
        Controls.Add(web);
    }

    public string Role { get; }
    protected override bool ShowWithoutActivation => wallpaper;

    protected override CreateParams CreateParams
    {
        get
        {
            var options = base.CreateParams;
            if (wallpaper) options.ExStyle |= 0x08000000 | 0x00000080 | 0x00000020;
            if (Role == "tray") options.ExStyle |= 0x00000080;
            return options;
        }
    }

    public async Task Open(CoreWebView2Environment environment, string media)
    {
        await web.EnsureCoreWebView2Async(environment);
        if (IsDisposed) return;
        var core = web.CoreWebView2;
        core.Settings.AreDefaultContextMenusEnabled = false;
        core.Settings.AreDevToolsEnabled = false;
        core.Settings.AreHostObjectsAllowed = false;
        core.Settings.IsStatusBarEnabled = false;
        core.Settings.IsZoomControlEnabled = false;
        core.SetVirtualHostNameToFolderMapping("lunalyr.local", Path.Combine(AppContext.BaseDirectory, "Web"),
            CoreWebView2HostResourceAccessKind.DenyCors);
        core.SetVirtualHostNameToFolderMapping("media.lunalyr.local", media, CoreWebView2HostResourceAccessKind.DenyCors);
        core.NavigationStarting += (_, args) =>
        {
            if (!args.Uri.StartsWith(Origin, StringComparison.Ordinal)) args.Cancel = true;
        };
        core.NewWindowRequested += (_, args) => args.Handled = true;
        core.PermissionRequested += (_, args) => args.State = CoreWebView2PermissionState.Deny;
        core.DownloadStarting += (_, args) => args.Cancel = true;
        core.WebMessageReceived += (_, args) =>
        {
            if (!args.Source.StartsWith(Origin, StringComparison.Ordinal)) return;
            string json = args.WebMessageAsJson;
            if (json.Length > 12_100_000) return;
            using var parsed = JsonDocument.Parse(json);
            if (parsed.RootElement.TryGetProperty("type", out var type) && type.GetString() == "ready") ready = true;
            Message?.Invoke(this, parsed.RootElement.Clone());
        };
        core.Navigate(Origin + "index.html?surface=" + Role);
    }

    public void Send<T>(string type, T value)
    {
        if (ready && !IsDisposed && web.CoreWebView2 is { } core)
            core.PostWebMessageAsJson(JsonSerializer.Serialize(new { type, value }, Store.Json));
    }

    public async Task SavePreview(string path)
    {
        await using var image = File.Create(path);
        await web.CoreWebView2.CapturePreviewAsync(CoreWebView2CapturePreviewImageFormat.Png, image);
    }

    protected override void WndProc(ref Message message)
    {
        if (message.Msg == 0x0312) Hotkey?.Invoke((int)message.WParam);
        if (wallpaper && message.Msg == 0x0084) { message.Result = -1; return; }
        base.WndProc(ref message);
    }
}
