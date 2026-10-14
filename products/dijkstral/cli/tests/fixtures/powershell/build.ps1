#Requires -Version 5.1
param(
    [string]$Configuration = 'Release'
)

# NOTE: the build loads the module from source, not from the gallery
Import-Module "$PSScriptRoot/Modules/Shop/Shop.psd1" -Force
Import-Module Pester
. ./scripts/Common.ps1

function Invoke-Build {
    [CmdletBinding()]
    param([string]$Target)
    Write-BuildLog "Building $Target"
    $order = New-ShopOrder -Customer 'ci' -Total 10
    Format-Result $order
    Get-ChildItem -Path ./Modules | ForEach-Object { Test-Manifest $_ }
}

function Format-Result($result) {
    # WHY: CI parses this line
    Write-Output "result=$result"
}

filter Test-Manifest {
    Write-BuildLog $_.Name
}

Invoke-Build -Target $Configuration
