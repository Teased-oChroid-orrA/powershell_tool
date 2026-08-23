using System;
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
/// </summary>
public partial class App : Application
{
    private Window? _window;

    public App()
    {
        Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();
        _window.Activate();
    }
}
