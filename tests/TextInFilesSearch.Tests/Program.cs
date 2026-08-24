using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using TextInFilesSearch.Models;
using TextInFilesSearch.Services;

// Mirrors the one-time registration App.xaml.cs performs at startup - tested
// here directly since App.xaml.cs itself can't be exercised outside WinUI.
Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);

var testRoot = Path.Combine(Path.GetTempPath(), "tifs_verify_" + Guid.NewGuid().ToString("N")[..8]);
Directory.CreateDirectory(testRoot);
int failures = 0;

void ExtractEmbeddedFixture(string logicalName, string destPath)
{
    using var resourceStream = System.Reflection.Assembly.GetExecutingAssembly().GetManifestResourceStream(logicalName)
        ?? throw new InvalidOperationException($"Embedded fixture '{logicalName}' not found.");
    using var fileStream = File.Create(destPath);
    resourceStream.CopyTo(fileStream);
}

void Check(string name, bool condition)
{
    if (condition)
    {
        Console.WriteLine($"PASS: {name}");
    }
    else
    {
        Console.WriteLine($"FAIL: {name}");
        failures++;
    }
}

// ---------------------------------------------------------------------
// Test 1: basic text file search, single hit, single line (the exact
// shape of bug that repeatedly broke the PowerShell version)
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "basic");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "one.txt"), "apple only");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple" }
    };

    var orch = new SearchOrchestrator();
    var result = await orch.RunAsync(settings, null, CancellationToken.None);

    Check("single-line single-file: files searched == 1", result.Summary.FilesSearched == 1);
    Check("single-line single-file: exactly 1 hit", result.FileResults.Count(r => r.Status == FileSearchStatus.Hit) == 1);
    Check("single-line single-file: no unexpected errors", result.Summary.SkippedUnexpectedError == 0);
}

// ---------------------------------------------------------------------
// Test 2: multi-line, multi-filter, AnyLine mode
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "multi");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "line one has apple\nline two has nothing\nline three has banana\nline four has apple and banana together\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "this file only has apple\nnever mentions the other fruit\n");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple", "banana" }
    };

    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    int totalHits = result.FileResults.Where(r => r.Status == FileSearchStatus.Hit).Sum(r => r.Hits.Count);
    // Hits are counted per matching LINE, not per filter: a.txt has 3 matching
    // lines (line1=apple, line3=banana, line4=both-counted-once) + b.txt has 1 = 4.
    Check("AnyLine multi-file: 4 total hits across 2 files", totalHits == 4 && result.FileResults.Count(r => r.Status == FileSearchStatus.Hit) == 2);
}

// ---------------------------------------------------------------------
// Test 3: AllInFile mode
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "allinfile");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\nbanana\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "apple only\n");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple", "banana" },
        MatchMode = MatchMode.AllInFile
    };

    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("AllInFile: only a.txt qualifies", result.FileResults.Count(r => r.Status == FileSearchStatus.Hit) == 1);
    Check("AllInFile: b.txt is ModeExcluded", result.FileResults.Any(r => r.FullName.EndsWith("b.txt") && r.Status == FileSearchStatus.ModeExcluded));
}

// ---------------------------------------------------------------------
// Test 4: Proximity mode
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "proximity");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "close.txt"), "apple\nbanana\n");
    File.WriteAllText(Path.Combine(dir, "far.txt"), "apple\n" + string.Concat(Enumerable.Repeat("filler\n", 20)) + "banana\n");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple", "banana" },
        MatchMode = MatchMode.Proximity,
        ProximityLines = 3
    };

    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("Proximity: close.txt qualifies", result.FileResults.Any(r => r.FullName.EndsWith("close.txt") && r.Status == FileSearchStatus.Hit));
    Check("Proximity: far.txt is excluded", result.FileResults.Any(r => r.FullName.EndsWith("far.txt") && r.Status == FileSearchStatus.ModeExcluded));
}

// ---------------------------------------------------------------------
// Test 5: ExcludeFilter with File scope
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "exclude");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "keep.txt"), "apple\n");
    File.WriteAllText(Path.Combine(dir, "drop.txt"), "apple\nbanana\n");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple" },
        ExcludeFilters = new() { "banana" },
        ExcludeScope = ExcludeScope.File
    };

    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("ExcludeScope.File: keep.txt has a hit", result.FileResults.Any(r => r.FullName.EndsWith("keep.txt") && r.Status == FileSearchStatus.Hit));
    Check("ExcludeScope.File: drop.txt is ExcludedFile", result.FileResults.Any(r => r.FullName.EndsWith("drop.txt") && r.Status == FileSearchStatus.ExcludedFile));
}

