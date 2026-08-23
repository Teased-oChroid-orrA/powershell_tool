using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using System.Threading.Tasks;

namespace TextInFilesSearch.Services;

/// <summary>
/// Read-only file access helpers: a byte reader with retry-with-backoff (for
/// files transiently locked by another program) and a hard timeout (for a
/// stalled network share), plus a recursive directory walker that tracks
/// resolved real paths to guard against a symlink/junction cycle.
///
/// This code never deletes, moves, or modifies anything - every method here
/// only ever opens files for reading.
/// </summary>
public static class FileReaderService
{
    public sealed class RetryStatusEventArgs : EventArgs
    {
        public int Attempt { get; init; }
        public int MaxRetries { get; init; }
        public string Path { get; init; } = string.Empty;
    }

    /// <summary>
    /// Reads a whole file as bytes with retry-with-backoff for transient
    /// sharing-violation errors and a hard timeout via async I/O, so a
    /// stalled network share can't block the whole run. onRetry is invoked
    /// before each retry attempt so the UI can show "locked, retrying...".
    /// </summary>
    public static async Task<byte[]> ReadFileBytesRobustAsync(
        string path,
        int timeoutSeconds,
        int maxRetries,
        int retryDelayMs,
        Action<RetryStatusEventArgs>? onRetry = null,
        CancellationToken cancellationToken = default)
    {
        int attempt = 0;
        while (true)
        {
            attempt++;
            cancellationToken.ThrowIfCancellationRequested();

            try
            {
                using var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                cts.CancelAfter(TimeSpan.FromSeconds(timeoutSeconds));

                await using var fs = new FileStream(
                    path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite,
                    bufferSize: 81920, useAsync: true);

                var buffer = new byte[fs.Length];
                int totalRead = 0;
                while (totalRead < buffer.Length)
                {
                    int read = await fs.ReadAsync(buffer.AsMemory(totalRead), cts.Token);
                    if (read == 0) break;
                    totalRead += read;
                }

                if (totalRead < buffer.Length)
                {
                    // The stream ended before delivering the byte count it reported
                    // at open time - the file was very likely truncated by another
                    // process mid-read. Throwing (rather than silently returning a
                    // zero-padded partial buffer) routes this through the same
                    // retry-then-fail path as a locked file, instead of feeding
                    // corrupt content into extraction/matching unnoticed.
                    throw new IOException($"'{path}' was truncated during read (expected {buffer.Length} byte(s), got {totalRead}) - likely modified concurrently by another process.");
                }

                return buffer;
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                throw new TimeoutException($"Timed out reading '{path}' after {timeoutSeconds} second(s).");
            }
            catch (IOException) when (attempt <= maxRetries)
            {
                onRetry?.Invoke(new RetryStatusEventArgs { Attempt = attempt, MaxRetries = maxRetries, Path = path });
                await Task.Delay(retryDelayMs * attempt, cancellationToken);
            }
            // FileNotFoundException / DirectoryNotFoundException / UnauthorizedAccessException
            // and anything else propagate immediately - retrying a genuinely
            // missing or forbidden file would just waste time.
        }
    }

