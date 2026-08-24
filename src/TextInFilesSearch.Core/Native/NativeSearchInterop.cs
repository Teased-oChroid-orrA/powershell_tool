using System;
using System.Runtime.InteropServices;

namespace TextInFilesSearch.Native;

/// <summary>
/// Raw P/Invoke surface for native_search.dll (native-search/src/ffi.rs).
/// Every signature here must match the Rust <c>extern "C"</c> side exactly -
/// see docs/ffi.md for the authoritative contract. Nothing outside the
/// <c>Native</c> namespace should call these directly; use
/// <see cref="TextInFilesSearch.Services.NativeSearchService"/> instead.
/// </summary>
internal static partial class NativeSearchInterop
{
    private const string LibraryName = "native_search";

    [LibraryImport(LibraryName, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ns_create(string indexDir, out NativeSearchHandle outHandle);

    // Raw-IntPtr overload used only by NativeSearchHandle.ReleaseHandle,
    // which cannot pass itself as a SafeHandle argument mid-release.
    [LibraryImport(LibraryName)]
    internal static partial void ns_destroy(IntPtr handle);

    [LibraryImport(LibraryName, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ns_index_document(
        NativeSearchHandle handle,
        string id,
        string path,
        string filename,
        string extension,
        string? title,
        long modifiedUnix,
        long createdUnix,
        long size,
        ReadOnlySpan<byte> body,
        nuint bodyLen);

    [LibraryImport(LibraryName, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ns_delete_document(NativeSearchHandle handle, string id);

    [LibraryImport(LibraryName)]
    internal static partial int ns_commit(NativeSearchHandle handle);

    [LibraryImport(LibraryName, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ns_search(
        NativeSearchHandle handle,
        string query,
        uint limit,
        out IntPtr outBuffer,
        out nuint outLen);

    [LibraryImport(LibraryName)]
    internal static partial int ns_last_error(out IntPtr outBuffer, out nuint outLen);

    [LibraryImport(LibraryName)]
    internal static partial void ns_free_buffer(IntPtr ptr, nuint len);

    /// <summary>
    /// Copies a native_search-owned buffer into a managed byte array and
    /// releases the native allocation - every buffer this DLL hands back
    /// (search results, error messages) must go through this exactly once.
    /// </summary>
    internal static byte[] CopyAndFreeBuffer(IntPtr ptr, nuint len)
    {
        if (ptr == IntPtr.Zero || len == 0)
        {
            return Array.Empty<byte>();
        }

        var bytes = new byte[len];
        Marshal.Copy(ptr, bytes, 0, checked((int)len));
        ns_free_buffer(ptr, len);
        return bytes;
    }

    /// <summary>
    /// Reads and clears the thread-local last-error message set by the most
    /// recent failing native_search call on this thread. Returns an empty
    /// string if none was set.
    /// </summary>
    internal static string TakeLastError()
    {
        int status = ns_last_error(out IntPtr buffer, out nuint len);
        if (status != (int)NativeSearchStatus.Ok)
        {
            return string.Empty;
        }

        byte[] bytes = CopyAndFreeBuffer(buffer, len);
        return bytes.Length == 0 ? string.Empty : System.Text.Encoding.UTF8.GetString(bytes);
    }
}
