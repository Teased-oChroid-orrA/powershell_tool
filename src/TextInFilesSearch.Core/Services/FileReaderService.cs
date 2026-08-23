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
    /// folders are counted and skipped, never fatal.
    /// </summary>
    public static IReadOnlyList<FileInfo> EnumerateFilesSafely(
        string rootPath,
        bool includeHidden,
        out int enumErrorCount)
    {
        var visited = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var results = new List<FileInfo>();
        var stack = new Stack<string>();
        stack.Push(rootPath);
        int errors = 0;

        while (stack.Count > 0)
        {
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

            foreach (var d in childDirs)
            {
                try
                {
                    var di = new DirectoryInfo(d);
                    if (!includeHidden && di.Attributes.HasFlag(FileAttributes.Hidden)) continue;
                    stack.Push(d);
                }
                catch
                {
                    errors++;
                }
            }
        }

        enumErrorCount = errors;
        return results;
    }
}