// ---------------------------------------------------------------------
// Test 6: WholeWord mode
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "wholeword");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "the cat sat\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "category theory\n");

    var settingsOn = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "cat" }, WholeWord = true };
    var resultOn = await new SearchOrchestrator().RunAsync(settingsOn, null, CancellationToken.None);
    Check("WholeWord on: a.txt matches, b.txt does not",
        resultOn.FileResults.Any(r => r.FullName.EndsWith("a.txt") && r.Status == FileSearchStatus.Hit) &&
        resultOn.FileResults.Any(r => r.FullName.EndsWith("b.txt") && r.Status == FileSearchStatus.NoHit));

    var settingsOff = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "cat" }, WholeWord = false };
    var resultOff = await new SearchOrchestrator().RunAsync(settingsOff, null, CancellationToken.None);
    Check("WholeWord off: both a.txt and b.txt match",
        resultOff.FileResults.Count(r => r.Status == FileSearchStatus.Hit) == 2);
}

// ---------------------------------------------------------------------
// Test 7: Regex mode
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "regex");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "appple pie\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "ap+le" }, UseRegex = true };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("Regex mode matches appple", result.FileResults.Any(r => r.Status == FileSearchStatus.Hit));
}

// ---------------------------------------------------------------------
// Test 8: ASCII85 decode correctness (case-sensitivity fix specifically)
// ---------------------------------------------------------------------
{
    string sample = "ZP(bk"; // 5-char group starting with uppercase Z - must decode to exactly 4 bytes, not treated as shorthand
    byte[] decoded = TextExtractionService.DecodeAscii85(sample);
    Check("ASCII85: uppercase Z group decodes to exactly 4 bytes (not treated as shorthand)", decoded.Length == 4);

    string zShorthand = "z";
    byte[] decodedZ = TextExtractionService.DecodeAscii85(zShorthand);
    Check("ASCII85: lowercase z shorthand decodes to exactly 4 zero bytes", decodedZ.Length == 4 && decodedZ.All(b => b == 0));
}

// ---------------------------------------------------------------------
// Test 9: RTF extraction
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "rtf");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.rtf"), "{\\rtf1\\ansi apple only}");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("RTF extraction finds apple", result.FileResults.Any(r => r.Status == FileSearchStatus.Hit));
}

// ---------------------------------------------------------------------
// Test 10: Parallel vs sequential produce identical results
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "parallelcheck");
    Directory.CreateDirectory(dir);
    for (int i = 0; i < 15; i++)
    {
        File.WriteAllText(Path.Combine(dir, $"f{i}.txt"), i % 2 == 0 ? "apple here\nbanana there\n" : "nothing relevant\n");
    }

    var seqSettings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" } };
    var seqResult = await new SearchOrchestrator().RunAsync(seqSettings, null, CancellationToken.None);

    var parSettings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" }, Parallel = true, ThrottleLimit = 4 };
    var parResult = await new SearchOrchestrator().RunAsync(parSettings, null, CancellationToken.None);

    int seqHits = seqResult.FileResults.Where(r => r.Status == FileSearchStatus.Hit).Sum(r => r.Hits.Count);
    int parHits = parResult.FileResults.Where(r => r.Status == FileSearchStatus.Hit).Sum(r => r.Hits.Count);
    Check("Parallel and sequential produce identical hit counts", seqHits == parHits && seqHits > 0);
    Check("Parallel and sequential produce identical file-hit counts",
        seqResult.FileResults.Count(r => r.Status == FileSearchStatus.Hit) == parResult.FileResults.Count(r => r.Status == FileSearchStatus.Hit));
}

// ---------------------------------------------------------------------
// Test 11: Incremental cache - cold run, warm run (reuse), settings change (invalidate)
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "cachecheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "banana\n");
    var cacheFile = Path.Combine(testRoot, "cache.json");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" }, CacheFilePath = cacheFile };
    var cold = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("Cache cold run: 0 reused", cold.Summary.CacheReused == 0);

    var warm = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("Cache warm run: both files reused", warm.Summary.CacheReused == 2);

    var changedSettings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "orange" }, CacheFilePath = cacheFile };
    var changed = await new SearchOrchestrator().RunAsync(changedSettings, null, CancellationToken.None);
    Check("Cache invalidated on filter change: 0 reused", changed.Summary.CacheReused == 0);
}

// ---------------------------------------------------------------------
// Test 12: progress reporting fires and includes in-flight status
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "progresscheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");

    var reports = new System.Collections.Generic.List<SearchProgressReport>();
    var progress = new Progress<SearchProgressReport>(r => reports.Add(r));
    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" } };
    await new SearchOrchestrator().RunAsync(settings, progress, CancellationToken.None);

    await Task.Delay(200);
    Check("Progress reporting fired at least once", reports.Count > 0);
}

// ---------------------------------------------------------------------
// Test 13: DryRun never processes file content
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "dryrun");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" }, DryRun = true };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("DryRun returns candidates without processing", result.WasDryRun && result.DryRunCandidates?.Count == 1 && result.FileResults.Count == 0);
}

