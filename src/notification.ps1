$ErrorActionPreference = 'Stop'
$owner = [Diagnostics.Process]::GetProcessById([int]$env:WORKSHOP_NOTICE_PARENT)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WorkshopActivation {
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
}
'@
$context = [Windows.Forms.ApplicationContext]::new()
$icon = [Windows.Forms.NotifyIcon]::new()
$timer = [Windows.Forms.Timer]::new()
$activate = {
    if (!$env:WORKSHOP_TEST_NOTICE_DIR) {
        $owner.Refresh()
        $window = $owner.MainWindowHandle
        if ([WorkshopActivation]::IsIconic($window)) { [void][WorkshopActivation]::ShowWindowAsync($window, 9) }
        [void][WorkshopActivation]::SetForegroundWindow($window)
    }
    [Console]::WriteLine('clicked')
    $context.ExitThread()
}
try {
    $icon.Icon = [Drawing.SystemIcons]::Application
    $icon.Text = 'Workshop'
    $icon.add_BalloonTipClicked($activate)
    $icon.add_Click($activate)
    $icon.add_BalloonTipClosed({ $context.ExitThread() })
    $deadline = [datetime]::UtcNow.AddMinutes(3)
    $timer.Interval = 1000
    $timer.add_Tick({ if ($owner.HasExited -or [datetime]::UtcNow -ge $deadline) { $context.ExitThread() } })
    if ($env:WORKSHOP_TEST_NOTICE_DIR) {
        # Keep the native message loop and click handler, with delivery confined to the private desktop test.
        $file = Join-Path $env:WORKSHOP_TEST_NOTICE_DIR ([guid]::NewGuid().ToString() + '.json')
        @{title=$env:WORKSHOP_NOTICE_TITLE; detail=$env:WORKSHOP_NOTICE_DETAIL} | ConvertTo-Json | Set-Content -LiteralPath ($file + '.tmp')
        Move-Item -LiteralPath ($file + '.tmp') -Destination $file
        $timer.Interval = 100
        $timer.add_Tick({ if (Test-Path -LiteralPath ($file + '.click')) { & $activate } })
    } else {
        $icon.Visible = $true
        $icon.ShowBalloonTip(10000, $env:WORKSHOP_NOTICE_TITLE, $env:WORKSHOP_NOTICE_DETAIL, [Windows.Forms.ToolTipIcon]::Info)
    }
    $timer.Start()
    [Windows.Forms.Application]::Run($context)
} finally {
    $timer.Dispose()
    $icon.Dispose()
    $context.Dispose()
    $owner.Dispose()
}
