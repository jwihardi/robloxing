if (-not ('TestDesktop' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'TestDesktop.cs') }

function Invoke-WorkshopUiTest([string]$Script, [hashtable]$Parameters = @{}) {
    $name = 'WorkshopTest-' + [guid]::NewGuid().ToString('N')
    $request = [IO.Path]::GetTempFileName()
    $log = [IO.Path]::GetTempFileName()
    $run = $null
    try {
        @{
            Script = (Resolve-Path -LiteralPath $Script).Path
            Parameters = $Parameters
            Desktop = $name
            Helper = Join-Path $PSScriptRoot 'TestDesktop.ps1'
            Log = $log
        } | Export-Clixml -LiteralPath $request
        # Pass paths and arguments as data, including paths containing spaces or quotes.
        $encodedPath = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($request))
        $code = @'
$ErrorActionPreference = 'Stop'
$request = Import-Clixml -LiteralPath ([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('REQUEST_PATH')))
try {
    . $request.Helper
    if ([TestDesktop]::CurrentName() -ne $request.Desktop) { throw 'Could not isolate the UI test desktop.' }
    # The isolated child deliberately tests the UI even when the caller uses CLI-only mode.
    Remove-Item Env:WORKSHOP_HEADLESS -ErrorAction SilentlyContinue
    $params = $request.Parameters
    & $request.Script @params *>&1 | Out-File -LiteralPath $request.Log -Encoding utf8
    exit 0
} catch {
    ($_ | Out-String) + $_.ScriptStackTrace | Out-File -LiteralPath $request.Log -Encoding utf8 -Append
    exit 1
}
'@
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($code.Replace('REQUEST_PATH', $encodedPath)))
        $run = [TestDesktop]::Launch($name, (Get-Process -Id $PID).Path, $encoded, (Get-Location).ProviderPath)
        while (!$run.Finished()) { Start-Sleep -Milliseconds 200 }
        $status = $run.ExitCode()
        if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Encoding utf8 }
        if ($status -ne 0) { throw "Isolated UI test exited with code $status." }
    } finally {
        if ($run) { $run.Dispose() }
        Remove-Item -LiteralPath $request, $log -ErrorAction SilentlyContinue
    }
}
