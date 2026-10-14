@{
    RootModule        = 'Shop.psm1'
    ModuleVersion     = '1.2.0'
    NestedModules     = @('Private/Pricing.psm1')
    RequiredModules   = @(
        'PSFramework',
        @{ ModuleName = 'Configuration'; ModuleVersion = '1.5' }
    )
    FunctionsToExport = @('New-ShopOrder', 'Get-OrderTotal')
}
