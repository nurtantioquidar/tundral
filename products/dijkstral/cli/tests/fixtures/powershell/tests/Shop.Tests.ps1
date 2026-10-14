BeforeAll {
    Import-Module "$PSScriptRoot/../Modules/Shop/Shop.psd1" -Force
    . $PSScriptRoot/../scripts/Common.ps1
}

Describe 'New-ShopOrder' {
    It 'creates an order' {
        (New-ShopOrder -Customer 'a' -Total 1).Customer | Should -Be 'a'
    }
}

function New-TestOrder {
    New-ShopOrder -Customer 'test' -Total 0
    Write-BuildLog 'test order'
}
