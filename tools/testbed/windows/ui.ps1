# UI Automation helpers for the Orither test VM, dot-sourced by test commands.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [Windows.Automation.AutomationElement]::RootElement
function Find-UI([string]$name, [string]$window = $null) {
  $scope = $root
  if ($window) {
    $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $window)
    $scope = $root.FindFirst([Windows.Automation.TreeScope]::Children, $cond)
    if (-not $scope) { throw "no window '$window'" }
  }
  $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
  $scope.FindFirst([Windows.Automation.TreeScope]::Descendants, $cond)
}
function Click-UI([string]$name, [string]$window = $null) {
  $el = Find-UI $name $window
  if (-not $el) { throw "no element '$name'" }
  $pattern = $null
  if ($el.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) { $pattern.Invoke(); "clicked $name"; return }
  if ($el.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) { $pattern.Select(); "selected $name"; return }
  throw "'$name' cannot be clicked"
}
function Get-Windows { $root.FindAll([Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ } }
function Get-Texts([string]$window) {
  $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $window)
  $w = $root.FindFirst([Windows.Automation.TreeScope]::Children, $cond)
  if (-not $w) { throw "no window '$window'" }
  $w.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ } | Select-Object -Unique
}
