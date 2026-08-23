using System;
using System.Collections.Generic;

namespace TextInFilesSearch.Models;

public enum MatchMode
{
    AnyLine,
    AllInFile,
    Proximity
}

public enum ExcludeScope
{
    Line,
    File
}

public enum GroupByMode
{
    Created,
    Modified,
    None
}

/// <summary>
/// Every user-configurable setting for a search run. This is the C# equivalent
/// of the PowerShell script's parameter block, and is also the shape used to
/// fingerprint the incremental cache (see CacheService).
/// </summary>
public sealed class SearchSettings
{
    public string SearchPath { get; set; } = string.Empty;
    public string OutputFolder { get; set; } = string.Empty;
    public string? OutputName { get; set; }

    public List<string> Filters { get; set; } = new();
    public List<string> ExcludeFilters { get; set; } = new();

    public MatchMode MatchMode { get; set; } = MatchMode.AnyLine;
    public int ProximityLines { get; set; } = 5;
    public ExcludeScope ExcludeScope { get; set; } = ExcludeScope.Line;

    public bool WholeWord { get; set; }
    public bool UseRegex { get; set; }

    public GroupByMode GroupBy { get; set; } = GroupByMode.Created;

    /// <summary>Null means "use the built-in default list" - mirrors the PowerShell script's convention.</summary>
    public List<string>? Extensions { get; set; }

    public List<string> ExcludeFolders { get; set; } = new();

    public bool IncludeHidden { get; set; }
    public double MaxFileSizeMB { get; set; } = 50;
    public int MaxEmbedLines { get; set; } = 4000;
    public int PdfTimeoutSeconds { get; set; } = 15;

    public bool ExportCsv { get; set; }
    public bool ExportJson { get; set; }
    public bool OpenReportWhenDone { get; set; }

    public bool Parallel { get; set; }
    public int ThrottleLimit { get; set; } = 5;

    public string? CacheFilePath { get; set; }
    public bool DryRun { get; set; }

    public int MaxRetries { get; set; } = 3;
    public int RetryDelayMs { get; set; } = 250;
    public int FileTimeoutSeconds { get; set; } = 30;

    public static readonly IReadOnlyList<string> DefaultExtensions = new List<string>
    {
        ".txt", ".log", ".csv", ".tsv", ".md", ".ini", ".cfg", ".conf",
        ".xml", ".json", ".yaml", ".yml", ".htm", ".html",
        ".ps1", ".psm1", ".bat", ".cmd", ".py", ".js", ".ts", ".cs",
        ".java", ".sql", ".rtf", ".docx", ".pptx", ".pdf"
    };
}

/// <summary>One matched line within one file, with one line of context on each side.</summary>
public sealed class LineHit
{
    public int LineNumber { get; init; }
    public string? Before { get; init; }
    public string MatchLine { get; init; } = string.Empty;
    public string? After { get; init; }
    public List<string> MatchedFilters { get; init; } = new();
}

public enum FileSearchStatus
{
    Hit,
    NoHit,
    TooLarge,
    Binary,
    ReadError,
    ExcludedFile,
    ModeExcluded,
    UnexpectedError
}

/// <summary>
/// The uniform result of processing exactly one file, whether it ended up with
/// hits or was skipped for some reason. Mirrors the PowerShell script's
/// Invoke-SingleFileSearch return object, which is what let that logic run
/// identically whether sequential or parallel.
/// </summary>
public sealed class FileSearchResult
{
    public string FullName { get; init; } = string.Empty;
    public FileSearchStatus Status { get; set; } = FileSearchStatus.NoHit;
    public List<LineHit> Hits { get; set; } = new();
    public DateTime Created { get; set; }
    public DateTime Modified { get; set; }
    public long FileLength { get; set; }
    public List<string> LinesCache { get; set; } = new();
    public int TotalLineCount { get; set; }
    public int? ProximityMinRange { get; set; }
    public bool LowConfidencePdf { get; set; }
    public string? ErrorMessage { get; set; }
}

/// <summary>Snapshot of counters accumulated across a whole run, for the summary panel and console/UI reporting.</summary>
public sealed class SearchRunSummary
{
    public int FilesSearched { get; set; }
    public int SkippedTooLarge { get; set; }
    public int SkippedBinary { get; set; }
    public int SkippedReadError { get; set; }
    public int SkippedByExclude { get; set; }
    public int SkippedByMode { get; set; }
    public int SkippedUnexpectedError { get; set; }
    public int CacheReused { get; set; }
    public List<(string FullName, string Message)> Warnings { get; } = new();
}

/// <summary>
/// A single live progress update pushed from the search engine to the UI
/// thread. This is the direct answer to "the PDF isn't actually hung, it's
/// working, but there's no visibility" - every long-running step (a slow PDF,
/// a locked-file retry, a per-file timeout countdown) reports through this
/// instead of the UI going silent until the whole file finishes.
/// </summary>
public sealed class SearchProgressReport
{
    public int FilesCompleted { get; set; }
    public int TotalFiles { get; set; }
    public int HitsSoFar { get; set; }

    /// <summary>The file currently being processed (sequential mode) or most recently started (parallel mode).</summary>
    public string? CurrentFileName { get; set; }

    /// <summary>Free-form status for the current file, e.g. "Extracting PDF text - 42 streams scanned" or "Retrying (locked by another program), attempt 2 of 3".</summary>
    public string? CurrentFileStatus { get; set; }

    public TimeSpan CurrentFileElapsed { get; set; }

    public bool IsDryRun { get; set; }

    /// <summary>Every file currently being processed right now (one entry in sequential mode, up to ThrottleLimit in parallel mode).</summary>
    public IReadOnlyList<InFlightFileStatus> InFlightFiles { get; set; } = Array.Empty<InFlightFileStatus>();
}
