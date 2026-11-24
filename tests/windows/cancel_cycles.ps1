param([string]$Binary = ".\target\release\workshop.exe")
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'TestDesktop.ps1')
if (![TestDesktop]::CurrentName().StartsWith('WorkshopTest-')) {
    $parameters = @{ Binary = (Resolve-Path -LiteralPath $Binary).Path }
    Invoke-WorkshopUiTest -Script $PSCommandPath -Parameters $parameters
    return
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$exe = (Resolve-Path -LiteralPath $Binary).Path
$root = Join-Path $env:TEMP ("ws-cycles-" + [guid]::NewGuid())
$project = Join-Path $root 'project'
[void](New-Item -ItemType Directory -Path $project -Force)
$fixtureApp = Join-Path $root 'Workshop-test.exe'
Copy-Item -LiteralPath $exe -Destination $fixtureApp
$env:WORKSHOP_HOME = Join-Path $root 'data'
$env:WORKSHOP_PROBE_DIR = $root
$env:WORKSHOP_CODEX = Join-Path $root 'provider.exe'
& "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /r:System.Web.Extensions.dll "/out:$env:WORKSHOP_CODEX" (Join-Path $PSScriptRoot 'FixtureProvider.cs')
if ($LASTEXITCODE -ne 0) { throw 'Could not compile fixture' }
[void](New-Item -ItemType File -Path (Join-Path $root 'left.hold'))
$ready = Join-Path $root 'left.ready'
$proc = [TestDesktop]::StartApp($fixtureApp, $project)
$ip = [System.Windows.Automation.InvokePattern]::Pattern
$vp = [System.Windows.Automation.ValuePattern]::Pattern
function Named([string]$name) {
    $proc.Refresh()
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty,$name)
    $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
}
function Wait-For([scriptblock]$condition,[string]$label) {
    $deadline = [datetime]::UtcNow.AddSeconds(40)
    do { if (& $condition) { return }; Start-Sleep -Milliseconds 100 } while ([datetime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $label"
}
function Children {
    $rows = @(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId)
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($proc.Id)
    do {
        $before = $ids.Count
        foreach ($row in $rows) {
            if ($ids.Contains([int]$row.ParentProcessId)) { [void]$ids.Add([int]$row.ProcessId) }
        }
    } while ($ids.Count -gt $before)
    [void]$ids.Remove($proc.Id)
    $ids
}
try {
    $appWindow = [TestDesktop]::WaitForWindow($proc)
    Wait-For { Named 'Message Codex' } 'composer'
    foreach ($cycle in 1..10) {
        if (Test-Path -LiteralPath $ready) { Remove-Item -LiteralPath $ready }
        (Named 'Message Codex').GetCurrentPattern($vp).SetValue("Cycle $cycle")
        (Named 'Send message').GetCurrentPattern($ip).Invoke()
        Wait-For { (Test-Path -LiteralPath $ready) -and (Named 'Stop response') } 'active worker'
        $ids = @(Children)
        if ($ids.Count -lt 3) { throw 'Expected lead, worker, and worker child' }
        (Named 'Stop response').GetCurrentPattern($ip).Invoke()
        Wait-For { (Named 'New chat').Current.IsEnabled -and !(Named 'Stop response') } 'cancel completion'
        Wait-For { @(Get-Process -Id $ids -ErrorAction SilentlyContinue).Count -eq 0 } 'job cleanup'
        (Named 'New chat').GetCurrentPattern($ip).Invoke()
        $proc.Refresh()
        [pscustomobject]@{cycle=$cycle; descendants_released=$ids.Count; private_mib=[math]::Round($proc.PrivateMemorySize64/1MB,2)} | ConvertTo-Json -Compress
    }
} finally {
    [TestDesktop]::CloseApp($proc, $appWindow)
}
