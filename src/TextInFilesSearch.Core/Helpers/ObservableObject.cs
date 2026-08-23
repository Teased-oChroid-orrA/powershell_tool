using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace TextInFilesSearch.Helpers;

/// <summary>
/// Minimal INotifyPropertyChanged base class. Deliberately dependency-free
/// (no CommunityToolkit.Mvvm) so the whole ViewModel layer can be compiled and
/// unit-tested in any plain .NET environment, not just one with NuGet
/// restore available - which mattered directly during development, since the
/// engine and ViewModel logic were verified with a real compiler and test
/// suite before ever touching a Windows machine.
/// </summary>
public abstract class ObservableObject : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    protected bool SetProperty<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value)) return false;
        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    protected void OnPropertyChanged([CallerMemberName] string? propertyName = null)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}