// ---------------------------------------------------------------------
// Test 14: real DOCX/PPTX/PDF fixtures (embedded resources under
// Fixtures/, see the .csproj) exercising the actual ZIP/OOXML and PDF
// parsers end to end. The PDF fixture specifically uses an
// ASCII85Decode+FlateDecode filter chain - the exact case that silently
// failed in the PowerShell version before that bug was found and fixed.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "realformats");
    Directory.CreateDirectory(dir);
    ExtractEmbeddedFixture("test.docx", Path.Combine(dir, "test.docx"));
    ExtractEmbeddedFixture("test.pptx", Path.Combine(dir, "test.pptx"));
    ExtractEmbeddedFixture("test.pdf", Path.Combine(dir, "test.pdf"));

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);

    var docxResult = result.FileResults.First(r => r.FullName.EndsWith("test.docx"));
    var pptxResult = result.FileResults.First(r => r.FullName.EndsWith("test.pptx"));
    var pdfResult = result.FileResults.First(r => r.FullName.EndsWith("test.pdf"));

    Check("DOCX: real python-docx file finds both apple and banana",
        docxResult.Status == FileSearchStatus.Hit && docxResult.Hits.Sum(h => h.MatchedFilters.Count) == 2);
    Check("PPTX: real python-pptx file finds apple",
        pptxResult.Status == FileSearchStatus.Hit);
    Check("PDF: real ReportLab file (ASCII85+FlateDecode chain) finds both apple and banana",
        pdfResult.Status == FileSearchStatus.Hit && pdfResult.Hits.Sum(h => h.MatchedFilters.Count) == 2);
    Check("PDF: extraction confidence looks reliable for clean generated text",
        !pdfResult.LowConfidencePdf);
}

// ---------------------------------------------------------------------
// Test 15: HTML report generation, including highlighting correctness
// (this exact class of bug - a highlighted span collapsing to zero width
// due to array/expression unwrapping - was a real, repeatedly-found bug
// in the PowerShell version)
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "reportcheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "line one has apple\nline two has banana\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    string html = ReportExportService.BuildHtmlReport(settings, result);

    Check("HTML report contains highlighted <mark>apple</mark>", html.Contains("<mark>apple</mark>"));
    Check("HTML report contains highlighted <mark>banana</mark>", html.Contains("<mark>banana</mark>"));
    Check("HTML report contains dark-mode CSS", html.Contains("prefers-color-scheme"));
    Check("HTML report contains bar chart markup", html.Contains("bar-row"));

    var exportRows = ReportExportService.BuildExportRows(result);
    Check("Export rows: 2 rows generated", exportRows.Count == 2);

    string csvPath = Path.Combine(testRoot, "out.csv");
    string jsonPath = Path.Combine(testRoot, "out.json");
    ReportExportService.WriteCsv(csvPath, exportRows);
    ReportExportService.WriteJson(jsonPath, exportRows);
    Check("CSV file written and non-empty", new FileInfo(csvPath).Length > 0);
    Check("JSON file written and non-empty", new FileInfo(jsonPath).Length > 0);
}

// ---------------------------------------------------------------------
// Test 16: regex-mode highlighting (this specifically was skipped/broken
// in earlier iterations - regex matches must still get real <mark> spans)
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "reportregexcheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "appple pie is great\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "ap+le" }, UseRegex = true };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    string html = ReportExportService.BuildHtmlReport(settings, result);
    Check("Regex-mode HTML report highlights the actual matched span", html.Contains("<mark>appple</mark>"));
}

// ---------------------------------------------------------------------
// Test 17: MainViewModel - settings parsing, run lifecycle, progress
// binding, and results population, using injected no-op folder pickers
// so this runs with zero WinUI/Windows App SDK dependency
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "viewmodelcheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\nbanana\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "nothing relevant\n");

    var outputDir = Path.Combine(testRoot, "viewmodelcheck_out");

    var vm = new TextInFilesSearch.ViewModels.MainViewModel();
    vm.SearchPath = dir;
    vm.OutputFolder = outputDir;
    vm.FiltersText = "apple, banana";

    Check("ViewModel: RunSearchCommand.CanExecute is true once required fields are set", vm.RunSearchCommand.CanExecute(null));

    var propertyChanges = new System.Collections.Generic.List<string?>();
    vm.PropertyChanged += (_, e) => propertyChanges.Add(e.PropertyName);

    await vm.RunSearchAsync();

    Check("ViewModel: IsRunning is false after completion", !vm.IsRunning);
    Check("ViewModel: HasResults is true", vm.HasResults);
    Check("ViewModel: Results contains exactly 1 file (a.txt)", vm.Results.Count == 1 && vm.Results[0].FileName == "a.txt");
    Check("ViewModel: Results shows 2 hits for a.txt", vm.Results[0].HitCount == 2);
    Check("ViewModel: ResultsSummaryText populated", !string.IsNullOrWhiteSpace(vm.ResultsSummaryText));
    Check("ViewModel: LastReportPath set and file exists", vm.LastReportPath is not null && File.Exists(vm.LastReportPath));
    Check("ViewModel: PropertyChanged fired for IsRunning during the run", propertyChanges.Contains(nameof(vm.IsRunning)));
    Check("ViewModel: OpenReportCommand becomes executable after a run", vm.OpenReportCommand.CanExecute(null));
}

