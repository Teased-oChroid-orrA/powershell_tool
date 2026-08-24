using System;

namespace TextInFilesSearch.Models;

/// <summary>
/// One document to hand to <see cref="TextInFilesSearch.Services.NativeSearchService"/>.
/// Text is already-extracted (ADR-001/ADR-003) - this type carries no raw
/// file bytes and the native module does no extraction of its own.
/// </summary>
public sealed record NativeDocumentInput(
    string Id,
    string Path,
    string FileName,
    string Extension,
    string? Title,
    DateTime Modified,
    DateTime Created,
    long Size,
    string Body);

/// <summary>
/// One search hit returned from the native index. Field names/casing here
/// are deliberate: <see cref="TextInFilesSearch.Services.NativeSearchService"/>
/// deserializes native_search's JSON (serde field names, e.g.
/// <c>modified_unix</c>) via <c>JsonNamingPolicy.SnakeCaseLower</c> against
/// this type's PascalCase property names - renaming a property here means
/// renaming its Rust counterpart in native-search/src/engine.rs too.
///
/// Deliberately a plain class with settable properties, not a record -
/// bound via x:DataType in MainWindow.xaml's "Fast re-search" results
/// list, and a record here reproduced a real field bug: WinUI 3's x:Bind
/// compiler/runtime type-metadata resolution against record types (their
/// init-only properties and compiler-synthesized members) threw a bare
/// XamlParseException at MainWindow.InitializeComponent() on first actual
/// launch on Windows, with no further detail. Every other x:DataType type
/// already in this codebase (InFlightFileStatus, FileResultViewModel,
/// ExtensionOption) is a plain class for the same reason - matching that
/// established pattern, not introducing a new one, is the fix.
/// </summary>
public sealed class NativeSearchHit
{
    public string Id { get; set; } = string.Empty;
    public string Path { get; set; } = string.Empty;
    public string Filename { get; set; } = string.Empty;
    public string Extension { get; set; } = string.Empty;
    public string Title { get; set; } = string.Empty;
    public long ModifiedUnix { get; set; }
    public long CreatedUnix { get; set; }
    public long Size { get; set; }
    public float Score { get; set; }

    public DateTime Modified => DateTimeOffset.FromUnixTimeSeconds(ModifiedUnix).UtcDateTime;
    public DateTime Created => DateTimeOffset.FromUnixTimeSeconds(CreatedUnix).UtcDateTime;
}
