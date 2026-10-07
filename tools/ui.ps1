# Small UI Automation driver for BeamScope.
#   tools\ui.ps1 -Click "Simulator"            invoke a button
#   tools\ui.ps1 -Toggle "Log scale"           toggle a checkbox
#   tools\ui.ps1 -Launch <exe>                 start the app and wait for its window
param([string]$Launch, [string]$Click, [string]$Toggle, [string]$Name = "beamscope")
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$CT = [System.Windows.Automation.ControlType]

if ($Launch) {
  $p = Start-Process -FilePath $Launch -PassThru
  for ($i = 0; $i -lt 40 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 250; $p.Refresh() }
  Start-Sleep 2
  "launched pid $($p.Id)"
}

function Find($label, $type) {
  $p = Get-Process -Name $Name | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  $root = $A::FromHandle($p.MainWindowHandle)
  $c = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, $type)),
    (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $label)))
  for ($i = 0; $i -lt 20; $i++) {
    $e = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
    if ($e) { return $e }
    Start-Sleep -Milliseconds 250
  }
  throw "not found: $label"
}

if ($Click) {
  (Find $Click $CT::Button).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
  "clicked '$Click'"
}
if ($Toggle) {
  (Find $Toggle $CT::CheckBox).GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
  "toggled '$Toggle'"
}
