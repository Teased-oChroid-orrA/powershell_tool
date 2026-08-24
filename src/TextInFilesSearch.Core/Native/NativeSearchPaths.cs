using System;
using System.IO;

namespace TextInFilesSearch.Native;

/// <summary>
/// Where the native_search index lives by default - see
/// docs/adr/ADR-007-index-persistence-location.md for why. Kept separate
/// from <see cref="TextInFilesSearch.Services.NativeSearchService"/> so
/// there's exactly one place this decision is made, not one computed
/// inline wherever a caller happens to construct the service.
/// </summary>
public static class NativeSearchPaths
{
    /// <summary>
    /// <c>%LOCALAPPDATA%\TextInFilesSearch\native-index</c> - app-owned,
    /// per-machine, regenerable state, not a user document, so it belongs
    /// under LocalApplicationData rather than anywhere the user picks.
    /// </summary>
    public static string GetDefaultIndexDirectory() =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "TextInFilesSearch",
            "native-index");

    /// <summary>
    /// Creates <paramref name="indexDirectory"/> if it doesn't already
    /// exist. <see cref="TextInFilesSearch.Native.NativeSearchHandle"/>'s
    /// underlying <c>ns_create</c> call requires the directory to already
    /// be present (ADR-001: the Rust module does no filesystem
    /// provisioning of its own) - call this first.
    /// </summary>
    public static void EnsureIndexDirectoryExists(string indexDirectory) =>
        Directory.CreateDirectory(indexDirectory);
}