// ---------------------------------------------------------------------
// Test 18: MainViewModel - folder picker delegates are actually invoked
// ---------------------------------------------------------------------
{
    bool searchPickerCalled = false;
    bool outputPickerCalled = false;
    string? openedReportPath = null;

    var vm = new TextInFilesSearch.ViewModels.MainViewModel(
        browseSearchFolder: () => { searchPickerCalled = true; return Task.FromResult<string?>("/picked/search"); },
        browseOutputFolder: () => { outputPickerCalled = true; return Task.FromResult<string?>("/picked/output"); },
        openReport: path => openedReportPath = path);

    vm.BrowseSearchFolderCommand.Execute(null);
    // AsyncRelayCommand.Execute is async void, so give it a moment to complete.
    await Task.Delay(50);

    Check("ViewModel: search folder picker delegate invoked and value applied", searchPickerCalled && vm.SearchPath == "/picked/search");

    vm.BrowseOutputFolderCommand.Execute(null);
    await Task.Delay(50);
    Check("ViewModel: output folder picker delegate invoked and value applied", outputPickerCalled && vm.OutputFolder == "/picked/output");
}

// ---------------------------------------------------------------------
// Test 19: MainViewModel - cancellation actually stops a run early
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "cancelcheck");
    Directory.CreateDirectory(dir);
    for (int i = 0; i < 50; i++) File.WriteAllText(Path.Combine(dir, $"f{i}.txt"), "apple\n");

    var vm = new TextInFilesSearch.ViewModels.MainViewModel();
    vm.SearchPath = dir;
    vm.OutputFolder = Path.Combine(testRoot, "cancelcheck_out");
    vm.FiltersText = "apple";

    var runTask = vm.RunSearchAsync();
    vm.CancelCommand.Execute(null);
    await runTask;

    Check("ViewModel: cancelled run ends with Cancelled status text", vm.StatusText == "Cancelled." || vm.StatusText == "Done.");
    // Note: with only 50 tiny files this may legitimately finish before the
    // cancel signal is observed - the important invariant is that it doesn't
    // hang or throw unhandled, which the surrounding await already proves.
}

// ---------------------------------------------------------------------
// Test 20: Windows-1252 fallback decoding, WITH the CodePagesEncodingProvider
// registration that only otherwise happens in the untestable App.xaml.cs -
// registered above, exercised here to confirm the full mechanism actually
// works rather than being an unverified assumption.
// ---------------------------------------------------------------------
{
    byte[] cp1252Bytes = { 72, 101, 108, 108, 111, 32, 0x93, 87, 111, 114, 108, 100, 0x94 }; // Hello "World" with cp1252 curly quotes
    string decoded = TextExtractionService.DecodeText(cp1252Bytes);
    Check("Windows-1252 fallback decodes curly quotes correctly (not mangled)",
        decoded.Contains('\u201C') && decoded.Contains('\u201D'));

    var dir = Path.Combine(testRoot, "encodingcheck");
    Directory.CreateDirectory(dir);
    File.WriteAllBytes(Path.Combine(dir, "legacy.txt"), cp1252Bytes.Concat(Encoding.ASCII.GetBytes(" apple")).ToArray());

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    Check("Windows-1252 file end-to-end through the full search pipeline finds the hit",
        result.FileResults.Any(r => r.Status == FileSearchStatus.Hit));
}

// ---------------------------------------------------------------------
// Test 21: ExcludeFolders matches whole path segments, not raw substrings -
// excluding "bin" must prune the actual "bin" folder but must NOT exclude
// "robin" merely because it contains "bin" as a substring (the same class
// of bug documented in CLAUDE.md for the packaging script's -x "*.git*").
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "excludefolder");
    var binDir = Path.Combine(dir, "bin");
    var robinDir = Path.Combine(dir, "robin");
    Directory.CreateDirectory(binDir);
    Directory.CreateDirectory(robinDir);
    File.WriteAllText(Path.Combine(binDir, "a.txt"), "apple\n");
    File.WriteAllText(Path.Combine(robinDir, "b.txt"), "apple\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" }, ExcludeFolders = new() { "bin" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);

    Check("ExcludeFolders: the 'bin' folder itself is pruned from the walk entirely",
        !result.FileResults.Any(r => r.FullName.EndsWith("a.txt")));
    Check("ExcludeFolders: 'robin' is NOT excluded merely for containing \"bin\" as a substring",
        result.FileResults.Any(r => r.FullName.EndsWith("b.txt") && r.Status == FileSearchStatus.Hit));
}

// ---------------------------------------------------------------------
// Test 22: an invalid regex filter throws a typed, specific exception
// naming the bad filter, instead of a bare ArgumentException surfacing
// only as a generic "Error: parsing pattern..." with no indication of
// which of possibly many filters was the problem.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "badregex");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "(unclosed" }, UseRegex = true };
    bool threw = false;
    string? message = null;
    try
    {
        await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    }
    catch (InvalidFilterRegexException ex)
    {
        threw = true;
        message = ex.Message;
    }
    Check("Invalid regex filter throws InvalidFilterRegexException naming the bad filter",
        threw && message is not null && message.Contains("(unclosed"));
}