    /// <summary>
    /// Manual recursive directory walk (instead of Directory.EnumerateFiles
    /// with a naive recursive option) that tracks visited real (resolved)
    /// directory paths to guard against a symlink/junction cycle. Inaccessible
    /// folders are counted and skipped, never fatal. Excluded directories are
    /// pruned from the walk itself (not filtered out of the result afterward)
    /// so a huge excluded tree (node_modules, .git) is never actually
    /// descended into. Cancellable and reports periodic progress so a large
    /// or slow (network-share) tree never looks hung with no feedback.
    /// </summary>
    public static IReadOnlyList<FileInfo> EnumerateFilesSafely(
        string rootPath,
        bool includeHidden,
        IReadOnlyList<string>? excludeFolders,
        CancellationToken cancellationToken,
        Action<int>? onProgress,
        out int enumErrorCount)
    {
        var visited = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var results = new List<FileInfo>();
        var stack = new Stack<string>();
        stack.Push(rootPath);
        int errors = 0;

        var excludes = excludeFolders ?? Array.Empty<string>();
        var sw = System.Diagnostics.Stopwatch.StartNew();
        var lastProgressReport = TimeSpan.Zero;

        while (stack.Count > 0)
        {
            cancellationToken.ThrowIfCancellationRequested();

            string dir = stack.Pop();

            string resolvedDir;
            try
            {
                resolvedDir = new DirectoryInfo(dir).FullName;
            }
            catch
            {
                errors++;
                continue;
            }

            if (!visited.Add(resolvedDir))
            {
                continue; // already visited this real directory - breaks any cycle
            }

            string[] childDirs;
            string[] childFiles;
            try
            {
                childDirs = Directory.GetDirectories(dir);
                childFiles = Directory.GetFiles(dir);
            }
            catch
            {
                errors++;
                continue;
            }

            foreach (var f in childFiles)
            {
                try
                {
                    var fi = new FileInfo(f);
                    if (!includeHidden && fi.Attributes.HasFlag(FileAttributes.Hidden)) continue;
                    results.Add(fi);
                }
                catch
                {
                    errors++;
                }
            }

            if (onProgress is not null && (sw.Elapsed - lastProgressReport).TotalMilliseconds >= 200)
            {
                onProgress(results.Count);
                lastProgressReport = sw.Elapsed;
            }

            foreach (var d in childDirs)
            {
                try
                {
                    var di = new DirectoryInfo(d);
                    if (!includeHidden && di.Attributes.HasFlag(FileAttributes.Hidden)) continue;
                    if (excludes.Count > 0 && IsExcludedDirectory(di.Name, di.FullName, excludes)) continue;
                    stack.Push(d);
                }
                catch
                {
                    errors++;
                }
            }
        }

        onProgress?.Invoke(results.Count);
        enumErrorCount = errors;
        return results;
    }

    /// <summary>
    /// Matches a directory against ExcludeFolders by whole path segment, not
    /// raw substring - a raw `fullPath.Contains(ex)` check (the previous
    /// approach) let excluding "bin" also exclude any path merely containing
    /// "bin" as a substring elsewhere, e.g. "C:\Users\robin\Documents". A
    /// plain folder name (no separator) must match a whole segment exactly;
    /// a path-like exclude term (contains a separator) must match a
    /// contiguous run of segments, so excluding "src/bin" still works as a
    /// sub-path exclusion without falling back to substring matching.
    /// </summary>
    private static bool IsExcludedDirectory(string directoryName, string fullPath, IReadOnlyList<string> excludeFolders)
    {
        char[] separators = { Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar };

        foreach (var raw in excludeFolders)
        {
            if (string.IsNullOrWhiteSpace(raw)) continue;
            string trimmed = raw.Trim().TrimEnd(separators);
            if (trimmed.Length == 0) continue;

            if (trimmed.IndexOfAny(separators) < 0)
            {
                if (string.Equals(directoryName, trimmed, StringComparison.OrdinalIgnoreCase)) return true;
                continue;
            }

            var exSegments = trimmed.Split(separators, StringSplitOptions.RemoveEmptyEntries);
            var pathSegments = fullPath.Split(separators, StringSplitOptions.RemoveEmptyEntries);
            for (int i = 0; i <= pathSegments.Length - exSegments.Length; i++)
            {
                bool allMatch = true;
                for (int j = 0; j < exSegments.Length; j++)
                {
                    if (!string.Equals(pathSegments[i + j], exSegments[j], StringComparison.OrdinalIgnoreCase))
                    {
                        allMatch = false;
                        break;
                    }
                }
                if (allMatch) return true;
            }
        }

        return false;
    }
}
