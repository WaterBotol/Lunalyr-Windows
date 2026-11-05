using System.Runtime.InteropServices;

namespace Lunalyr;

internal static class Desktop
{
    internal readonly record struct Layer(nint Parent, nint After);
    internal static Layer? Discover(bool request)
    {
        nint progman = FindWindow("Progman", null);
        if (progman == 0) return null;
        Layer? Find()
        {
            nint icons = FindWindowEx(progman, 0, "SHELLDLL_DefView", null);
            nint worker = FindWindowEx(progman, 0, "WorkerW", null);
            if (icons != 0 && worker != 0 && (GetWindowLongPtr(progman, -20).ToInt64() & 0x00200000) != 0)
                return new Layer(progman, icons);
            Layer? legacy = null;
            EnumWindows((window, _) =>
            {
                if (FindWindowEx(window, 0, "SHELLDLL_DefView", null) == 0) return true;
                nint parent = FindWindowEx(0, window, "WorkerW", null);
                if (parent == 0 || !IsWindowVisible(parent)) return true;
                legacy = new Layer(parent, 1);
                return false;
            }, 0);
            return legacy;
        }
        var layer = Find();
        if (layer is not null || !request) return layer;
        // Explorer exposes no supported live wallpaper API. Use its shell-local
        // WorkerW message once when neither established window layout exists.
        SendMessageTimeout(progman, 0x052C, 0xD, 1, 2, 650, out _);
        return Find();
    }

    internal static bool Attach(nint window, Layer layer, Rectangle screen)
    {
        if (!IsWindow(layer.Parent) || !IsWindow(window)) return false;
        long style = GetWindowLongPtr(window, -16).ToInt64();
        SetWindowLongPtr(window, -16, new nint((style & ~0x80000000L) | 0x40000000L));
        if (layer.After != 1)
        {
            var ex = GetWindowLongPtr(window, -20).ToInt64();
            SetWindowLongPtr(window, -20, new nint(ex | 0x80000));
            if (!SetLayeredWindowAttributes(window, 0, 255, 2)) return false;
        }
        SetParent(window, layer.Parent);
        if (GetParent(window) != layer.Parent) return false;
        EnableWindow(window, false);
        var point = new Point { X = screen.Left, Y = screen.Top };
        if (!ScreenToClient(layer.Parent, ref point)) return false;
        return SetWindowPos(window, layer.After, point.X, point.Y, screen.Width, screen.Height, 0x0010 | 0x0020);
    }

    internal static bool Covered(Rectangle screen)
    {
        nint active = GetForegroundWindow();
        if (active == 0 || active == GetShellWindow() || !IsWindowVisible(active)
            || !GetWindowRect(active, out var bounds)) return false;
        var window = Rectangle.FromLTRB(bounds.Left, bounds.Top, bounds.Right, bounds.Bottom);
        var overlap = Rectangle.Intersect(screen, window);
        if (overlap.Width < screen.Width * .90 || overlap.Height < screen.Height * .88) return false;
        if (IsZoomed(active)) return true;
        return window.Left <= screen.Left + 2 && window.Top <= screen.Top + 2
            && window.Right >= screen.Right - 2 && window.Bottom >= screen.Bottom - 2;
    }

    internal static bool Exists(nint handle) => IsWindow(handle);
    internal static nint Parent(nint handle) => GetParent(handle);
    internal static bool Hotkey(nint handle, int id, uint key) => RegisterHotKey(handle, id, 0x4003, key);
    internal static void RemoveHotkey(nint handle, int id) => UnregisterHotKey(handle, id);

    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left, Top, Right, Bottom; }
    private delegate bool WindowCallback(nint window, nint data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern nint FindWindow(string cls, string? name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern nint FindWindowEx(nint parent, nint after, string cls, string? name);
    [DllImport("user32.dll")] private static extern bool EnumWindows(WindowCallback callback, nint data);
    [DllImport("user32.dll")] private static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] private static extern bool EnableWindow(nint window, bool enable);
    [DllImport("user32.dll")] private static extern bool SetLayeredWindowAttributes(nint window, uint colour, byte alpha, uint flags);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint window);
    [DllImport("user32.dll")] private static extern bool IsZoomed(nint window);
    [DllImport("user32.dll")] private static extern nint GetParent(nint window);
    [DllImport("user32.dll")] private static extern nint GetShellWindow();
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool GetWindowRect(nint window, out Rect rect);
    [DllImport("user32.dll")] private static extern nint SetParent(nint window, nint parent);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] private static extern nint GetWindowLongPtr(nint window, int index);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] private static extern nint SetWindowLongPtr(nint window, int index, nint value);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(nint window, nint after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] private static extern bool ScreenToClient(nint window, ref Point point);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern nint SendMessageTimeout(nint window, uint msg, nint w, nint l, uint flags, uint timeout, out nint result);
    [DllImport("user32.dll")] private static extern bool RegisterHotKey(nint window, int id, uint modifiers, uint key);
    [DllImport("user32.dll")] private static extern bool UnregisterHotKey(nint window, int id);
}
