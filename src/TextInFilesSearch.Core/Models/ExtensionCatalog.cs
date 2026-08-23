using System;
using System.Collections.Generic;
using System.Linq;

namespace TextInFilesSearch.Models;

/// <summary>One named group of related extensions, for the extension type-to-filter/tick-list UI.</summary>
public sealed class ExtensionCategoryDefinition
{
    public required string Category { get; init; }
    public required IReadOnlyList<string> Extensions { get; init; }
}

/// <summary>
/// Single source of truth for every extension the app knows how to search,
/// grouped by category for the picker UI. SearchSettings.DefaultExtensions
/// is the flattened form of this list, so the "built-in default" the engine
/// searches and the catalog the picker shows the user can never drift apart.
/// </summary>
public static class ExtensionCatalog
{
    public static readonly IReadOnlyList<ExtensionCategoryDefinition> Categories = new List<ExtensionCategoryDefinition>
    {
        new() { Category = "Documents", Extensions = new[] { ".docx", ".pdf", ".rtf", ".txt", ".md" } },
        new() { Category = "Spreadsheets", Extensions = new[] { ".xlsx", ".csv", ".tsv" } },
        new() { Category = "Presentations", Extensions = new[] { ".pptx" } },
        new() { Category = "Archives", Extensions = new[] { ".zip" } },
        new() { Category = "Logs & structured data", Extensions = new[] { ".log", ".json", ".xml", ".yaml", ".yml", ".ini", ".cfg", ".conf", ".toml", ".env" } },
        new() { Category = "Web", Extensions = new[] { ".htm", ".html", ".css", ".scss", ".less" } },
        new()
        {
            Category = "Code",
            Extensions = new[]
            {
                ".cs", ".java", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".rs", ".rb",
                ".php", ".swift", ".kt", ".c", ".h", ".cpp", ".hpp", ".sql"
            }
        },
        new() { Category = "Scripts", Extensions = new[] { ".ps1", ".psm1", ".bat", ".cmd", ".sh", ".zsh" } },
    };

    public static readonly IReadOnlyList<string> AllExtensions = Categories
        .SelectMany(c => c.Extensions)
        .Distinct(StringComparer.OrdinalIgnoreCase)
        .OrderBy(e => e, StringComparer.OrdinalIgnoreCase)
        .ToList();
}
