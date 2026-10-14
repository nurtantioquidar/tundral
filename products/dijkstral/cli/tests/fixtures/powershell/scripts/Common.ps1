Set-StrictMode -Version Latest

function Write-BuildLog {
    param([string]$Message)
    Write-Host "[build] $Message"
}

function Get-RepoRoot {
    Split-Path -Parent $PSScriptRoot
}
