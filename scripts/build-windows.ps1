# Build a runnable Windows release folder, including the GNU WebView2 loader.
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $loader = $null
    $app = $null
    cargo build --locked --release --bins --message-format=json-render-diagnostics | ForEach-Object {
        $record = $_ | ConvertFrom-Json
        if ($record.reason -eq 'build-script-executed' -and $record.package_id -match 'webview2-com-sys') {
            foreach ($path in $record.linked_paths) {
                $candidate = Join-Path ($path -replace '^native=', '') 'WebView2Loader.dll'
                if (Test-Path -LiteralPath $candidate) { $loader = $candidate }
            }
        }
        if ($record.reason -eq 'compiler-artifact' -and $record.target.name -eq 'workshop' -and $record.executable) {
            $app = $record.executable
        }
    }
    if ($LASTEXITCODE -ne 0) { throw 'Workshop build failed' }
    if (!$app -or !$loader) { throw 'Could not locate the Windows desktop executable and WebView2 loader in Cargo output' }
    Copy-Item -LiteralPath $loader -Destination (Split-Path $app) -Force
    $app
} finally { Pop-Location }