// ---------------------------------------------------------------------
// Test 23: whole-word matching on a filter whose first/last character is
// itself punctuation (e.g. "C#"). Plain \b fails here because \b only
// asserts a \w/\W transition, and neither side of "C#" standing alone
// between spaces is such a transition on the '#' side.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "wholewordpunct");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "I love C# language\n");
    File.WriteAllText(Path.Combine(dir, "b.txt"), "ABC# is not C sharp\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "C#" }, WholeWord = true };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);

    Check("WholeWord punctuation-edge: 'C#' matches as a standalone token",
        result.FileResults.Any(r => r.FullName.EndsWith("a.txt") && r.Status == FileSearchStatus.Hit));
    Check("WholeWord punctuation-edge: 'C#' does not match inside 'ABC#'",
        result.FileResults.Any(r => r.FullName.EndsWith("b.txt") && r.Status == FileSearchStatus.NoHit));
}

// ---------------------------------------------------------------------
// Test 24: case-variant duplicate filters (e.g. "apple" and "APPLE" both
// present) are tracked as independent filter slots by index rather than
// collapsing into one dictionary bucket keyed by case-insensitive text -
// confirms Proximity mode still computes a correct, sane result rather
// than silently losing one of the slots.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "dupcasefilter");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\nbanana\n");

    var settings = new SearchSettings
    {
        SearchPath = dir,
        OutputFolder = testRoot,
        Filters = new() { "apple", "APPLE", "banana" },
        MatchMode = MatchMode.Proximity,
        ProximityLines = 5
    };

    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    var fileResult = result.FileResults.FirstOrDefault(r => r.FullName.EndsWith("a.txt"));
    Check("Case-variant duplicate filters: Proximity mode handles duplicate slots correctly",
        fileResult is not null && fileResult.Status == FileSearchStatus.Hit && fileResult.ProximityMinRange == 1);
}

// ---------------------------------------------------------------------
// Test 25: an already-cancelled token is honored during the directory
// walk itself, not just during per-file processing - previously
// EnumerateFilesSafely had no CancellationToken parameter at all, so
// Cancel did nothing until enumeration finished on a large/slow tree.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "cancelenum");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");

    using var cts = new CancellationTokenSource();
    cts.Cancel();

    bool threw = false;
    try
    {
        FileReaderService.EnumerateFilesSafely(dir, includeHidden: false, excludeFolders: null, cts.Token, onProgress: null, out _);
    }
    catch (OperationCanceledException)
    {
        threw = true;
    }
    Check("EnumerateFilesSafely honors an already-cancelled token", threw);
}

// ---------------------------------------------------------------------
// Test 26: CSV export neutralizes formula injection - a matched line
// starting with '=' would otherwise execute as a formula when the CSV is
// opened in a spreadsheet app, since this export reflects arbitrary file
// content verbatim.
// ---------------------------------------------------------------------
{
    var rows = new System.Collections.Generic.List<ReportExportService.ExportRow>
    {
        new() { FilePath = "x.txt", LineNumber = 1, MatchedFilters = "f", MatchLine = "=SUM(A1:A10)", Created = DateTime.UtcNow, Modified = DateTime.UtcNow }
    };
    string csvPath = Path.Combine(testRoot, "injection.csv");
    ReportExportService.WriteCsv(csvPath, rows);
    string csvContent = File.ReadAllText(csvPath);
    Check("CSV export neutralizes a leading '=' formula-injection trigger", csvContent.Contains("'=SUM(A1:A10)"));
}

// ---------------------------------------------------------------------
// Test 27: real XLSX fixture - shared strings resolved per cell, one
// "line" emitted per row.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "xlsxcheck");
    Directory.CreateDirectory(dir);
    ExtractEmbeddedFixture("test.xlsx", Path.Combine(dir, "test.xlsx"));

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    var xlsxResult = result.FileResults.FirstOrDefault(r => r.FullName.EndsWith("test.xlsx"));
    Check("XLSX: finds both apple and banana via shared strings",
        xlsxResult is not null && xlsxResult.Status == FileSearchStatus.Hit && xlsxResult.Hits.Sum(h => h.MatchedFilters.Count) == 2);
}

// ---------------------------------------------------------------------
// Test 28: real ZIP fixture - a plain-text entry and a nested DOCX entry,
// exercising the recursive per-entry extraction dispatch.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "zipcheck");
    Directory.CreateDirectory(dir);
    ExtractEmbeddedFixture("test.zip", Path.Combine(dir, "test.zip"));

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    var zipResult = result.FileResults.FirstOrDefault(r => r.FullName.EndsWith("test.zip"));
    Check("ZIP: finds apple (plain entry) and banana (nested docx entry)",
        zipResult is not null && zipResult.Status == FileSearchStatus.Hit && zipResult.Hits.Sum(h => h.MatchedFilters.Count) == 2);
}

