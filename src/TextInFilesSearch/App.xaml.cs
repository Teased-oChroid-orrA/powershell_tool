using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.UI.Xaml;
using TextInFilesSearch.Views;

namespace TextInFilesSearch;

/// <summary>
/// Application entry point. Registers the legacy code-pages provider once at
/// startup (needed for the Windows-1252 fallback in TextExtractionService -
/// .NET no longer includes legacy encodings by default) and creates the
/// single main window. This app makes no network calls anywhere, at startup
/// or otherwise - see docs/architecture.md.
///
/// Global exception handlers are wired up before anything else runs. This
/// exists because of a real field report: the app was launched on a clean
/// Windows machine for the first time ever (CI only checks that publish
/// output files exist, never launches the GUI - see
/// docs/deployment.md's "clean-machine test procedure") and nothing
/// happened - no window, no taskbar entry, no error. Without a handler, any
/// unhandled exception during App/MainWindow construction fails exactly
/// like that: silently. This can't catch every possible failure (a truly
/// native module-load failure - e.g. a missing Visual C++ Redistributable
/// dependency of Microsoft.WindowsAppRuntime.dll's own native components -
/// can terminate the process before any managed code, including this
/// handler, ever runs), but it turns every failure it *can* catch from
/// silent into visible, which is strictly better than the alternative.
/// </summary>
public partial class App : Application
{
    private Window? _window;

    public App()
    {
        AppDomain.CurrentDomain.UnhandledException += OnAppDomainUnhandledException;
        UnhandledException += OnXamlUnhandledException;

        Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();
        _window.Activate();
    }

    // Explicitly System.UnhandledExceptionEventArgs, not
    // Microsoft.UI.Xaml.UnhandledExceptionEventArgs (also in scope via the
    // `using Microsoft.UI.Xaml;` above) - the two types share a name and
    // the bare name is ambiguous (CS0104) without qualifying one of them.
    private static void OnAppDomainUnhandledException(object sender, System.UnhandledExceptionEventArgs e) =>
        ReportFatalError(e.ExceptionObject as Exception, "AppDomain.UnhandledException");

    private void OnXamlUnhandledException(object sender, Microsoft.UI.Xaml.UnhandledExceptionEventArgs e)
    {
        ReportFatalError(e.Exception, "Application.UnhandledException");
        // Deliberately not setting e.Handled = true: swallowing a XAML-layer
        // exception and continuing risks leaving the app in a half-broken
        // state that's harder to diagnose than a clean, reported exit.
    }

    /// <summary>
    /// Writes the exception to a log file next to the executable and shows
    /// a plain Win32 message box - not a WinUI ContentDialog, which needs a
    /// working XAML window/dispatcher that may not exist at the point this
    /// fires (e.g. an exception during MainWindow's own construction).
    /// MessageBoxW has no such dependency, so it's the one reporting
    /// mechanism that can still work even when almost everything else
    /// about the app's startup has already failed.
    /// </summary>
    private static void ReportFatalError(Exception? ex, string source)
    {
        string details = ex?.ToString() ?? "(no exception object was provided)";

        try
        {
            string logPath = Path.Combine(AppContext.BaseDirectory, "crash.log");
            File.AppendAllText(logPath, $"{DateTime.Now:O} [{source}]{Environment.NewLine}{details}{Environment.NewLine}{Environment.NewLine}");
        }
        catch
        {
            // Logging failed too (e.g. read-only install location) - the
            // message box below is still attempted regardless.
        }

        try
        {
            string message =
                $"TextInFilesSearch hit an unexpected error during startup and needs to close.\n\n" +
                $"Details were written to crash.log next to the executable.\n\n" +
                $"{ex?.GetType().Name}: {ex?.Message}";
            NativeMessageBox.Show(message, "TextInFilesSearch - Error");
        }
        catch
        {
            // If even this fails, there is nothing further this process can
            // do to report the problem to the user.
        }
    }
}

/// <summary>
/// Bare Win32 MessageBoxW P/Invoke - deliberately not routed through any
/// WinUI/XAML API. See <see cref="App.ReportFatalError"/> for why: this
/// needs to work even when the XAML runtime itself is the thing that
/// failed to initialize.
/// </summary>
internal static class NativeMessageBox
{
    private const uint MB_OK = 0x00000000;
    private const uint MB_ICONERROR = 0x00000010;
    private const uint MB_SYSTEMMODAL = 0x00001000;

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = false)]
    private static extern int MessageBoxW(IntPtr hWnd, string text, string caption, uint type);

    public static void Show(string text, string caption) =>
        MessageBoxW(IntPtr.Zero, text, caption, MB_OK | MB_ICONERROR | MB_SYSTEMMODAL);
}
