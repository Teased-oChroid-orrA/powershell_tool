namespace TextInFilesSearch.Models;

/// <summary>
/// Live status of one file currently being processed. In parallel mode there
/// can be several of these at once (up to the throttle limit); the UI shows
/// them all so a slow PDF is visibly "42 streams scanned, 8.2s elapsed" rather
/// than the whole run looking frozen.
/// </summary>
public sealed class InFlightFileStatus
{
    public string FileName { get; set; } = string.Empty;
    public string StatusText { get; set; } = "Starting...";
    public double ElapsedSeconds { get; set; }
}
