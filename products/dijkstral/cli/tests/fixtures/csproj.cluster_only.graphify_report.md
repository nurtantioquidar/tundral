# Graph Report - csproj  (2026-10-15)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 113 nodes · 134 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.58)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6
- Community 7

## God Nodes (most connected - your core abstractions)
1. `Shop` - 12 edges
2. `SettingsViewModel` - 11 edges
3. `Window` - 11 edges
4. `UserControl` - 11 edges
5. `Shop.Tests` - 9 edges
6. `MainWindow` - 7 edges
7. `Core` - 6 edges
8. `IItemService` - 5 edges
9. `Item` - 5 edges
10. `SaveButton` - 4 edges

## Surprising Connections (you probably didn't know these)
- `Shop.Tests` --references--> `Microsoft.NET.Sdk`  [EXTRACTED]
  tests/Shop.Tests/Shop.Tests.csproj → src/Core/Core.csproj
- `Shop.Tests` --references--> `net9.0`  [EXTRACTED]
  tests/Shop.Tests/Shop.Tests.csproj → src/Shop/Shop.csproj
- `UserControl` --references--> `SaveCommand`  [EXTRACTED]
  src/Shop/Views/SettingsView.xaml → src/Shop/Views/MainWindow.xaml
- `Shop.Tests` --references--> `net8.0`  [EXTRACTED]
  tests/Shop.Tests/Shop.Tests.csproj → src/Core/Core.csproj
- `UserControl` --references--> `ResetCommand`  [INFERRED]
  src/Shop/Views/SettingsView.xaml → src/Shop/ViewModels/SettingsViewModel.cs

## Import Cycles
- None detected.

## Communities (8 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.10
Nodes (15): MoneyConverter, UpperConverter, Order.Total, Query, Status, Shop.Views, RootPanel, SaveButton (+7 more)

### Community 1 - "Community 1"
Cohesion: 0.11
Nodes (18): net48, $(TargetFrameworks), Legacy, CommunityToolkit.Mvvm (8.2.2), Dapper (2.1.35), Microsoft.NET.Test.Sdk (17.11.1), Newtonsoft.Json, Serilog (3.1.1) (+10 more)

### Community 2 - "Community 2"
Cohesion: 0.12
Nodes (12): ResetCommand, Title, UserName, Shop.ViewModels, SettingsViewModel, Plain, ResetCommand, SaveCommand (+4 more)

### Community 3 - "Community 3"
Cohesion: 0.15
Nodes (12): Shop.Core, HomeModel, Microsoft.AspNetCore.Components.Web, route:/home, Card, Item, IItemService, Item (+4 more)

### Community 4 - "Community 4"
Cohesion: 0.18
Nodes (9): LayoutComponentBase, route:/, route:/items/{id:int}, Load, OnInitialized, Card, Twice, System.Math (+1 more)

### Community 5 - "Community 5"
Cohesion: 0.25
Nodes (6): Shop.Services, Wizard, Page, Steps, WizardPage, Frame

### Community 6 - "Community 6"
Cohesion: 0.40
Nodes (4): DesignView, Layout, UserControl, Grid

## Knowledge Gaps
- **40 isolated node(s):** `MoneyConverter`, `UpperConverter`, `Order.Total`, `Query`, `Status` (+35 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 63 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `UserControl` connect `Community 2` to `Community 0`?**
  _High betweenness centrality (0.123) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `SettingsViewModel` (e.g. with `ResetCommand` and `SaveCommand`) actually correct?**
  _`SettingsViewModel` has 5 INFERRED edges - model-reasoned connections that need verification._
- **What connects `MoneyConverter`, `UpperConverter`, `Order.Total` to the rest of the system?**
  _40 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.09538461538461539 - nodes in this community are weakly interconnected._
- **Why does `SaveCommand` connect `Community 0` to `Community 2`?**
  _High betweenness centrality (0.105) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `UserControl` (e.g. with `SettingsViewModel` and `ResetCommand`) actually correct?**
  _`UserControl` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.11462450592885376 - nodes in this community are weakly interconnected._