using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using TextInFilesSearch.Models;

namespace TextInFilesSearch.Services;

/// <summary>One cached file's prior result, keyed by full path in <see cref="CacheFile"/>.</summary>
public sealed class CachedFileEntry
{
    public long Length { get; set; }
    public long LastWriteTimeTicks { get; set; }
    public FileSearchStatus Status { get; set; }
    public List<LineHit> Hits { get; set; } = new();
    public DateTime Created { get; set; }
    public DateTime Modified { get; set; }
    public List<string> LinesCache { get; set; } = new();
    public int TotalLineCount { get; set; }
    public int? ProximityMinRange { get; set; }
    public bool LowConfidencePdf { get; set; }
    public string? ErrorMessage { get; set; }

    public FileSearchResult ToFileSearchResult(string fullName) => new()
    {
        FullName = fullName,
        Status = Status,
        Hits = Hits,
        Created = Created,
        Modified = Modified,
        FileLength = Length,
        LinesCache = LinesCache,
        TotalLineCount = TotalLineCount,
        ProximityMinRange = ProximityMinRange,
        LowConfidencePdf = LowConfidencePdf,
        ErrorMessage = ErrorMessage
    };
}

internal sealed class CacheFile
{
    public string Fingerprint { get; set; } = string.Empty;
    public Dictionary<string, CachedFileEntry> Entries { get; set; } = new();
}

/// <summary>
/// A small JSON cache mapping each file's path to its last search result,
/// fingerprinted by the settings that affect matching (filters, mode, etc).
/// A file whose size and modified time haven't changed since the fingerprint
/// last matched is reused untouched instead of being re-read - the single
/// biggest speed win for repeated searches over the same large folder.
/// </summary>
public sealed class CacheService
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = false,
        Converters = { new JsonStringEnumConverter() }
    };

    public static string ComputeFingerprint(SearchSettings settings)
    {
        var fp = new
        {
            settings.Filters,
            settings.ExcludeFilters,
            settings.MatchMode,
            settings.ProximityLines,
            settings.ExcludeScope,
            settings.WholeWord,
            settings.UseRegex,
            settings.MaxFileSizeMB
        };
        return JsonSerializer.Serialize(fp, JsonOptions);
    }

    /// <summary>Returns null if the cache file doesn't exist, can't be read, or was built with different settings (a full rescan is then correct and expected).</summary>
    public Dictionary<string, CachedFileEntry>? TryLoad(string cacheFilePath, SearchSettings settings)
    {
        if (!File.Exists(cacheFilePath)) return null;

        try
        {
            string json = File.ReadAllText(cacheFilePath);
            var cache = JsonSerializer.Deserialize<CacheFile>(json, JsonOptions);
            if (cache is null) return null;

            string currentFingerprint = ComputeFingerprint(settings);
            if (cache.Fingerprint != currentFingerprint) return null;

            return cache.Entries;
        }
        catch
        {
            // A corrupt or unreadable cache file just means we start fresh -
            // never a fatal error, and the file gets overwritten at the end.
            return null;
        }
    }

    public void Save(string cacheFilePath, SearchSettings settings, IReadOnlyList<FileInfo> candidates, IReadOnlyList<FileSearchResult> allResults)
    {
        var candidateByPath = candidates.ToDictionary(f => f.FullName, f => f, StringComparer.OrdinalIgnoreCase);

        var entries = new Dictionary<string, CachedFileEntry>();
        foreach (var r in allResults)
        {
            if (!candidateByPath.TryGetValue(r.FullName, out var fi)) continue;

            entries[r.FullName] = new CachedFileEntry
            {
                Length = fi.Length,
                LastWriteTimeTicks = fi.LastWriteTimeUtc.Ticks,
                Status = r.Status,
                Hits = r.Hits,
                Created = r.Created,
                Modified = r.Modified,
                LinesCache = r.LinesCache,
                TotalLineCount = r.TotalLineCount,
                ProximityMinRange = r.ProximityMinRange,
                LowConfidencePdf = r.LowConfidencePdf,
                ErrorMessage = r.ErrorMessage
            };
        }

        var cacheFile = new CacheFile
        {
            Fingerprint = ComputeFingerprint(settings),
            Entries = entries
        };

        try
        {
            string json = JsonSerializer.Serialize(cacheFile, JsonOptions);
            File.WriteAllText(cacheFilePath, json);
        }
        catch
        {
            // Failing to write the cache should never fail the search itself -
            // it just means next run starts from scratch again.
        }
    }
}
