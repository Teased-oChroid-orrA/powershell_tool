using System;
using System.Runtime.InteropServices;

namespace TextInFilesSearch.Native;

/// <summary>
/// Wraps the opaque handle returned by native_search's
/// <c>ns_cancel_token_create</c> (issue #2 Section 17). Independent of
/// <see cref="NativeSearchHandle"/> - a cancellation token can be created
/// and cancelled from a different thread than the one blocked inside a
/// <c>ns_search</c> call. Same SafeHandle rationale as
/// <see cref="NativeSearchHandle"/>.
/// </summary>
internal sealed class NativeSearchCancellationHandle : SafeHandle
{
    public NativeSearchCancellationHandle() : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        NativeSearchInterop.ns_cancel_token_destroy(handle);
        return true;
    }
}
