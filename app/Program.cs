using System.Net;
using Microsoft.Web.WebView2.Core;

namespace Lunalyr;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);

        string? smoke = args is ["--smoke", var path] ? Path.GetFullPath(path) : null;
        using var mutex = new Mutex(true, "Local\\Lunalyr.Windows.Replacement", out bool first);
        if (!first && smoke is null)
        {
            MessageBox.Show("Lunalyr is running in the notification area.", "Lunalyr");
            return;
        }

        string data = smoke is null
            ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Lunalyr", "Windows2")
            : Path.Combine(smoke, "profile");
        Directory.CreateDirectory(data);
        try
        {
            using var controller = new Controller(data, smoke, args.Contains("--autostart"));
            Application.Run(controller);
            Environment.ExitCode = controller.ExitCode;
        }
        catch (WebView2RuntimeNotFoundException)
        {
            MessageBox.Show("Install the Microsoft Edge WebView2 Runtime and reopen Lunalyr.", "Lunalyr");
            Environment.ExitCode = 1;
        }
        catch (Exception error)
        {
            File.WriteAllText(Path.Combine(data, "startup-error.txt"), error.ToString());
            if (smoke is null) MessageBox.Show(error.Message, "Lunalyr");
            Environment.ExitCode = 1;
        }
    }

    public static HttpClient NewHttp() => new(new SocketsHttpHandler
    {
        AutomaticDecompression = DecompressionMethods.All,
        ConnectTimeout = TimeSpan.FromSeconds(4),
        AllowAutoRedirect = false
    });
}
