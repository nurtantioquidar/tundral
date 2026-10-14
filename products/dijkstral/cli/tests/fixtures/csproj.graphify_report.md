# Graph Report - csproj  (2026-10-15)

## Corpus Check
- 20 files · ~550 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 1 file(s) not represented in the graph (top: .props 1)

## Summary
- 118 nodes · 134 edges · 9 communities (8 shown, 1 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.53)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Window
- SettingsViewModel
- Home.cshtml
- Shop.csproj
- Shop.Tests.csproj
- Index.razor
- ItemService.cs
- UserControl
- ComponentBase

## God Nodes (most connected - your core abstractions)
1. `SettingsViewModel` - 11 edges
2. `Window` - 11 edges
3. `UserControl` - 11 edges
4. `MainWindow` - 7 edges
5. `Item` - 5 edges
6. `IItemService` - 5 edges
7. `Shop.Core` - 4 edges
8. `UserControl` - 4 edges
9. `SearchBox` - 4 edges
10. `SaveButton` - 4 edges

## Surprising Connections (you probably didn't know these)
- `UserControl` --references--> `Title`  [INFERRED]
  src/Shop/Views/SettingsView.xaml → src/Shop/ViewModels/SettingsViewModel.cs
- `UserControl` --references--> `UserName`  [INFERRED]
  src/Shop/Views/SettingsView.xaml → src/Shop/ViewModels/SettingsViewModel.cs
- `UserControl` --references--> `SaveCommand`  [INFERRED]
  src/Shop/Views/SettingsView.xaml → src/Shop/ViewModels/SettingsViewModel.cs
- `UserControl` --references--> `ResetCommand`  [INFERRED]
  src/Shop/Views/SettingsView.xaml → src/Shop/ViewModels/SettingsViewModel.cs
- `UserControl` --references--> `SettingsViewModel`  [EXTRACTED]
  src/Shop/Views/DesignView.xaml → src/Shop/ViewModels/SettingsViewModel.cs

## Import Cycles
- None detected.

## Communities (9 total, 1 thin omitted)

### Community 0 - "Window"
Cohesion: 0.10
Nodes (15): MoneyConverter, UpperConverter, Order.Total, Query, Status, Shop.Views, RootPanel, SaveButton (+7 more)

### Community 1 - "SettingsViewModel"
Cohesion: 0.11
Nodes (13): ResetCommand, Title, UserName, Shop.ViewModels, SettingsViewModel, Plain, ResetCommand, SaveCommand (+5 more)

### Community 2 - "Home.cshtml"
Cohesion: 0.15
Nodes (12): Shop.Core, HomeModel, Microsoft.AspNetCore.Components.Web, route:/home, Card, Item, IItemService, Item (+4 more)

### Community 3 - "Shop.csproj"
Cohesion: 0.14
Nodes (12): net48, $(TargetFrameworks), CommunityToolkit.Mvvm (8.2.2), Newtonsoft.Json, Serilog (3.1.1), System.ValueTuple (4.5.0), Microsoft.NET.Sdk.Web, net8.0 (+4 more)

### Community 4 - "Shop.Tests.csproj"
Cohesion: 0.17
Nodes (6): Dapper (2.1.35), Microsoft.NET.Test.Sdk (17.11.1), xunit (2.9.2), net8.0, net9.0, Microsoft.NET.Sdk

### Community 5 - "Index.razor"
Cohesion: 0.18
Nodes (9): LayoutComponentBase, route:/, route:/items/{id:int}, Load, OnInitialized, Card, Twice, System.Math (+1 more)

### Community 6 - "ItemService.cs"
Cohesion: 0.25
Nodes (6): Shop.Services, Wizard, Page, Steps, WizardPage, Frame

### Community 7 - "UserControl"
Cohesion: 0.40
Nodes (4): DesignView, Layout, UserControl, Grid

## Knowledge Gaps
- **49 isolated node(s):** `Dapper (2.1.35)`, `net8.0`, `Newtonsoft.Json`, `Microsoft.NET.Sdk`, `Name` (+44 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 72 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SettingsViewModel` connect `SettingsViewModel` to `UserControl`?**
  _High betweenness centrality (0.037) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `SettingsViewModel` (e.g. with `ResetCommand` and `SaveCommand`) actually correct?**
  _`SettingsViewModel` has 5 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Dapper (2.1.35)`, `net8.0`, `Newtonsoft.Json` to the rest of the system?**
  _49 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Window` be split into smaller, more focused modules?**
  _Cohesion score 0.09538461538461539 - nodes in this community are weakly interconnected._
- **Why does `IItemService` connect `Home.cshtml` to `Index.razor`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `UserControl` (e.g. with `SettingsViewModel` and `ResetCommand`) actually correct?**
  _`UserControl` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Should `SettingsViewModel` be split into smaller, more focused modules?**
  _Cohesion score 0.11255411255411256 - nodes in this community are weakly interconnected._