using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace Shop.ViewModels;

public partial class SettingsViewModel : ObservableObject
{
    [ObservableProperty]
    private string title = "";

    [ObservableProperty] private string _userName = "";

    public string Plain { get; set; }

    [RelayCommand]
    private async Task SaveAsync()
    {
        await Task.CompletedTask;
    }

    [RelayCommand]
    private void Reset()
    {
    }
}