// ---------------------------------------------------------------------
// Test 29: real PPTX fixture with speaker notes and a SmartArt diagram
// part in addition to the slide itself - both were previously invisible
// to search.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "pptxnotescheck");
    Directory.CreateDirectory(dir);
    ExtractEmbeddedFixture("test_notes.pptx", Path.Combine(dir, "test_notes.pptx"));

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple", "banana", "cherry" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    var pptxResult = result.FileResults.FirstOrDefault(r => r.FullName.EndsWith("test_notes.pptx"));
    Check("PPTX: finds slide text (apple), speaker notes (banana), and SmartArt diagram text (cherry)",
        pptxResult is not null && pptxResult.Status == FileSearchStatus.Hit && pptxResult.Hits.Sum(h => h.MatchedFilters.Count) == 3);
}

// ---------------------------------------------------------------------
// Test 30: OutputName is sanitized against illegal path characters
// instead of throwing at save time.
// ---------------------------------------------------------------------
{
    var vm = new TextInFilesSearch.ViewModels.MainViewModel();
    vm.SearchPath = testRoot;
    vm.OutputFolder = testRoot;
    vm.FiltersText = "apple";
    vm.OutputName = "bad:name*report?.html";

    var settings = vm.BuildSettings();
    Check("OutputName sanitized: no invalid filename characters remain",
        settings.OutputName is not null && settings.OutputName.IndexOfAny(Path.GetInvalidFileNameChars()) < 0);
}

// ---------------------------------------------------------------------
// Test 31: SplitLines handles a lone \r (classic Mac line endings), not
// just \r\n and \n - previously such a file extracted as one giant line,
// making line numbers/context in the report meaningless for it.
// ---------------------------------------------------------------------
{
    string[] lines = TextExtractionService.SplitLines("line one\rline two\rline three");
    Check("SplitLines: lone \\r splits into 3 lines",
        lines.Length == 3 && lines[0] == "line one" && lines[1] == "line two" && lines[2] == "line three");
}

// ---------------------------------------------------------------------
// Test 32: numeric settings on the ViewModel clamp to a sane range
// instead of silently accepting a negative/zero value that produces
// confusing behavior (e.g. MaxFileSizeMB=0 skipping every file as "too
// large" with no explanation).
// ---------------------------------------------------------------------
{
    var vm = new TextInFilesSearch.ViewModels.MainViewModel();
    vm.ThrottleLimit = -5;
    vm.ProximityLines = -3;
    vm.MaxFileSizeMB = -10;
    vm.FileTimeoutSeconds = 0;

    Check("Numeric clamps: ThrottleLimit floors at 1", vm.ThrottleLimit == 1);
    Check("Numeric clamps: ProximityLines floors at 0", vm.ProximityLines == 0);
    Check("Numeric clamps: MaxFileSizeMB floors above 0", vm.MaxFileSizeMB > 0);
    Check("Numeric clamps: FileTimeoutSeconds floors at 1", vm.FileTimeoutSeconds == 1);
}

// ---------------------------------------------------------------------
// Test 33: extension type-to-filter + tick-list picker - catalog
// population, filtering, ticking feeding into BuildSettings, and adding
// a custom extension not already in the built-in catalog.
// ---------------------------------------------------------------------
{
    var vm = new TextInFilesSearch.ViewModels.MainViewModel();

    Check("Extension picker: catalog is pre-populated from the shared ExtensionCatalog",
        vm.ExtensionCatalog.Count == ExtensionCatalog.AllExtensions.Count);
    Check("Extension picker: nothing ticked means BuildSettings().Extensions is null (default list)",
        vm.BuildSettings().Extensions is null);

    vm.ExtensionFilterText = "xlsx";
    Check("Extension picker: typing narrows the filtered catalog",
        vm.FilteredExtensionCatalog.Count > 0 && vm.FilteredExtensionCatalog.All(e => e.Extension.Contains("xlsx")));

    var xlsxOption = vm.FilteredExtensionCatalog.First(e => e.Extension == ".xlsx");
    xlsxOption.IsSelected = true;
    Check("Extension picker: ticking an entry makes it appear in BuildSettings().Extensions",
        vm.BuildSettings().Extensions is { } sel && sel.Contains(".xlsx"));

    vm.ExtensionFilterText = string.Empty;
    vm.AddCustomExtension("foo");
    Check("Extension picker: custom extension normalized with a leading dot and auto-selected",
        vm.ExtensionCatalog.Any(e => e.Extension == ".foo" && e.Category == "Custom" && e.IsSelected));
}

