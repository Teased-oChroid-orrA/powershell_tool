using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using TextInFilesSearch.Models;

namespace TextInFilesSearch.Services;

public sealed class SearchRunResult
{
    public List<FileSearchResult> FileResults { get; } = new();
    public SearchRunSummary Summary { get; } = new();
    public bool WasDryRun { get; set; }
    public List<FileInfo>? DryRunCandidates { get; set; }
}

/// <summary>
/// Coordinates a full search run: enumerate, optionally consult the cache,
/// process files (sequential or throttled-parallel), and report live progress
/// throughout - including per-file activity so a slow PDF is visibly still
/// working rather than the whole run looking frozen, which was the single
/// biggest complaint about the PowerShell version's console-only reporting.
/// </summary>
public sealed class SearchOrchestrator
{
    public async Task<SearchRunResult> RunAsync(
        SearchSettings settings,
        IProgress<SearchProgressReport>? progress,
        CancellationToken cancellationToken)
    {
        var runResult = new SearchRunResult();

        var extensionSet = new HashSet<string>(
            settings.Extensions ?? SearchSettings.DefaultExtensions,
            StringComparer.OrdinalIgnoreCase);
        bool searchAllExtensions = extensionSet.Count == 1 && extensionSet.Contains("*");

        var allFiles = FileReaderService.EnumerateFilesSafely(settings.SearchPath, settings.IncludeHidden, out int enumErrors);

        var candidates = new List<FileInfo>();
        foreach (var f in allFiles)
        {
            if (settings.ExcludeFolders.Any(ex => f.FullName.Contains(ex, StringComparison.OrdinalIgnoreCase)))
                continue;
            if (!searchAllExtensions && !extensionSet.Contains(f.Extension))
                continue;
            candidates.Add(f);
        }

        if (settings.DryRun)
        {
            runResult.WasDryRun = true;
            runResult.DryRunCandidates = candidates;
            progress?.Report(new SearchProgressReport { IsDryRun = true, TotalFiles = candidates.Count });
            return runResult;
        }

        // ---- Incremental cache ----
        var cache = new CacheService();
        Dictionary<string, CachedFileEntry>? priorCache = null;
        if (!string.IsNullOrWhiteSpace(settings.CacheFilePath))
        {
            priorCache = cache.TryLoad(settings.CacheFilePath!, settings);
        }

        var toProcess = new List<FileInfo>();
        var reused = new List<FileSearchResult>();

        foreach (var f in candidates)
        {
            if (priorCache is not null &&
                priorCache.TryGetValue(f.FullName, out var entry) &&
                entry.Length == f.Length &&
                entry.LastWriteTimeTicks == f.LastWriteTimeUtc.Ticks)
            {
                reused.Add(entry.ToFileSearchResult(f.FullName));
                runResult.Summary.CacheReused++;
            }
            else
            {
                toProcess.Add(f);
            }
        }

        var matchState = CompiledMatchState.Build(settings);
        var maxBytes = (long)(settings.MaxFileSizeMB * 1024 * 1024);

        var fresh = new List<FileSearchResult>();
        int filesCompleted = 0;
        int hitsSoFar = 0;
        var inFlight = new ConcurrentDictionary<string, InFlightFileStatus>();

        void ReportProgress()
        {
            progress?.Report(new SearchProgressReport
            {
                FilesCompleted = filesCompleted,
                TotalFiles = toProcess.Count,
                HitsSoFar = hitsSoFar,
                InFlightFiles = inFlight.Values.ToList()
            });
        }

        if (settings.Parallel && toProcess.Count > 0)
        {
            using var throttle = new SemaphoreSlim(Math.Max(1, settings.ThrottleLimit));
            var tasks = toProcess.Select(async file =>
            {
                await throttle.WaitAsync(cancellationToken);
                try
                {
                    var result = await ProcessOneFileAsync(file, settings, matchState, maxBytes, inFlight, cancellationToken);
                    Interlocked.Increment(ref filesCompleted);
                    if (result.Status == FileSearchStatus.Hit) Interlocked.Add(ref hitsSoFar, result.Hits.Count);
                    lock (fresh) { fresh.Add(result); }
                    ReportProgress();
                    return result;
                }
                finally
                {
                    throttle.Release();
                }
            }).ToList();

            // Lightweight ticker so in-flight elapsed times keep updating even
            // between file completions - this is what makes a slow PDF visibly
            // "still going" instead of the display freezing for 10+ seconds.
            using var tickerCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            var ticker = TickProgressAsync(tickerCts.Token, ReportProgress);

            await Task.WhenAll(tasks);
            tickerCts.Cancel();
            try { await ticker; } catch (OperationCanceledException) { }
        }
        else
        {
            foreach (var file in toProcess)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var result = await ProcessOneFileAsync(file, settings, matchState, maxBytes, inFlight, cancellationToken);
                filesCompleted++;
                if (result.Status == FileSearchStatus.Hit) hitsSoFar += result.Hits.Count;
                fresh.Add(result);
                ReportProgress();
            }
        }

        runResult.FileResults.AddRange(reused);
        runResult.FileResults.AddRange(fresh);

        if (!string.IsNullOrWhiteSpace(settings.CacheFilePath))
        {
            cache.Save(settings.CacheFilePath!, settings, candidates, runResult.FileResults);
        }

