using System;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TextInFilesSearch.Models;
using TextInFilesSearch.ViewModels;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace TextInFilesSearch.Views;

/// <summary>
/// Code-behind for the single main window. Folder picking and "open the
/// report" are the only two things here that genuinely need a WinUI/Win32
/// window handle (via WinRT interop) - everything else is delegated to
/// MainViewModel, which has no WinUI dependency and is unit tested
/// separately (see tests/TextInFilesSearch.Tests).
/// </summary>
public sealed partial class MainWindow : Window
{
    public MainViewModel ViewModel { get; }

    public MatchMode[] MatchModeOptions { get; } = (MatchMode[])Enum.GetValues(typeof(MatchMode));
    public ExcludeScope[] ExcludeScopeOptions { get; } = (ExcludeScope[])Enum.GetValues(typeof(ExcludeScope));
    public GroupByMode[] GroupByOptions { get; } = (GroupByMode[])Enum.GetValues(typeof(GroupByMode));

    public MainWindow()
    {
        InitializeComponent();
        Title = "Text In Files Search";
        SetWindowIcon();

        ViewModel = new MainViewModel(
            browseSearchFolder: BrowseForFolderAsync,
            browseOutputFolder: BrowseForFolderAsync,
            openReport: OpenReport);
    }

    /// <summary>
    /// ApplicationIcon in the .csproj bakes an icon into the .exe's Win32
    /// resources (what File Explorer shows for the file), but a WinUI 3
    /// window's own title-bar/taskbar icon is a separate, runtime-only
    /// setting - AppWindow.SetIcon needs an actual file path, so the same
    /// .ico is also copied to the output/publish folder as Content (see the
    /// .csproj) and loaded from there.
    /// </summary>
    private void SetWindowIcon()
    {
        try
        {
            string iconPath = Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico");
            if (!File.Exists(iconPath)) return;

            var hwnd = WindowNative.GetWindowHandle(this);
            var windowId = Win32Interop.GetWindowIdFromWindow(hwnd);
            AppWindow.GetFromWindowId(windowId)?.SetIcon(iconPath);
        }
        catch
        {
            // A missing/unloadable icon is cosmetic, never worth failing startup over.
        }
    }

    /// <summary>
    /// WinUI's FolderPicker needs a window handle for unpackaged desktop apps
    /// (there's no implicit "current window" the way there is in packaged/
    /// UWP-style apps) - this is the standard interop pattern for that.
    /// </summary>
    private async Task<string?> BrowseForFolderAsync()
    {
        var picker = new FolderPicker
        {
            SuggestedStartLocation = PickerLocationId.ComputerFolder
        };
        picker.FileTypeFilter.Add("*");

        var hwnd = WindowNative.GetWindowHandle(this);
        InitializeWithWindow.Initialize(picker, hwnd);

        var folder = await picker.PickSingleFolderAsync();
        return folder?.Path;
    }

    /// <summary>
    /// Opens the finished report with whatever application is associated
    /// with .html files - the same "hand off to the OS default app" behavior
    /// as the original tool, just via .NET's process launcher instead of
    /// PowerShell's Invoke-Item.
    /// </summary>
    private static void OpenReport(string path)
    {
        try
        {
            Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
        }
        catch
        {
            // Opening the report is a convenience, not a critical operation -
            // if the OS can't find an associated app, the file itself still
            // exists at the reported path and the user can open it manually.
        }
    }
}