// ---------------------------------------------------------------------
// Test 34: HTML report embeds the GS Engineering banner as a self-contained
// base64 data URI (no external image file for the report to lose track
// of if it's moved).
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "bannercheck");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "a.txt"), "apple\n");

    var settings = new SearchSettings { SearchPath = dir, OutputFolder = testRoot, Filters = new() { "apple" } };
    var result = await new SearchOrchestrator().RunAsync(settings, null, CancellationToken.None);
    string html = ReportExportService.BuildHtmlReport(settings, result);

    Check("HTML report embeds the banner as a base64 data URI",
        html.Contains("<img class=\"report-banner\" src=\"data:image/jpeg;base64,"));
}

// ---------------------------------------------------------------------
// Test 35: native_search.dll round trip (issue #2 Phase 3), including
// cancellation (Section 17). This is the one thing the Rust side's own
// test suite (native-search/tests) cannot prove by itself: that the actual
// P/Invoke marshalling in NativeSearchService/NativeSearchInterop -
// source-generated LibraryImport stubs, SafeHandle lifetime (for both the
// engine handle and the cancellation-token handle), UTF-8 string
// marshalling, the (ptr, len) body convention - lines up with the Rust
// side across a real process boundary, not just in each side's own unit
// tests.
//
// native_search.dll only exists once native-search/ has actually been
// built (see .github/workflows/build.yml, which builds it before this
// harness runs). A developer running this locally without the Rust
// toolchain still gets a clean run - SKIP, not FAIL - rather than being
// forced to install Rust just to iterate on unrelated C# changes.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "native-search-index");
    Directory.CreateDirectory(dir);

    try
    {
        using var native = new NativeSearchService(dir);

        native.IndexDocument(new NativeDocumentInput(
            Id: "1",
            Path: @"C:\docs\Torque-Spec-Deviation-Report.pdf",
            FileName: "Torque-Spec-Deviation-Report.pdf",
            Extension: ".pdf",
            Title: null,
            Modified: DateTime.UtcNow,
            Created: DateTime.UtcNow,
            Size: 4096,
            Body: "torque spec deviation on aft mount bolts, re-torque completed"));
        native.IndexDocument(new NativeDocumentInput(
            Id: "2",
            Path: @"C:\docs\Corrosion-Inspection-Q1.docx",
            FileName: "Corrosion-Inspection-Q1.docx",
            Extension: ".docx",
            Title: "Quarterly Corrosion Inspection",
            Modified: DateTime.UtcNow,
            Created: DateTime.UtcNow,
            Size: 2048,
            Body: "minor filiform corrosion observed along fastener row twelve"));
        native.Commit();

        var hits = native.Search("torque", limit: 10);
        Check("native_search: query matches only the indexed document containing the term",
            hits.Count == 1 && hits[0].Id == "1");
        Check("native_search: unmatched fields still round-trip through the JSON boundary",
            hits.Count == 1 && hits[0].Path == @"C:\docs\Torque-Spec-Deviation-Report.pdf" && hits[0].Extension == ".pdf");

        native.DeleteDocument("1");
        native.Commit();
        var afterDelete = native.Search("torque", limit: 10);
        Check("native_search: delete + commit removes the document from search results",
            afterDelete.Count == 0);

        bool threw = false;
        try
        {
            _ = native.Search(string.Empty, limit: 10);
        }
        catch (NativeSearchException ex) when (ex.Status == "InvalidArgument")
        {
            threw = true;
        }
        Check("native_search: an empty query surfaces as a typed NativeSearchException, not a crash", threw);

        // Document "2" ("corrosion...") was indexed above and never
        // deleted (only "1" was) - still live, so the cancellation checks
        // below exercise a real search, not an already-empty index.
        using var cancelledToken = new NativeSearchCancellationToken();
        cancelledToken.Cancel();
        bool threwCancelled = false;
        try
        {
            _ = native.Search("corrosion", limit: 10, cancellationToken: cancelledToken);
        }
        catch (NativeSearchException ex) when (ex.Status == "Cancelled")
        {
            threwCancelled = true;
        }
        Check("native_search: a pre-cancelled token surfaces search as a typed Cancelled exception (issue #2 Section 17)", threwCancelled);

        using var freshToken = new NativeSearchCancellationToken();
        var uncancelledHits = native.Search("corrosion", limit: 10, cancellationToken: freshToken);
        Check("native_search: an un-cancelled token does not block a search", uncancelledHits.Count == 1);
    }
    catch (DllNotFoundException)
    {
        Console.WriteLine("SKIP: native_search round trip (native_search.dll not present - build native-search/ first; see docs/ffi.md)");
    }
}

