class TaxRule {
    [decimal]$Rate
    TaxRule([decimal]$rate) { $this.Rate = $rate }
    [decimal] Apply([decimal]$amount) { return $amount * (1 + $this.Rate) }
}

function Get-TaxedAmount {
    param([decimal]$Amount)
    $rule = [TaxRule]::new(0.2)
    return $rule.Apply($Amount)
}

function Get-Discount([decimal]$Amount) {
    if ($Amount -gt 100) { return Get-TaxedAmount -Amount 5 }
    0
}
