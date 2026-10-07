# Invoke a button by its accessible name inside a process's window (UI Automation).
# tools\uiclick.ps1 -Name beamscope -Button "Connect camera"
param([string]$Name = "beamscope", [string]$Button)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$p = Get-Process -Name $Name | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { throw "no window for $Name" }
$root = [System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$cond = New-Object System.Windows.Automation.AndCondition(
  (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)),
  (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Button)))
$deadline = (Get-Date).AddSeconds(5)
do {
  $el = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
  if ($el) { break }
  Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $deadline)
if (-not $el) { throw "button '$Button' not found" }
$el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
"clicked '$Button'"
