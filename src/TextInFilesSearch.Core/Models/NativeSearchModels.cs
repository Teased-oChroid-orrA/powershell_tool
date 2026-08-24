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
/// this record's PascalCase property names - renaming a property here means
/// renaming its Rust counterpart in native-search/src/engine.rs too.
/// </summary>
public sealed record NativeSearchHit(
    string Id,
    string Path,
    string Filename,
    string Extension,
    string Title,
    long ModifiedUnix,
    long CreatedUnix,
    long Size,
    float Score)
{
    public DateTime Modified => DateTimeOffset.FromUnixTimeSeconds(ModifiedUnix).UtcDateTime;
    public DateTime Created => DateTimeOffset.FromUnixTimeSeconds(CreatedUnix).UtcDateTime;
}