        foreach (var r in runResult.FileResults)
        {
            switch (r.Status)
            {
                case FileSearchStatus.TooLarge: runResult.Summary.SkippedTooLarge++; break;
                case FileSearchStatus.Binary: runResult.Summary.SkippedBinary++; runResult.Summary.FilesSearched++; break;
                case FileSearchStatus.ReadError: runResult.Summary.SkippedReadError++; break;
                case FileSearchStatus.ExcludedFile: runResult.Summary.SkippedByExclude++; runResult.Summary.FilesSearched++; break;
                case FileSearchStatus.ModeExcluded: runResult.Summary.SkippedByMode++; runResult.Summary.FilesSearched++; break;
                case FileSearchStatus.UnexpectedError:
                    runResult.Summary.SkippedUnexpectedError++;
                    runResult.Summary.Warnings.Add((r.FullName, r.ErrorMessage ?? "Unknown error"));
                    break;
                case FileSearchStatus.NoHit:
                case FileSearchStatus.Hit:
                    runResult.Summary.FilesSearched++;
                    break;
            }
        }

        return runResult;
    }

    private static async Task TickProgressAsync(CancellationToken token, Action report)
    {
        try
        {
            while (!token.IsCancellationRequested)
            {
                await Task.Delay(500, token);
                report();
            }
        }
        catch (OperationCanceledException) { /* expected on completion */ }
    }

    /// <summary>
    /// Processes exactly one file end to end: robust byte read, format-aware
    /// text extraction, line matching, AllInFile/Proximity gating. Never
    /// throws - any unexpected failure comes back as
    /// FileSearchStatus.UnexpectedError with the message attached, so one bad
    /// file can never take down a whole run.
    /// </summary>
    private static async Task<FileSearchResult> ProcessOneFileAsync(
        FileInfo file,
        SearchSettings settings,
        CompiledMatchState matchState,
        long maxBytes,
        ConcurrentDictionary<string, InFlightFileStatus> inFlight,
        CancellationToken cancellationToken)
    {
        var result = new FileSearchResult
        {
            FullName = file.FullName,
            Created = file.CreationTimeUtc,
            Modified = file.LastWriteTimeUtc,
            FileLength = file.Length
        };

        if (file.Length > maxBytes)
        {
            result.Status = FileSearchStatus.TooLarge;
            return result;
        }

        var status = new InFlightFileStatus { FileName = file.Name, StatusText = "Reading..." };
        inFlight[file.FullName] = status;
        var sw = System.Diagnostics.Stopwatch.StartNew();

        try
        {
            byte[] bytes;
            try
            {
                bytes = await FileReaderService.ReadFileBytesRobustAsync(
                    file.FullName, settings.FileTimeoutSeconds, settings.MaxRetries, settings.RetryDelayMs,
                    onRetry: args =>
                    {
                        status.StatusText = $"Locked by another program - retrying ({args.Attempt} of {args.MaxRetries})...";
                        status.ElapsedSeconds = sw.Elapsed.TotalSeconds;
                    },
                    cancellationToken);
            }
            catch (Exception ex)
            {
                result.Status = FileSearchStatus.ReadError;
                result.ErrorMessage = ex.Message;
                return result;
            }

            string ext = file.Extension.ToLowerInvariant();
            string[]? lines;
            bool lowConfidence = false;

            status.StatusText = "Extracting text...";
            status.ElapsedSeconds = sw.Elapsed.TotalSeconds;

            switch (ext)
            {
                case ".docx":
                    lines = TextExtractionService.ExtractDocxLines(bytes);
                    break;
                case ".pptx":
                    lines = TextExtractionService.ExtractPptxLines(bytes);
                    break;
                case ".pdf":
                    lines = TextExtractionService.ExtractPdfLines(
                        bytes, settings.PdfTimeoutSeconds, out _,
                        onProgress: (streamsScanned, elapsed) =>
                        {
                            status.StatusText = $"Extracting PDF text - {streamsScanned} stream(s) scanned";
                            status.ElapsedSeconds = elapsed.TotalSeconds;
                        });
                    if (lines is not null) lowConfidence = !TextExtractionService.PdfExtractionLooksReliable(lines);
                    break;
                case ".rtf":
                    lines = TextExtractionService.ExtractRtfLines(bytes);
                    break;
                default:
                    if (TextExtractionService.LooksBinary(bytes))
                    {
                        result.Status = FileSearchStatus.Binary;
                        return result;
                    }
                    string text = TextExtractionService.DecodeText(bytes);
                    lines = text.Split(new[] { "\r\n", "\n" }, StringSplitOptions.None);
                    break;
            }

            if (lines is null || lines.Length == 0)
            {
                result.Status = FileSearchStatus.ReadError;
                return result;
            }

            result.LowConfidencePdf = lowConfidence;

            status.StatusText = "Matching filters...";
            status.ElapsedSeconds = sw.Elapsed.TotalSeconds;

            MatchingEngine.ApplyLineMatching(
                lines, settings, matchState,
                out var hits, out bool excludedByFile, out bool passesMode, out int? proximityMinRange);

            if (excludedByFile)
            {
                result.Status = FileSearchStatus.ExcludedFile;
                return result;
            }

            if (hits.Count == 0)
            {
                result.Status = FileSearchStatus.NoHit;
                return result;
            }

            if (!passesMode)
            {
                result.Status = FileSearchStatus.ModeExcluded;
                return result;
            }

            result.Status = FileSearchStatus.Hit;
            result.Hits = hits;
            result.TotalLineCount = lines.Length;
            result.LinesCache = lines.Length > settings.MaxEmbedLines
                ? lines.Take(settings.MaxEmbedLines).ToList()
                : lines.ToList();
            result.ProximityMinRange = proximityMinRange;

            return result;
        }
        catch (Exception ex)
        {
            result.Status = FileSearchStatus.UnexpectedError;
            result.ErrorMessage = ex.Message;
            return result;
        }
        finally
        {
            inFlight.TryRemove(file.FullName, out _);
        }
    }
}
