using System;
using System.Collections.Generic;
using System.Text;
using System.Text.Json;
using TextInFilesSearch.Models;
using TextInFilesSearch.Native;

namespace TextInFilesSearch.Services;

/// <summary>
/// Thrown when a native_search call fails. <see cref="Status"/> mirrors
/// native-search/src/error.rs's <c>NsStatus</c>; <see cref="Message"/> is
/// whatever native_search's thread-local last-error slot held at the time.
/// </summary>
public sealed class NativeSearchException : Exception
{
    internal NativeSearchException(NativeSearchStatus status, string message)
        : base($"{status}: {message}")
    {
        Status = status.ToString();
    }

    public string Status { get; }
}

/// <summary>
/// Safe C# wrapper over native_search.dll (see native-search/src/ffi.rs and
/// docs/ffi.md). Owns one Tantivy index handle for its lifetime; callers use
/// <see cref="IDisposable"/> (or a <c>using</c> block) to release it
/// deterministically rather than waiting on finalization, since the
/// underlying <see cref="NativeSearchHandle"/> is a <see cref="System.Runtime.InteropServices.SafeHandle"/>
/// and will still be released on GC as a last resort.
///
/// This is a new, separate search capability alongside the existing
/// per-run line scan in <see cref="SearchOrchestrator"/>/<see cref="MatchingEngine"/>
/// - see ADR-001. It does not replace them in this phase.
/// </summary>
public sealed class NativeSearchService : IDisposable
{
    private static readonly JsonSerializerOptions HitJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
    };

    private readonly NativeSearchHandle _handle;
    private bool _disposed;

    /// <summary>
    /// Opens (or creates) an index at <paramref name="indexDirectory"/>,
    /// which must already exist - this class does no filesystem
    /// provisioning of its own (see native-search/src/engine.rs's
    /// <c>open_or_create</c> doc comment for why).
    /// </summary>
    public NativeSearchService(string indexDirectory)
    {
        int status = NativeSearchInterop.ns_create(indexDirectory, out NativeSearchHandle handle);
        if (status != (int)NativeSearchStatus.Ok || handle.IsInvalid)
        {
            handle.Dispose();
            throw NewException(status);
        }
        _handle = handle;
    }

    /// <summary>
    /// Indexes (or re-indexes, if <see cref="NativeDocumentInput.Id"/>
    /// already exists) one document. Call <see cref="Commit"/> afterward for
    /// the change to become searchable.
    /// </summary>
    public void IndexDocument(NativeDocumentInput document)
    {
        ThrowIfDisposed();
        byte[] bodyBytes = Encoding.UTF8.GetBytes(document.Body);
        int status = NativeSearchInterop.ns_index_document(
            _handle,
            document.Id,
            document.Path,
            document.FileName,
            document.Extension,
            document.Title,
            new DateTimeOffset(document.Modified.ToUniversalTime()).ToUnixTimeSeconds(),
            new DateTimeOffset(document.Created.ToUniversalTime()).ToUnixTimeSeconds(),
            document.Size,
            bodyBytes,
            (nuint)bodyBytes.Length);

        if (status != (int)NativeSearchStatus.Ok)
        {
            throw NewException(status);
        }
    }

    public void DeleteDocument(string id)
    {
        ThrowIfDisposed();
        int status = NativeSearchInterop.ns_delete_document(_handle, id);
        if (status != (int)NativeSearchStatus.Ok)
        {
            throw NewException(status);
        }
    }

    /// <summary>Commits pending index/delete calls and makes them searchable.</summary>
    public void Commit()
    {
        ThrowIfDisposed();
        int status = NativeSearchInterop.ns_commit(_handle);
        if (status != (int)NativeSearchStatus.Ok)
        {
            throw NewException(status);
        }
    }

    /// <summary>
    /// Pass <paramref name="cancellationToken"/> and call its
    /// <see cref="NativeSearchCancellationToken.Cancel"/> from another
    /// thread to abort a long-running search (issue #2 Section 17) - it
    /// surfaces here as a <see cref="NativeSearchException"/> whose
    /// <see cref="NativeSearchException.Status"/> is <c>"Cancelled"</c>.
    /// </summary>
    public IReadOnlyList<NativeSearchHit> Search(string query, int limit = 50, NativeSearchCancellationToken? cancellationToken = null)
    {
        ThrowIfDisposed();
        int status = NativeSearchInterop.ns_search(
            _handle,
            query,
            (uint)limit,
            cancellationToken?.Handle,
            out IntPtr buffer,
            out nuint len);
        if (status != (int)NativeSearchStatus.Ok)
        {
            throw NewException(status);
        }

        byte[] json = NativeSearchInterop.CopyAndFreeBuffer(buffer, len);
        if (json.Length == 0)
        {
            return Array.Empty<NativeSearchHit>();
        }

        return JsonSerializer.Deserialize<List<NativeSearchHit>>(json, HitJsonOptions)
            ?? (IReadOnlyList<NativeSearchHit>)Array.Empty<NativeSearchHit>();
    }

    private static NativeSearchException NewException(int status)
    {
        var typedStatus = (NativeSearchStatus)status;
        return new NativeSearchException(typedStatus, NativeSearchInterop.TakeLastError());
    }

    private void ThrowIfDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(NativeSearchService));
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        _handle.Dispose();
    }
}
