param([string]$Binary = ".\target\release\workshop.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WorkshopPointer {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref Point p);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    private delegate bool EnumChildProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumChildProc callback, IntPtr param);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr h,System.Text.StringBuilder text,int size);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr h, EnumChildProc callback, IntPtr param);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr h, System.Text.StringBuilder name, int size);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h,int id);
    public static IntPtr Dialog(int pid,string title) {
        IntPtr found=IntPtr.Zero;
        EnumWindows(delegate(IntPtr win,IntPtr param) {
            uint owner; GetWindowThreadProcessId(win,out owner);
            if(owner==pid){var name=new System.Text.StringBuilder(128);GetWindowText(win,name,128);if(name.ToString()==title){found=win;return false;}}
            return true;
        },IntPtr.Zero);
        return found;
    }
    public static IntPtr FilenameEdit(IntPtr dialog) {
        IntPtr found=IntPtr.Zero;
        EnumChildWindows(dialog, delegate(IntPtr child, IntPtr param) {
            var name=new System.Text.StringBuilder(128);
            GetClassName(child,name,name.Capacity);
            int id=GetDlgCtrlID(child);
            if(name.ToString()=="Edit" && (id==1148 || id==1152)){found=child;return false;}
            return true;
        },IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, string text);
}
'@
[void][WorkshopPointer]::SetThreadDpiAwarenessContext([IntPtr](-4))
$exe = (Resolve-Path -LiteralPath $Binary).Path
$root = Join-Path $env:TEMP ("ws-picker-clicks-" + [guid]::NewGuid())
$project = Join-Path $root 'project'
[void](New-Item -ItemType Directory -Path $project -Force)
$fixtureApp = Join-Path $root 'Workshop-test.exe'
Copy-Item -LiteralPath $exe -Destination $fixtureApp
$env:WORKSHOP_HOME = Join-Path $root 'data'
$env:WORKSHOP_PROBE_DIR = $root
$env:WORKSHOP_CODEX = Join-Path $root 'provider.exe'
$env:WORKSHOP_CLAUDE = $env:WORKSHOP_CODEX
$env:CODEX_HOME = Join-Path $root 'codex'
$env:CLAUDE_CONFIG_DIR = Join-Path $root 'claude'
& "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /r:System.Web.Extensions.dll "/out:$env:WORKSHOP_CODEX" (Join-Path $PSScriptRoot 'FixtureProvider.cs')
if ($LASTEXITCODE -ne 0) { throw 'Could not compile fixture' }

function Named($name) {
    $proc.Refresh()
    if ($proc.MainWindowHandle -eq [IntPtr]::Zero) { return }
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
    $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $name))
}
function Wait-For([scriptblock]$condition, $description) {
    $deadline = [datetime]::UtcNow.AddSeconds(8)
    do {
        if (& $condition) { return }
        Start-Sleep -Milliseconds 50
    } while ([datetime]::UtcNow -lt $deadline)
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
    $names = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ }
    throw "Timed out waiting for $description. Visible controls: $($names -join ', ')"
}
function Click($name, [double]$fraction = 0.5) {
    $el = Named $name
    if (!$el) { throw "Missing $name" }
    $rect = $el.Current.BoundingRectangle
    $point = [WorkshopPointer+Point]::new()
    $point.X = [int]($rect.X + $rect.Width * $fraction)
    $point.Y = [int]($rect.Y + $rect.Height / 2)
    $handle = $proc.MainWindowHandle
    [void][WorkshopPointer]::ScreenToClient($handle, [ref]$point)
    $pos = [IntPtr](($point.Y -shl 16) -bor ($point.X -band 65535))
    # InvokePattern skips hit testing and would pass even if focus swallowed the press.
    [void][WorkshopPointer]::PostMessage($handle, 0x200, 0, $pos)
    [void][WorkshopPointer]::PostMessage($handle, 0x201, 1, $pos)
    [void][WorkshopPointer]::PostMessage($handle, 0x202, 0, $pos)
}
function Choose-Attachment($title,$path) {
    Wait-For { [WorkshopPointer]::Dialog($proc.Id,$title) -ne [IntPtr]::Zero } $title
    $dialog = [System.Windows.Automation.AutomationElement]::FromHandle([WorkshopPointer]::Dialog($proc.Id,$title))
    if ($dialog.Current.ProcessId -ne $proc.Id) { throw 'Unexpected picker process' }
    $handle = [IntPtr]$dialog.Current.NativeWindowHandle
    Wait-For { [WorkshopPointer]::IsWindowVisible($handle) -and [WorkshopPointer]::IsWindowVisible([WorkshopPointer]::FilenameEdit($handle)) } 'visible native filename editor'
    [void]$proc.WaitForInputIdle(1000)
    $edit = [WorkshopPointer]::FilenameEdit($handle)
    [void][WorkshopPointer]::SendMessage($edit,0x000c,0,$path)
    [void][WorkshopPointer]::PostMessage($handle,0x0111,1,[WorkshopPointer]::GetDlgItem($handle,1))
    if ($title -eq 'Attach a folder') {
        # Some shell versions navigate into the typed folder before accepting it.
        Start-Sleep -Milliseconds 350
        if ([WorkshopPointer]::Dialog($proc.Id,$title) -eq $handle) {
            [void][WorkshopPointer]::PostMessage($handle,0x0111,1,[WorkshopPointer]::GetDlgItem($handle,1))
        }
    }
    Wait-For { [WorkshopPointer]::Dialog($proc.Id,$title) -eq [IntPtr]::Zero } "$title closing"
}
$proc = Start-Process -FilePath $fixtureApp -ArgumentList $project -WindowStyle Normal -PassThru
try {
    [void]$proc.WaitForInputIdle(10000)
    Wait-For { $el = Named 'Chat model and effort'; $el -and $el.Current.IsEnabled } 'model discovery'
    Click 'Message Codex'
    Click 'Chat model and effort'
    Wait-For { Named 'Chat reasoning effort' } 'first-click effort picker'
    Click 'Chat model'
    Wait-For { Named 'Select model' } 'model menu'
    Click 'fixture-deep'
    Wait-For { $slider = Named 'Chat reasoning effort'; $slider -and $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Maximum -eq 1 } 'selected model with two effort levels'
    Click 'Chat reasoning effort' 0.05
    Wait-For { (Named 'Chat reasoning effort').GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value -eq 0 } 'low effort'
    Click 'Chat reasoning effort' 0.95
    Wait-For { (Named 'Chat reasoning effort').GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value -eq 1 } 'high effort'
    Click 'Subagents'
    Wait-For { Named 'Chat subagents' } 'direct switch to chat settings'
    Click 'Use global defaults'
    Wait-For { Named 'Maximum subagent effort' } 'editable settings'
    Click 'Chat model and effort'
    Wait-For { Named 'Chat reasoning effort' } 'direct switch back to effort'
    Click 'Context and usage'
    Wait-For { Named 'Context window' } 'usage details'
    if ((Named 'Add to message').Current.BoundingRectangle.Right -gt (Named 'Subagents').Current.BoundingRectangle.X -or (Named 'Subagents').Current.BoundingRectangle.Right -gt (Named 'Chat model and effort').Current.BoundingRectangle.X) { throw 'Expected Add and Subagents on the left, model on the right' }
    Click 'Add to message'
    Wait-For { Named 'Paste text' } 'add menu'
    Click 'Subagents'
    Wait-For { Named 'Edit global defaults' } 'defaults entry in Subagents'
    Click 'Edit global defaults'
    Wait-For { Named 'Subagent defaults' } 'global settings'
    Click 'Subagents'
    Wait-For { !(Named 'Subagent defaults') } 'toggle closed'
    Click 'Settings'
    Wait-For { Named 'Sign in' } 'provider accounts'
    Click 'Sign in'
    Wait-For { Named 'Sign out' } 'provider login result'
    Click 'Sign out'
    Wait-For { Named 'Confirm' } 'sign-out confirmation'
    Click 'Confirm'
    Wait-For { (Named 'Signed out.') -and !(Named 'Sign out') } 'provider logout result'
    Click 'Plugins'
    Wait-For { Named 'No plugins installed.' } 'installed plugins'
    Click 'Browse plugins'
    Wait-For { Named 'Sample plugin' } 'marketplace catalog'
    (Named 'Plugin source').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('sample@fixture')
    Click 'Install'
    Wait-For { Named 'Uninstall' } 'installed plugin'
    Click 'Uninstall'
    Wait-For { Named 'Confirm' } 'plugin removal confirmation'
    Click 'Confirm'
    Wait-For { Named 'Plugin uninstalled.' } 'plugin removal'
    Click 'Skills'
    $skill = Join-Path $root 'sample-skill'
    [void](New-Item -ItemType Directory -Path $skill)
    Set-Content (Join-Path $skill 'SKILL.md') "---`nname: sample-skill`ndescription: Test skill`n---"
    Wait-For { Named 'Skill source' } 'skill installer'
    (Named 'Skill source').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($skill)
    Click 'Install'
    Wait-For { Named 'sample-skill' } 'installed skill'
    if (!(Test-Path (Join-Path $env:CODEX_HOME 'skills/sample-skill/SKILL.md'))) { throw 'Skill was not installed' }
    (Named 'Filter list').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('sample-skill')
    Click 'Uninstall'
    Wait-For { Named 'Confirm' } 'skill removal confirmation'
    Click 'Confirm'
    Wait-For { !(Test-Path (Join-Path $env:CODEX_HOME 'skills/sample-skill')) } 'skill removal'
    if (!(Get-ChildItem (Join-Path $env:CODEX_HOME 'removed-skills') -Recurse -Filter SKILL.md)) { throw 'Missing archived skill' }
    Click 'Close settings'
    Wait-For { Named 'Message Codex' } 'return to composer'
    $note = Join-Path $root 'attached note.txt'
    Set-Content $note 'Attachment transport check'
    Click 'Add to message'
    Wait-For { Named 'Attach files…' } 'attachment menu'
    Click 'Attach files…'
    Choose-Attachment 'Attach files' $note
    Wait-For { Named 'Remove attached note.txt' } 'file in draft'
    Click 'Remove attached note.txt'
    Wait-For { !(Named 'Remove attached note.txt') } 'remove draft attachment'
    $image = Join-Path $root 'sample.png'
    [IO.File]::WriteAllBytes($image,[Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg=='))
    Click 'Add to message'
    Wait-For { Named 'Attach files…' } 'attachment menu'
    Click 'Attach files…'
    Choose-Attachment 'Attach files' ('"' + $note + '" "' + $image + '"')
    Wait-For { (Named 'Remove attached note.txt') -and (Named 'Remove sample.png') } 'multiple attachments'
    Click 'Add to message'
    Wait-For { Named 'Attach folder…' } 'folder command'
    Click 'Attach folder…'
    Choose-Attachment 'Attach a folder' $project
    [void](New-Item -ItemType File (Join-Path $root 'record-inference'))
    Click 'Send message'
    Wait-For { Get-ChildItem $root -Filter 'lead-*.json' } 'attachment-only turn'
    $wire = Get-Content (Get-ChildItem $root -Filter 'lead-*.json' | Select-Object -First 1).FullName -Raw | ConvertFrom-Json
    if (!(($wire.input | Where-Object type -eq 'text').text -match 'Attachment transport check')) { throw 'Provider did not receive attached text' }
    if (($wire.input | Where-Object type -eq 'localImage').path -ne $image) { throw 'Provider did not receive the image' }
    if ($wire.input[0].text -notlike "*$project*") { throw 'Provider did not receive the selected folder' }
    'Composer, attachments and provider settings passed'
} finally {
    if (!$proc.HasExited) {
        [void]$proc.CloseMainWindow()
        if (!$proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit() }
    }
}