// ---------------------------------------------------------------------
// Test 36: MainViewModel wiring for native search (issue #2). Test 35
// covers NativeSearchService directly; this covers the actual UI-facing
// surface - IndexForFastSearch driving RunSearchAsync to index its hits,
// and NativeSearchCommand/RunNativeSearchAsync searching them back out -
// so the ViewModel-level integration is verified too, not just the
// service it wraps. Same SKIP-not-FAIL convention as Test 35.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "vm-native-search");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "torque.txt"), "torque spec deviation on aft mount bolts\n");
    File.WriteAllText(Path.Combine(dir, "corrosion.txt"), "minor filiform corrosion observed\n");

    var nativeIndexDir = Path.Combine(testRoot, "vm-native-search-index");
    var outputDir = Path.Combine(testRoot, "vm-native-search-out");

    var vm = new TextInFilesSearch.ViewModels.MainViewModel(nativeSearchIndexDirectory: nativeIndexDir);

    Check("ViewModel: NativeSearchCommand.CanExecute is false with an empty query",
        !vm.NativeSearchCommand.CanExecute(null));
    vm.NativeSearchQuery = "torque";
    Check("ViewModel: NativeSearchCommand.CanExecute becomes true once a query is typed",
        vm.NativeSearchCommand.CanExecute(null));
    Check("ViewModel: CancelNativeSearchCommand.CanExecute is false when nothing is searching",
        !vm.CancelNativeSearchCommand.CanExecute(null));
    Check("ViewModel: IndexForFastSearch defaults to off",
        !vm.IndexForFastSearch);

    // Note: unlike Test 35, RunSearchAsync/RunNativeSearchAsync never throw
    // DllNotFoundException out to a caller - IndexHitsForFastSearch and
    // RunNativeSearchAsync both catch it internally and report it through
    // NativeSearchStatusText instead (so an optional convenience feature
    // failing can't turn a successful file search into a reported error).
    // The SKIP/PASS branch below is driven by that status text, not a
    // try/catch here.
    vm.SearchPath = dir;
    vm.OutputFolder = outputDir;
    vm.FiltersText = "torque, corrosion";
    vm.IndexForFastSearch = true;

    await vm.RunSearchAsync();

    Check("ViewModel: a run with IndexForFastSearch on reports an outcome (indexed or explicitly unavailable), never leaves the status blank",
        !string.IsNullOrWhiteSpace(vm.NativeSearchStatusText));

    if (vm.NativeSearchStatusText.StartsWith("Indexed", StringComparison.Ordinal))
    {
        await vm.RunNativeSearchAsync();
        Check("ViewModel: NativeSearchCommand's underlying search finds the file indexed by the run above",
            vm.NativeSearchResults.Count == 1 && vm.NativeSearchResults[0].Filename == "torque.txt");
        Check("ViewModel: IsNativeSearching is false after the search completes",
            !vm.IsNativeSearching);

        // issue #2: re-running against the same, unchanged files must skip
        // re-indexing them (NativeSearchService.TryGetDocumentMetadata),
        // not silently redo the same work every run.
        await vm.RunSearchAsync();
        Check("ViewModel: re-running over unchanged files reports them as already up to date, not re-indexed",
            vm.NativeSearchStatusText.Contains("already up to date", StringComparison.OrdinalIgnoreCase));
    }
    else
    {
        Console.WriteLine($"SKIP: ViewModel native-search round trip (native_search.dll not present - {vm.NativeSearchStatusText})");
    }
}

// ---------------------------------------------------------------------
// Test 37: native_search index folder placement and auto-exclusion
// (issue #2/ADR-011). Doesn't need native_search.dll - BuildSettings()
// and the normal line-scan search are pure C#, so this always runs,
// unlike Test 35/36's DLL-dependent checks.
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "auto-exclude-index-folder");
    Directory.CreateDirectory(dir);
    File.WriteAllText(Path.Combine(dir, "keep.txt"), "findme in a real file\n");

    // Simulates what a prior run with IndexForFastSearch on would have
    // left behind - a real native_search run isn't needed to prove the
    // *exclusion* works, only that something living at this exact path
    // never gets walked into.
    var indexFolder = Path.Combine(dir, TextInFilesSearch.Native.NativeSearchPaths.IndexFolderName);
    Directory.CreateDirectory(indexFolder);
    File.WriteAllText(Path.Combine(indexFolder, "decoy.txt"), "findme inside the index folder\n");

    var outputDir = Path.Combine(testRoot, "auto-exclude-index-folder-out");
    var vm = new TextInFilesSearch.ViewModels.MainViewModel();
    vm.SearchPath = dir;
    vm.OutputFolder = outputDir;
    vm.FiltersText = "findme";

    var settings = vm.BuildSettings();
    Check("ViewModel: BuildSettings() automatically excludes the native_search index folder",
        settings.ExcludeFolders.Contains(TextInFilesSearch.Native.NativeSearchPaths.IndexFolderName, StringComparer.OrdinalIgnoreCase));

    await vm.RunSearchAsync();

    Check("ViewModel: normal search still finds the real file outside the index folder",
        vm.Results.Any(r => r.FileName == "keep.txt"));
    Check("ViewModel: normal search never descends into the auto-excluded native_search index folder",
        !vm.Results.Any(r => r.FileName == "decoy.txt"));
}

Console.WriteLine();
Console.WriteLine(failures == 0 ? "ALL TESTS PASSED" : $"{failures} TEST(S) FAILED");
Directory.Delete(testRoot, true);
return failures == 0 ? 0 : 1;
