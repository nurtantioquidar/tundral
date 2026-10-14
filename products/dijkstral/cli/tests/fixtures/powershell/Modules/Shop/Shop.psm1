using namespace System.Collections.Generic
using module ./Private/Pricing.psm1

enum OrderStatus {
    Open
    Paid
    Shipped
}

class Order {
    [string]$Customer
    [decimal]$Total
    [OrderStatus]$Status
    hidden [List[string]]$Notes

    Order([string]$customer, [decimal]$total) {
        $this.Customer = $customer
        $this.Total = $total
        $this.Status = [OrderStatus]::Open
    }

    [decimal] WithTax() {
        return Get-TaxedAmount -Amount $this.Total
    }

    [void] MarkPaid() {
        $this.Status = [OrderStatus]::Paid
        $this.Log('paid')
    }

    hidden [void] Log([string]$message) {
        Write-Verbose $message
    }
}

class PriorityOrder : Order {
    [int]$Rank

    PriorityOrder([string]$customer, [decimal]$total, [int]$rank) : base($customer, $total) {
        $this.Rank = $rank
    }

    [Order] Downgrade() {
        return [Order]::new($this.Customer, $this.Total)
    }
}

function New-ShopOrder {
    param(
        [Parameter(Mandatory)][string]$Customer,
        [decimal]$Total
    )
    $order = [Order]::new($Customer, $Total)
    Write-ShopLog "created order for $Customer"
    return $order
}

function Get-OrderTotal {
    param([Order[]]$Orders)
    ($Orders | Measure-Object -Property Total -Sum).Sum
}

function Write-ShopLog([string]$Message) {
    Write-Verbose $Message
}

Export-ModuleMember -Function New-ShopOrder, Get-OrderTotal
