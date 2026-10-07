# Repro: connect camera, turn auto exposure off, drag the exposure slider down step by step.
param([string]$Exe = "C:\Users\gosha\Documents\dataray-re\beamscope\src-tauri\target\repro\release\beamscope.exe")
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.TreeScope]::Descendants

$crash = Join-Path $env:LOCALAPPDATA "BeamScope\crash.log"
if (Test-Path $crash) { Remove-Item $crash }
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep 6
$p.Refresh()
$root = $A::FromHandle($p.MainWindowHandle)

function Find($name, $type) {
  $c = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, $type)),
    (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $name)))
  for ($i = 0; $i -lt 20; $i++) { $e = $root.FindFirst($T, $c); if ($e) { return $e }; Start-Sleep -Milliseconds 250 }
  throw "not found: $name"
}

(Find "Connect camera" ([System.Windows.Automation.ControlType]::Button)).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep 4
$ae = Find "Auto exposure" ([System.Windows.Automation.ControlType]::CheckBox)
$ae.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
"auto exposure toggled"
Start-Sleep 1
$sl = Find "Exposure" ([System.Windows.Automation.ControlType]::Slider)
$rv = $sl.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
"slider value $($rv.Current.Value)"
for ($v = [int]$rv.Current.Value; $v -ge 0; $v -= 10) {
  try { $rv.SetValue($v) } catch { "SetValue failed at $v : $_"; break }
  Start-Sleep -Milliseconds 60
  if ($p.HasExited) { "APP EXITED at slider $v (code $($p.ExitCode))"; break }
}
Start-Sleep 2
if ($p.HasExited) { "exited, code $($p.ExitCode)" } else { "still running" }
if (Test-Path $crash) { "---- crash.log"; Get-Content $crash -TotalCount 60 }
