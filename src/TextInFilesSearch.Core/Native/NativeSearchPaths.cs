using System;
using System.IO;

namespace TextInFilesSearch.Native;

/// <summary>
/// Where the native_search index lives - see
/// docs/adr/ADR-011-in-folder-index-location.md (supersedes ADR-007) for
/// why. Kept separate from
/// <see cref="TextInFilesSearch.Services.NativeSearchService"/> so there's
/// exactly one place this decision is made, not one computed inline
/// wherever a caller happens to construct the service.
/// </summary>
public static class NativeSearchPaths
{
    /// <summary>
    /// Name of the index subfolder created at the root of whatever folder
    /// is being searched. Dot-prefixed (matches the convention of
    /// tool-owned folders like <c>.git</c>) so it reads as "not a document
    /// in this folder" at a glance. Also the exact string
    /// <see cref="TextInFilesSearch.ViewModels.MainViewModel.BuildSettings"/>
    /// adds to <c>ExcludeFolders</c> automatically - both sides must use
    /// this same constant, not a hand-typed copy of it, or the exclusion
    /// and the actual folder name can silently drift apart.
    /// </summary>
    public const string IndexFolderName = ".native-search-index";

    /// <summary>
    /// <paramref name="searchPath"/>\<see cref="IndexFolderName"/> - the
    /// index lives inside the folder it indexes (ADR-011), not a global
    /// per-machine location, so a "Fast re-search" only ever searches
    /// documents that came from indexing *this* folder tree, and deleting
    /// the folder naturally takes its index with it.
    /// </summary>
    public static string GetIndexDirectory(string searchPath) =>
        Path.Combine(searchPath, IndexFolderName);

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
