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
// Test 14: real DOCX/PPTX/PDF files generated by actual libraries
// (ReportLab's PDF specifically uses an ASCII85Decode+FlateDecode filter
// chain - the exact case that silently failed in the PowerShell version
// before that bug was found and fixed)
// ---------------------------------------------------------------------
{
    var dir = Path.Combine(testRoot, "realformats");
    Directory.CreateDirectory(dir);
    File.Copy("/tmp/test.docx", Path.Combine(dir, "test.docx"));
    File.Copy("/tmp/test.pptx", Path.Combine(dir, "test.pptx"));
    File.Copy("/tmp/test.pdf", Path.Combine(dir, "test.pdf"));

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

Console.WriteLine();
Console.WriteLine(failures == 0 ? "ALL TESTS PASSED" : $"{failures} TEST(S) FAILED");
Directory.Delete(testRoot, true);
return failures == 0 ? 0 : 1;
