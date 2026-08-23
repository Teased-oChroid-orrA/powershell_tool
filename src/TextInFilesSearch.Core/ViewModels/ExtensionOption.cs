using TextInFilesSearch.Helpers;

namespace TextInFilesSearch.ViewModels;

/// <summary>
/// One tickable entry in the extension type-to-filter picker: either one of
/// the built-in catalog extensions (see Models.ExtensionCatalog) or a custom
/// one the user typed in and added themselves.
/// </summary>
public sealed class ExtensionOption : ObservableObject
{
    public required string Extension { get; init; }
    public required string Category { get; init; }

    private bool _isSelected;
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }

    public string Display => $"{Extension}  ({Category})";
}
