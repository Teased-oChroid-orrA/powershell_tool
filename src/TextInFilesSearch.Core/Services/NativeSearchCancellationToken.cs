using System;
using TextInFilesSearch.Native;

namespace TextInFilesSearch.Services;

/// <summary>
/// Cancels an in-flight <see cref="NativeSearchService.Search"/> call
/// (issue #2 Section 17). Independent of any <see cref="NativeSearchService"/>
/// instance - create one, pass it to <c>Search</c>, and call
/// <see cref="Cancel"/> from another thread (e.g. in response to a .NET
/// <see cref="System.Threading.CancellationToken"/> firing) to abort it.
///
/// Cancellation is checked before the search starts and again before each
/// index segment is scanned - real, working cancellation for the common
/// multi-segment case, but not a guarantee of instant mid-scan
/// interruption of a single large segment (see native-search/src/engine.rs's
/// <c>CancellableCollector</c> doc comment).
/// </summary>
public sealed class NativeSearchCancellationToken : IDisposable
{
    internal readonly NativeSearchCancellationHandle Handle;
    private bool _disposed;

    public NativeSearchCancellationToken()
    {
        int status = NativeSearchInterop.ns_cancel_token_create(out NativeSearchCancellationHandle handle);
        if (status != (int)NativeSearchStatus.Ok || handle.IsInvalid)
        {
            handle.Dispose();
            throw new NativeSearchException((NativeSearchStatus)status, NativeSearchInterop.TakeLastError());
        }
        Handle = handle;
    }

    public void Cancel()
    {
        ThrowIfDisposed();
        int status = NativeSearchInterop.ns_cancel_token_cancel(Handle);
        if (status != (int)NativeSearchStatus.Ok)
        {
            throw new NativeSearchException((NativeSearchStatus)status, NativeSearchInterop.TakeLastError());
        }
    }

    private void ThrowIfDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(NativeSearchCancellationToken));
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        Handle.Dispose();
    }
}
