namespace TextInFilesSearch.Native;

/// <summary>Mirrors native-search/src/error.rs's <c>NsStatus</c> exactly - keep these in sync.</summary>
internal enum NativeSearchStatus
{
    Ok = 0,
    InvalidArgument = 1,
    FileNotFound = 2,
    AccessDenied = 3,
    UnsupportedFormat = 4,
    ExtractionFailed = 5,
    IndexError = 6,
    QueryError = 7,
    OutOfMemory = 8,
    Cancelled = 9,
    CorruptIndex = 10,
    InternalError = 11,
}
