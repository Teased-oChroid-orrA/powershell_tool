using System;
using System.Runtime.InteropServices;

namespace TextInFilesSearch.Native;

/// <summary>
/// Wraps the opaque handle returned by native_search's <c>ns_create</c>.
/// A <see cref="SafeHandle"/> rather than a raw <see cref="IntPtr"/> so the
/// handle is reliably released even on an unhandled exception or abnormal
/// shutdown, and so every native_search call that takes it goes through
/// SafeHandle's ref-counted Dangerous*Ref pairing instead of racing a
/// concurrent Dispose - the standard .NET pattern for a native resource
/// handle, not something native_search.dll needs to implement itself.
/// </summary>
internal sealed class NativeSearchHandle : SafeHandle
{
    public NativeSearchHandle() : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        NativeSearchInterop.ns_destroy(handle);
        return true;
    }
}
