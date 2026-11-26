param([string]$Binary = ".\target\release\workshop.exe", [switch]$StudioOnly, [switch]$SketchOnly, [switch]$RecommendedOnly, [switch]$DeliveryOnly, [switch]$TranscriptOnly, [switch]$UsageOnly, [switch]$ProviderTasksOnly, [switch]$NavigationOnly, [switch]$GoalOnly)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'TestDesktop.ps1')
if (![TestDesktop]::CurrentName().StartsWith('WorkshopTest-')) {
    $parameters = @{ Binary = (Resolve-Path -LiteralPath $Binary).Path }
    if ($GoalOnly) { $parameters.GoalOnly = $true }
    if ($NavigationOnly) { $parameters.NavigationOnly = $true }
    if ($ProviderTasksOnly) { $parameters.ProviderTasksOnly = $true }
    if ($StudioOnly) { $parameters.StudioOnly = $true }
    if ($SketchOnly) { $parameters.SketchOnly = $true }
    if ($RecommendedOnly) { $parameters.RecommendedOnly = $true }
    if ($DeliveryOnly) { $parameters.DeliveryOnly = $true }
    if ($TranscriptOnly) { $parameters.TranscriptOnly = $true }
    if ($UsageOnly) { $parameters.UsageOnly = $true }
    Invoke-WorkshopUiTest -Script $PSCommandPath -Parameters $parameters
    return
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WorkshopPointer {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref Point p);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr target, uint flags);
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
    public static IntPtr FilenameEdit(IntPtr dialog, bool save) {
        IntPtr found=IntPtr.Zero;
        EnumChildWindows(dialog, delegate(IntPtr child, IntPtr param) {
            var name=new System.Text.StringBuilder(128);
            GetClassName(child,name,name.Capacity);
            int id=GetDlgCtrlID(child);
            if(name.ToString()=="Edit" && (id==1148 || id==1152 || (save && id==1001))){found=child;return false;}
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
$env:WORKSHOP_TEST_NOTICE_DIR = (New-Item -ItemType Directory -Path (Join-Path $root 'notifications')).FullName
$env:WORKSHOP_CODEX = Join-Path $root 'provider.exe'
$env:WORKSHOP_CLAUDE = $env:WORKSHOP_CODEX
$env:WORKSHOP_GEMINI = $env:WORKSHOP_CODEX
$env:WORKSHOP_ANTIGRAVITY = $env:WORKSHOP_CODEX
$env:CODEX_HOME = Join-Path $root 'codex'
$env:CLAUDE_CONFIG_DIR = Join-Path $root 'claude'
# Route only this public skill URL to a temporary fixture. The normal Git installer runs offline.
$upstream = Join-Path $root 'skill-source'
$subdir = Join-Path $upstream 'skills/image-to-sculpt'
[void](New-Item -ItemType Directory -Path $subdir -Force)
Set-Content (Join-Path $subdir 'SKILL.md') 'Image to Sculpt fixture'
& git init -q -b main $upstream
& git -C $upstream add skills
& git -C $upstream -c user.name=Fixture -c user.email=fixture@localhost commit -qm 'Skill source'
if ($LASTEXITCODE -ne 0) { throw 'Could not create the test skill source' }
$idx = [int]$env:GIT_CONFIG_COUNT
[Environment]::SetEnvironmentVariable("GIT_CONFIG_KEY_$idx", "url.$(([uri]$upstream).AbsoluteUri).insteadOf")
[Environment]::SetEnvironmentVariable("GIT_CONFIG_VALUE_$idx", 'https://github.com/gokayfem/image-to-sculpt.git')
$env:GIT_CONFIG_COUNT = $idx + 1
# Simulated interactive sign-in must never allocate a terminal on the user's desktop.
& "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /target:winexe /r:System.Web.Extensions.dll "/out:$env:WORKSHOP_CODEX" (Join-Path $PSScriptRoot 'FixtureProvider.cs')
if ($LASTEXITCODE -ne 0) { throw 'Could not compile fixture' }

function Named($name) {
    $proc.Refresh()
    if ($appWindow -eq [IntPtr]::Zero) { return }
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
    $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $name))
}
function Wait-For([scriptblock]$condition, $description, [int]$seconds = 8) {
    $deadline = [datetime]::UtcNow.AddSeconds($seconds)
    do {
        if (& $condition) { return }
        Start-Sleep -Milliseconds 50
    } while ([datetime]::UtcNow -lt $deadline)
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
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
    $handle = $appWindow
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
    $save = $title -eq 'Save asset'
    Wait-For { [WorkshopPointer]::IsWindowVisible($handle) -and [WorkshopPointer]::IsWindowVisible([WorkshopPointer]::FilenameEdit($handle,$save)) } 'visible native filename editor'
    $edit = [WorkshopPointer]::FilenameEdit($handle,$save)
    [void]$proc.WaitForInputIdle(1000)
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
function Draw-SketchLine {
    $rect = (Named 'Sketch canvas').Current.BoundingRectangle
    foreach ($i in 0..8) {
        $p = [WorkshopPointer+Point]::new()
        $p.X = [int]($rect.X + $rect.Width * (0.20 + $i * 0.075))
        $p.Y = [int]($rect.Y + $rect.Height * (0.65 - $i * 0.0375))
        [void][WorkshopPointer]::ScreenToClient($appWindow,[ref]$p)
        $pos = [IntPtr](($p.Y -shl 16) -bor ($p.X -band 65535))
        [void][WorkshopPointer]::PostMessage($appWindow,0x200,[int]($i -gt 0),$pos)
        if ($i -eq 0) { [void][WorkshopPointer]::PostMessage($appWindow,0x201,1,$pos) }
        Start-Sleep -Milliseconds 15
    }
    [void][WorkshopPointer]::PostMessage($appWindow,0x202,0,$pos)
}
function Open-SketchPad {
    Click 'Add to message'
    Wait-For { Named 'Draw a sketch' } 'sketch menu item'
    Click 'Draw a sketch'
    Wait-For { Named 'Sketch canvas' } 'drawing surface'
}
function Test-ProviderTasks {
    $toggle = [System.Windows.Automation.TogglePattern]::Pattern
    function Count-Pixels($image) {
        $bounds = (Named 'Codex').Current.BoundingRectangle
        $dpi = [WorkshopPointer]::GetDpiForWindow($appWindow) / 96.0
        $p = [WorkshopPointer+Point]::new()
        $p.X = [int]($bounds.Right-86*$dpi); $p.Y = [int]($bounds.Top+$bounds.Height/2-8*$dpi)
        [void][WorkshopPointer]::ScreenToClient($appWindow,[ref]$p)
        $bmp = [Drawing.Bitmap]::new($image)
        $mask = [Text.StringBuilder]::new()
        try {
            for ($y=0; $y -lt 16*$dpi; $y++) { for ($x=0; $x -lt 76*$dpi; $x++) {
                $pixel=$bmp.GetPixel($p.X+$x,$p.Y+$y)
                [void]$mask.Append([int]([int]$pixel.R+$pixel.G+$pixel.B -gt 350))
            } }
            $mask.ToString()
        } finally { $bmp.Dispose() }
    }
    Click 'Subagents'
    Click 'Provider settings'
    Wait-For { Named 'Codex' } 'providers from the chat shortcut'
    Click 'Codex'
    Wait-For { (Named 'Workers') -and (Named 'Image generation') } 'Codex task choices'
    $before = Join-Path $root 'all-provider-tasks.png'
    Save-PrivateWindow $before
    $all = Count-Pixels $before
    Click 'Workers'
    Wait-For { (Named 'Workers').GetCurrentPattern($toggle).Current.ToggleState -eq 'Off' } 'Codex workers disabled'
    if ((Named 'Image generation').GetCurrentPattern($toggle).Current.ToggleState -ne 'On') { throw 'Changing workers changed image generation' }
    $after = Join-Path $root 'provider-tasks.png'
    Wait-For { Save-PrivateWindow $after; (Count-Pixels $after) -ne $all } 'visible provider task count changes with the checkbox'
    Click 'Claude Code'
    Wait-For { (Named 'Workers') -and !(Named 'Image generation') } 'Claude supported tasks'
    if ((Named 'Workers').GetCurrentPattern($toggle).Current.ToggleState -ne 'On') { throw 'Changing Codex workers changed Claude workers' }
    Click 'Close settings'
    Click 'Subagents'
    if ((Named 'Use global defaults').GetCurrentPattern($toggle).Current.ToggleState -eq 'On') { Click 'Use global defaults' }
    Wait-For { (Named 'Claude Sonnet 5.5') -and !(Named 'fixture-fast') } 'worker choices honor provider permissions'
    Click 'Provider settings'
    Click 'Codex'
    Wait-For { (Named 'Workers').GetCurrentPattern($toggle).Current.ToggleState -eq 'Off' } 'remembered Codex worker permission'
    Click 'Workers'
    Click 'Image generation'
    Click 'Antigravity'
    Wait-For { (Named 'Image generation').GetCurrentPattern($toggle).Current.ToggleState -eq 'On' } 'independent Antigravity image permission'
    Click 'Image generation'
    Click 'Tasks'
    Wait-For { (Named 'Task defaults') -or (Named 'Worker defaults') } 'task settings opened'
    Click 'Image generation'
    Wait-For { (Named 'Allowed models') -and !(Named 'Any available model') } 'all image providers disabled'
    Click 'Choose providers'
    Wait-For { (Named 'Image generation').GetCurrentPattern($toggle).Current.ToggleState -eq 'Off' } 'remembered Antigravity image permission'
    Click 'Image generation'
    Click 'Codex'
    Wait-For { (Named 'Image generation').GetCurrentPattern($toggle).Current.ToggleState -eq 'Off' } 'remembered Codex image permission'
    Click 'Image generation'
    Click 'Tasks'
    Wait-For { (Named 'Task defaults') -and (Named 'Workers') } 'worker defaults in task settings'
    Click 'Workers'
    Wait-For { (Named 'Worker defaults') -and (Named 'Maximum worker effort') -and (Named 'fixture-fast') } 'worker model and effort defaults'
    $auto = 'Auto-delegate project tasks'
    Wait-For { (Named $auto).GetCurrentPattern($toggle).Current.ToggleState -eq 'On' } 'default automatic delegation'
    Click $auto
    Click 'Image generation'
    Wait-For { !(Named $auto) } 'independent Studio image defaults'
    Click 'Workers'
    Wait-For { (Named $auto).GetCurrentPattern($toggle).Current.ToggleState -eq 'Off' } 'remembered scheduling preference'
    Click $auto
    Click 'Image generation'
    Click 'Providers'
    Wait-For { Named 'Workers' } 'provider details preserved'
    Click 'Codex'
    Wait-For { !(Named 'Workers') } 'collapsed provider'
    Click 'Close settings'
}
function Save-PrivateWindow($path) {
    $rect = [WorkshopPointer+Rect]::new()
    if (![WorkshopPointer]::GetClientRect($appWindow,[ref]$rect)) { throw 'Could not measure the private Workshop window' }
    $bmp = [Drawing.Bitmap]::new($rect.Right, $rect.Bottom)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    try {
        # Render client content into client-sized pixels, including the entire composer at high DPI.
        if (![WorkshopPointer]::PrintWindow($appWindow,$dc,3)) { throw 'Could not render the private Workshop window' }
    } finally { $g.ReleaseHdc($dc); $g.Dispose() }
    try { $bmp.Save($path,[Drawing.Imaging.ImageFormat]::Png) } finally { $bmp.Dispose() }
}
function Test-NavigationLayout {
    $scale = [WorkshopPointer]::GetDpiForWindow($appWindow) / 96.0
    function Snapshot($screen) {
        Start-Sleep -Milliseconds 140
        Save-PrivateWindow (Join-Path $root "$size-$screen.png")
    }
    function Check-Center($name,$screen) {
        $bounds = (Named $name).Current.BoundingRectangle
        $origin = [WorkshopPointer+Point]::new()
        $origin.X = [int]$bounds.X
        $origin.Y = [int]$bounds.Y
        [void][WorkshopPointer]::ScreenToClient($appWindow,[ref]$origin)
        $bmp = [Drawing.Bitmap]::new((Join-Path $root "$size-$screen.png"))
        try {
            $w = [int]$bounds.Width
            $h = [int]$bounds.Height
            $inset = [int](4*$scale)
            $bg = $bmp.GetPixel($origin.X+$inset, $origin.Y+[int]($h/2))
            $xMin=$w; $xMax=0; $yMin=$h; $yMax=0
            for ($y=[int]($h*0.18); $y -lt $h*0.82; $y++) {
                for ($x=$inset; $x -lt $w-$inset; $x++) {
                    $c = $bmp.GetPixel($origin.X+$x,$origin.Y+$y)
                    $difference = [Math]::Abs([int]$bg.R-$c.R)+[Math]::Abs([int]$bg.G-$c.G)+[Math]::Abs([int]$bg.B-$c.B)
                    if ($difference -gt 160) {
                        $xMin=[Math]::Min($xMin,$x); $xMax=[Math]::Max($xMax,$x)
                        $yMin=[Math]::Min($yMin,$y); $yMax=[Math]::Max($yMax,$y)
                    }
                }
            }
            if ($xMax -le $xMin -or $yMax -le $yMin) {throw "No visible label for $name at $size"}
            # Font side bearings and descenders can be a few pixels off the logical text box.
            $dx=($xMax+$xMin+1-$w)/(2*$scale); $dy=($yMax+$yMin+1-$h)/(2*$scale)
            if ([Math]::Abs($dx) -gt 3 -or [Math]::Abs($dy) -gt 3) {throw "$name at $size is off center by ($dx, $dy) logical pixels"}
        } finally { $bmp.Dispose() }
    }
    function Section($menu,$name,$ready) {
        if (Named $menu) { Click $menu }
        Wait-For { Named $name } "$name navigation"
        Click $name
        Wait-For { Named $ready } "$name contents"
    }
    foreach ($size in @(1050,760)) {
        [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int]($size*$scale),[int](616*$scale),0x16)
        Wait-For { Named 'Message Codex' } 'chat composer'
        Snapshot 'chat'
        Check-Center 'Explore project' 'chat'
        Click 'Add to message'
        Wait-For { Named 'Draw a sketch' } 'composer actions'
        Snapshot 'add'
        Click 'Draw a sketch'
        Wait-For { Named 'Sketch canvas' } 'inline sketch'
        Snapshot 'sketch'
        Click 'Done'
        if (Named 'Remove sketch') { Click 'Remove sketch' }
        Click 'Chat model and effort'
        Wait-For { Named 'Chat reasoning effort' } 'effort selection'
        Snapshot 'effort'
        Click 'Chat model'
        Wait-For { Named 'Select model' } 'model list'
        Snapshot 'models-list'
        Click 'Subagents'
        Wait-For { Named 'Use global defaults' } 'worker policy'
        Snapshot 'subagents'
        Click 'Done'
        Click 'Project tools'
        Wait-For { Named 'No saved assets yet' } 'empty project library'
        if ((Named 'Art brief') -or (Named 'Model brief')) { throw 'Asset creation belongs in chat' }
        Snapshot 'saved-assets'
        Section 'Tools and history' 'Decompilation' 'Choose binary…'
        Snapshot 'studio-reports'
        Check-Center 'Choose binary…' 'studio-reports'
        Click 'Run settings'
        Wait-For { Named 'Functions per run' } 'run settings'
        Snapshot 'run-settings'
        Click 'Back to reports'
        Click 'Settings'
        Section 'Settings sections' 'Providers' 'Codex'
        Click 'Codex'
        Wait-For { Named 'Decompilation summaries' } 'provider roles'
        Snapshot 'providers'
        Check-Center 'Refresh' 'providers'
        Click 'Codex'
        Section 'Settings sections' 'Plugins' 'Install skill Image to Sculpt'
        Snapshot 'plugins'
        Click 'Browse plugins'
        Wait-For { Named 'Calendar' } 'marketplace'
        Snapshot 'marketplace'
        Section 'Settings sections' 'Skills' 'Filter list'
        Snapshot 'skills'
        Section 'Settings sections' 'Tasks' 'Task defaults'
        Click 'Workers'
        Wait-For { Named 'Worker defaults' } 'worker settings'
        Snapshot 'worker-settings'
        Click 'Image generation'
        Section 'Settings sections' 'Notifications' "Notify me when I’m needed"
        Snapshot 'notifications'
        Section 'Settings sections' 'Appearance' 'New theme'
        Snapshot 'appearance'
        Check-Center 'New theme' 'appearance'
        Click 'New theme'
        Wait-For { Named 'Theme CSS editor' } 'theme editing'
        Snapshot 'theme-editor'
        Click 'Cancel'
        Section 'Settings sections' 'Providers' 'Codex'
        Click 'Close settings'
        Wait-For { Named 'Choose binary…' } 'return to the previous Studio workspace'
        Section 'Tools and history' 'Saved assets' 'Locate Blender'
        Click 'Screenshots'
        Wait-For { Named 'No screenshots yet' } 'empty capture album'
        Snapshot 'album'
        Click 'Settings'
        Click 'Close settings'
        Wait-For { Named 'Screenshot album' } 'return to the album'
        Click 'Chats'
    }
    Write-Output "Navigation and responsive layouts passed. Screenshots in $root"
}

function Test-ChatGoals {
    function Draft($value) { (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue($value) }
    function Objective($value) { (Named 'Goal objective').GetCurrentPattern($valuePattern).SetValue($value) }
    Draft '/goal'
    Click 'Send message'
    Wait-For { Named 'Goal objective' } 'bare goal command opens editor'
    if ((Named 'Start goal').Current.IsEnabled) { throw 'An empty goal can start' }
    [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
    [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
    Wait-For { !(Named 'Goal objective') } 'Escape leaves the goal editor'
    Draft '/goal Check the typed objective'
    Click 'Add to message'
    Click 'Set a goal'
    Wait-For { (Named 'Goal objective').GetCurrentPattern($valuePattern).Current.Value -eq 'Check the typed objective' } 'the menu and slash command share one objective'
    Click 'Close goal editor'
    Draft 'Keep this draft'
    Click 'Add to message'
    Click 'Set a goal'
    Wait-For { (Named 'Goal objective').GetCurrentPattern($valuePattern).Current.Value -eq 'Keep this draft' } 'start a goal from a draft'
    Objective 'Cancel this edit'
    Click 'Close goal editor'
    Wait-For { (Named 'Message Codex').GetCurrentPattern($valuePattern).Current.Value -eq 'Keep this draft' } 'cancel preserves the draft'
    $goal = 'Check the two fixture files'
    [void](New-Item -ItemType File (Join-Path $root 'goal-mode'))
    Draft "/goal $goal"
    Click 'Send message'
    Wait-For { Named 'Goal complete' } 'a progress turn continues and completes the goal'
    Click 'Goal complete'
    Wait-For { (Named 'Goal objective').GetCurrentPattern($valuePattern).Current.Value -eq $goal } 'saved objective'
    if ((Named 'Goal update').GetCurrentPattern($valuePattern).Current.Value -notlike '*Verified both fixture files*') {throw 'Missing goal evidence'}
    Save-PrivateWindow (Join-Path $root 'goal-complete.png')
    Click 'Close goal editor'
    Draft 'Unsent and unrelated'
    Click 'Goal complete'
    Objective 'Check the updated fixtures'
    [void](New-Item -ItemType File (Join-Path $root 'goal.hold'))
    Click 'Update goal'
    Wait-For { (Named 'Stop response') -and (Named 'Active goal') } 'edited goal running'
    Click 'Active goal'
    Click 'Pause goal'
    Wait-For { (Named 'Resume goal') -and !(Named 'Stop response') } 'paused goal can resume'
    Remove-Item -LiteralPath (Join-Path $root 'goal.hold')
    Save-PrivateWindow (Join-Path $root 'goal-paused.png')
    Click 'Resume goal'
    Wait-For { Named 'Goal complete' } 'resumed goal completes'
    Wait-For { (Named 'Message Codex').GetCurrentPattern($valuePattern).Current.Value -eq 'Unsent and unrelated' } 'goal management preserved unrelated draft'
    Click 'New chat'
    Wait-For { !(Named 'Goal complete') -and (Named 'Explore project') } 'fresh chat has no inherited goal'
    Click $goal
    Wait-For { Named 'Goal complete' } 'original chat keeps its goal'
    Click 'Goal complete'
    Click 'Remove goal'
    Click 'Confirm removal'
    Wait-For { !(Named 'Goal complete') -and !(Named 'Goal objective') -and (Named 'Message Codex') } 'remove only the goal'
    Click 'New chat'
    Click 'Settings'
    Click 'Notifications'
    $alerts = "Notify me when I’m needed"
    Click $alerts
    Wait-For { (Named $alerts).GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq 'On' } 'goal alerts enabled'
    Click 'Close settings'
    $hold = Join-Path $root 'notice.hold'
    Set-Content $hold 'hold'
    Draft '/goal Finish the background check'
    Click 'Send message'
    Wait-For { Named 'Stop response' } 'background goal started'
    Click 'Settings'
    Remove-Item -LiteralPath $hold
    Wait-For { Get-ChildItem -LiteralPath $env:WORKSHOP_TEST_NOTICE_DIR -Filter '*.json' | ForEach-Object {
        $notice = Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json
        if ($notice.title -eq 'Goal complete' -and $notice.detail -eq 'Finish the background check') { $true }
    } } 'background goal completion notification'
    Write-Output "Chat goal command, continuation, editing, pause, resume and navigation passed. Screenshots in $root"
}
function Test-Recommendations {
    Wait-For { (Named 'Install skill Image to Sculpt') -and (Named 'Caveman') -and (Named 'Frontend Design') } 'Codex recommendations'
    $filter = (Named 'Filter list').GetCurrentPattern($valuePattern)
    $filter.SetValue('terse')
    Wait-For { (Named 'Caveman') -and !(Named 'Image to Sculpt') } 'search recommendations by use case'
    $filter.SetValue('')
    Wait-For { Named 'Install skill Image to Sculpt' } 'restore recommended list'
    Save-PrivateWindow (Join-Path $root 'recommended.png')
    $normal = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow).Current.BoundingRectangle
    $scale = [WorkshopPointer]::GetDpiForWindow($appWindow) / 96.0
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int](800*$scale),[int](650*$scale),0x16)
    Wait-For { $button=Named 'Install skill Image to Sculpt'; $bounds=[System.Windows.Automation.AutomationElement]::FromHandle($appWindow).Current.BoundingRectangle
        $button -and (Named 'Settings sections') -and $button.Current.BoundingRectangle.Right -le $bounds.Right } 'install control inside compact settings'
    Click 'Install skill Image to Sculpt'
    Wait-For { (Named 'Installed') -and !(Named 'Install skill Image to Sculpt') } 'recommended skill installed'
    if ((Get-Content (Join-Path $env:CODEX_HOME 'skills/image-to-sculpt/SKILL.md')) -ne 'Image to Sculpt fixture') { throw 'Wrong skill source' }
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int]$normal.Width,[int]$normal.Height,0x16)
    Click 'Claude Code'
    Wait-For { Named 'Install plugin Image to Sculpt' } 'provider-specific plugin recommendation'
    $installHold = Join-Path $root 'plugin.install.hold'
    $refreshHold = Join-Path $root 'recommend.refresh.hold'
    Set-Content $installHold 'hold'
    Set-Content $refreshHold 'hold'
    $before = (Named 'Frontend Design').Current.BoundingRectangle
    Click 'Install plugin Image to Sculpt'
    Wait-For { (Named 'Installing… Image to Sculpt') -and (Named 'Frontend Design') } 'inline installation progress'
    Remove-Item -LiteralPath $installHold
    Wait-For { Test-Path (Join-Path $root 'recommend.refresh.ready') } 'install complete and provider reread pending'
    if (!(Named 'Installed') -or !(Named 'Caveman') -or $before -ne (Named 'Frontend Design').Current.BoundingRectangle) {
        throw 'Recommendation rows disappeared or shifted during install'
    }
    Remove-Item -LiteralPath $refreshHold
    Wait-For { (Named 'Installed') -and !(Named 'Install plugin Image to Sculpt') } 'Claude marketplace plugin installed'
    Click 'Installed plugins'
    Wait-For { (Named 'Image to Sculpt') -and (Named 'Uninstall') } 'recommended plugin in installed list'
    Click 'Uninstall'
    Wait-For { Named 'Confirm' } 'confirm recommended plugin removal'
    Click 'Confirm'
    Wait-For { !(Named 'Image to Sculpt') -and (Named 'Plugin uninstalled. No plugins installed.') } 'removed recommended plugin'
    Click 'Recommended'
    Wait-For { Named 'Install plugin Image to Sculpt' } 'recommendation available after uninstall'
    Click 'Codex'
    Wait-For { (Named 'Installed') -and !(Named 'Install skill Image to Sculpt') } 'Codex installation survives provider switch'
    Set-Content (Join-Path $root 'many.plugins') 'large catalog'
    Click 'Browse plugins'
    Wait-For { (Named 'Calendar') -and (Named 'Show more') } 'first marketplace page'
    $search = (Named 'Filter list').GetCurrentPattern($valuePattern)
    $search.SetValue('extra 65')
    Wait-For { (Named 'Extra 65') -and !(Named 'Show more') } 'search beyond first page'
    $search.SetValue('')
    Wait-For { Named 'Show more' } 'restored first page'
    Click 'Show more'
    Wait-For { !(Named 'Show more') } 'remaining marketplace results'
    $count = @(Get-Content (Join-Path $root 'browse.requests')).Count
    Click 'Recommended'
    Wait-For { Named 'Caveman' } 'back to recommendations'
    Set-Content (Join-Path $root 'browse.fail') 'offline'
    Click 'Browse plugins'
    Wait-For { (Named 'Calendar') -and (Named 'Show more') } 'cached marketplace'
    if (@(Get-Content (Join-Path $root 'browse.requests')).Count -ne $count) { throw 'Returning to Browse repeated the catalog request' }
    Click 'Refresh'
    Wait-For { (Named 'Calendar') -and (Named 'Codex plugin catalog: Fixture marketplace offline') } 'catalog stays visible after refresh failure'
    Remove-Item -LiteralPath (Join-Path $root 'browse.fail'),(Join-Path $root 'many.plugins')
    Click 'Refresh'
    Wait-For { (Named 'Calendar') -and !(Named 'Show more') -and !(Named 'Codex plugin catalog: Fixture marketplace offline') } 'retry updates the catalog'
    Click 'Recommended'
    Wait-For { Named 'Caveman' } 'return to recommendations'
    'Recommended catalog, search and both provider install paths passed'
}
function Test-Delivery {
    function Draft($text) { (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue($text) }
    function Queue-Draft($text) {
        Draft $text
        Click 'Send options'
        Wait-For { Named 'Send after this run' } 'send options'
        Click 'Send after this run'
        Wait-For { (Named 'Remove queued message') -and !(Named 'Send after this run') } 'queued draft'
    }
    Wait-For { (Named 'New chat').Current.IsEnabled } 'previous chat to finish'
    Click 'New chat'
    $mode = Join-Path $root 'delivery-mode'
    [void](New-Item -ItemType File $mode)
    try {
        Draft 'Wait for a correction'
        Click 'Send message'
        Wait-For { (Named 'Stop response') -and (Test-Path (Join-Path $root 'delivery.log')) } 'active lead'
        Queue-Draft 'Follow up discard'
        Click 'Remove queued message'
        Wait-For { !(Named 'Remove queued message') } 'removed queued message'
        [void](New-Item -ItemType File -Force (Join-Path $root 'record-inference'))
        $attachment = Join-Path $root 'queued-reference.txt'
        Set-Content -LiteralPath $attachment -Value 'Queued attachment payload'
        Click 'Add to message'
        Wait-For { Named 'Attach files…' } 'queued attachment picker'
        Click 'Attach files…'
        Choose-Attachment 'Attach files' $attachment
        Wait-For { Named 'Remove queued-reference.txt' } 'queued attachment in draft'
        Queue-Draft 'Follow up after stop'
        Click 'Stop response'
        Wait-For { Named 'Send next' } 'paused queue'
        Draft 'Keep my draft'
        Click 'Send next'
        Wait-For { (Named 'Codex reply') -and (Named 'New chat').Current.IsEnabled -and
            (Select-String -Quiet -LiteralPath (Join-Path $root 'delivery.log') -Pattern '^Follow up after stop$') } 'resumed queue'
        $wire = Get-ChildItem $root -Filter 'lead-*.json' | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json } |
            Where-Object { $_.input[0].text -like 'Follow up after stop*' }
        if (!(($wire.input | Where-Object type -eq 'text').text -match 'Queued attachment payload')) { throw 'Queued attachment was not delivered' }
        if ((Named 'Message Codex').GetCurrentPattern($valuePattern).Current.Value -ne 'Keep my draft') {throw 'Queue consumed an unrelated draft'}
        Click 'New chat'
        Draft 'delegate'
        Click 'Send message'
        Wait-For { (Named 'Dispatched 2 subagents') -and (Test-Path (Join-Path $root 'waiting-worker')) } 'running worker'
        Click 'Dispatched 2 subagents'
        Wait-For { Named 'Write left' } 'dispatched workers'
        Click 'Write left'
        Wait-For { Named 'Send direction' } 'worker controls'
        Click 'Send direction'
        Wait-For { Named 'Return to lead' } 'addressed composer'
        Draft 'Use the blue version'
        Click 'Send to worker'
        Wait-For { (Named 'New chat').Current.IsEnabled -and !(Named 'Return to lead') } 'directed completion'
        if (!(Select-String -Quiet -LiteralPath (Join-Path $root 'lead-feedback') -Pattern 'Use the blue version')) { throw 'Missing lead receipt' }
        if (Select-String -Quiet -LiteralPath (Join-Path $root 'delivery.log') -Pattern 'Follow up discard|Keep my draft') { throw 'Sent an unrequested message' }
        'Queued delivery, removal, pause, draft retention and direct worker steering passed'
    } finally { Remove-Item -LiteralPath $mode }
}
function Test-Attention {
    $toggle = "Notify me when I$([char]0x2019)m needed"
    Click 'Settings'
    Wait-For { (Named 'Notifications').Current.IsEnabled } 'notification settings section'
    Click 'Notifications'
    Wait-For { Named $toggle } 'notification preference'
    $pattern = [System.Windows.Automation.TogglePattern]::Pattern
    if ((Named $toggle).GetCurrentPattern($pattern).Current.ToggleState -ne 'Off') { throw 'Notifications must start disabled' }
    Click $toggle
    Wait-For { (Named $toggle).GetCurrentPattern($pattern).Current.ToggleState -eq 'On' } 'enabled notifications'
    Click 'Close settings'
    foreach ($blocked in @($false,$true)) {
        Click 'New chat'
        $hold = Join-Path $root 'notice.hold'
        [void](New-Item -ItemType File -Path $hold)
        if ($blocked) {
            Remove-Item -LiteralPath (Join-Path $root 'small-talk')
            [void](New-Item -ItemType File -Path (Join-Path $root 'blocker'))
        }
        $prompt = if ($blocked) { 'Prepare the release' } else { 'Hello from another chat' }
        (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue($prompt)
        Click 'Send message'
        Wait-For { Named 'Stop response' } 'background chat running'
        Click 'Settings'
        Wait-For { Named $toggle } 'a different screen while the run finishes'
        Remove-Item -LiteralPath $hold
        Wait-For { (Named 'New chat').Current.IsEnabled } 'background run completion'
        if (!$blocked) { Click 'Close settings' }
    }
    Wait-For { @(Get-ChildItem -LiteralPath $env:WORKSHOP_TEST_NOTICE_DIR -Filter '*.json').Count -ge 1 } 'notification delivery'
    $notices = @(Get-ChildItem -LiteralPath $env:WORKSHOP_TEST_NOTICE_DIR -Filter '*.json')
    $notice = Get-Content -Raw -LiteralPath $notices[0].FullName | ConvertFrom-Json
    if ($notices.Count -ne 1 -or $notice.title -ne 'Chat needs your attention' -or $notice.detail -notlike '*Missing release input*') { throw 'Expected only the blocked chat to notify with its missing input' }
    Click 'Close settings'
    Click 'New chat'
    Wait-For { !(Named 'Retry') -and !(Named 'Codex reply') } 'leave the blocked conversation'
    [void](New-Item -ItemType File -Path ($notices[0].FullName + '.click'))
    Wait-For { $el=Named 'Your message'; $el -and $el.GetCurrentPattern($valuePattern).Current.Value -eq 'Prepare the release' } 'notification activation restores the correct chat'
    Click 'New chat'
    [void](New-Item -ItemType File -Force (Join-Path $root 'chat-assets'))
    Copy-Item -LiteralPath $env:WORKSHOP_TEST_CAPTURE_IMAGE -Destination (Join-Path $root 'studio-image.png') -Force
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Draw the moonlit harbor')
    [void](New-Item -ItemType File -Path $hold)
    Click 'Send message'
    Wait-For { Named 'Stop response' } 'asset request in progress'
    Click 'Settings'
    Wait-For { Named $toggle } 'background chat'
    Remove-Item -LiteralPath $hold
    Wait-For { @(Get-ChildItem -LiteralPath $env:WORKSHOP_TEST_NOTICE_DIR -Filter '*.json').Count -eq 2 } 'background image notification'
    $ready = Get-ChildItem -LiteralPath $env:WORKSHOP_TEST_NOTICE_DIR -Filter '*.json' | Where-Object { $_.FullName -ne $notices[0].FullName }
    if ((Get-Content -Raw -LiteralPath $ready.FullName | ConvertFrom-Json).title -ne 'Image ready for review') { throw 'Missing image review alert' }
    [void](New-Item -ItemType File -Path ($ready.FullName + '.click'))
    Wait-For { (Named 'Save image') -and (Named 'Moonlit harbor') } 'notification opens the chat containing the image'
    'Background notifications and chat asset activation passed'
}
function Test-ChatAssets {
    Add-Type -AssemblyName System.Drawing
    $studioImage = [Drawing.Bitmap]::new(40,24)
    try {
        $art = [Drawing.Graphics]::FromImage($studioImage)
        try {
            $art.Clear([Drawing.Color]::DarkSlateGray)
            $art.FillPolygon([Drawing.Brushes]::SeaGreen, [Drawing.Point[]]@([Drawing.Point]::new(5,21),[Drawing.Point]::new(14,4),[Drawing.Point]::new(22,21)))
        } finally { $art.Dispose() }
        $studioImage.Save((Join-Path $root 'studio-image.png'),[Drawing.Imaging.ImageFormat]::Png)
    } finally { $studioImage.Dispose() }
    foreach ($flag in @('chat-assets','record-inference','studio.hold')) {
        [void](New-Item -ItemType File -Force (Join-Path $root $flag))
    }
    Click 'New chat'
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Draw a forest icon')
    Click 'Send message'
    Wait-For { (Named 'Stop response') -and (Named 'Generating an image…') } 'cancellable generation in the chat'
    Click 'Stop response'
    Wait-For { (Named 'New chat').Current.IsEnabled } 'cancelled image request'
    Remove-Item -LiteralPath (Join-Path $root 'studio.hold')
    if (Named 'Save image') { throw 'Cancelled generation delivered an image' }
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Draw a forest icon again')
    Click 'Send message'
    Wait-For { (Named 'Save image') -and (Named 'New chat').Current.IsEnabled } 'finished image in the same chat'
    $request = Get-ChildItem $root -Filter 'studio-image-*.json' | Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Raw | ConvertFrom-Json
    if ($request.model -ne 'fixture-deep' -or $request.effort -ne 'high') { throw 'Generation lost the selected model or effort' }
    Click 'Outputs'
    Wait-For { Named 'Open Forest icon' } 'generated image in Outputs'
    Click 'Open Forest icon'
    Wait-For { Named 'Full image' } 'open full-size output'
    Click 'Save image'
    $exported=Join-Path $project 'forest.png'
    Choose-Attachment 'Save asset' $exported
    Wait-For { Test-Path -LiteralPath $exported } 'PNG export completed'
    $result=[Drawing.Bitmap]::FromFile($exported)
    try { if($result.Width -ne 40 -or $result.Height -ne 24){throw 'Incorrect exported image'} } finally { $result.Dispose() }
    Click 'Annotate'
    Wait-For { Named 'Sketch canvas' } 'mark up the generated image'
    Draw-SketchLine
    Click 'Done'
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Refine the marked icon')
    Click 'Send message'
    Wait-For { (Named 'Compare') -and (Named 'New chat').Current.IsEnabled } 'marked-up revision in the chat'
    $request = Get-ChildItem $root -Filter 'studio-image-*.json' | Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Raw | ConvertFrom-Json
    if (!($request.input | Where-Object { $_.type -eq 'localImage' -and $_.path -like '*markup*' })) { throw 'The generator did not receive the marked-up pixels' }
    Click 'Outputs'
    Wait-For { Named 'Open Refined icon' } 'revised image in Outputs'
    Save-PrivateWindow (Join-Path $root 'chat-asset-outputs.png')
    Click 'Outputs'
    Click 'Project tools'
    Wait-For { (Named 'Search assets') -and (Named 'Attach to chat') } 'saved project library'
    if ((Named 'Art brief') -or (Named 'Model brief')) { throw 'Unexpected generation editor in the library' }
    (Named 'Search assets').GetCurrentPattern($valuePattern).SetValue('no matching title')
    Wait-For { Named 'No matching assets.' } 'filtered project library'
    (Named 'Search assets').GetCurrentPattern($valuePattern).SetValue('Forest icon')
    Wait-For { Named 'Attach to chat' } 'restored library asset'
    Click 'Details'
    Wait-For { Named 'Run details' } 'asset metadata'
    Click 'Preview'
    Click 'Attach to chat'
    Wait-For { Named 'Remove image.png' } 'attach an earlier asset to the chat'
    Click 'Remove image.png'
    Click 'New chat'
    Click 'Outputs'
    Wait-For { Named 'Images, 3D models and screenshots from this chat appear here.' } 'outputs scoped to the new chat'
    Click 'Outputs'
    $steamBlender = Join-Path ${env:ProgramFiles(x86)} 'Steam/steamapps/common/Blender/blender.exe'
    if ($env:WORKSHOP_BLENDER -or (Test-Path -LiteralPath $steamBlender) -or (Get-Command blender -ErrorAction SilentlyContinue)) {
        (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Build a lantern')
        Click 'Send message'
        Wait-For { (Named 'Save model') -and (Named 'New chat').Current.IsEnabled } 'Blender render and GLB in the chat' 100
        Click 'Outputs'
        Wait-For { Named 'Open Copper lantern' } 'completed model in Outputs'
        Click 'Open Copper lantern'
        Wait-For { (Named 'Model inspector') -and (Named 'Wireframe').Current.IsEnabled } 'interactive model from Outputs' 15
        Click 'Wireframe'
        Click 'Close model inspector'
        Wait-For { Named 'Save model' } 'return to the conversation'
        Click 'Save model'
        $glb = Join-Path $project 'lantern.glb'
        Choose-Attachment 'Save asset' $glb
        Wait-For { Test-Path -LiteralPath $glb } 'GLB export completed'
        if ([Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($glb),0,4) -ne 'glTF') { throw 'Chat export is not a binary glTF model' }
        Save-PrivateWindow (Join-Path $root 'chat-model.png')
    }
    Remove-Item -LiteralPath (Join-Path $root 'record-inference')
    'Chat image generation, cancellation, marked-up revision, Outputs and the shared asset library passed'
}
function Test-Sketch {
    Wait-For { (Named 'New chat').Current.IsEnabled } 'available project'
    Click 'New chat'
    Wait-For { (Named 'Message Codex') -and !(Named 'Send message').Current.IsEnabled } 'empty sketch draft'
    Open-SketchPad
    Draw-SketchLine
    Wait-For { (Named 'Send message').Current.IsEnabled } 'sketch-only send'
    Click 'Undo stroke'
    Wait-For { !(Named 'Send message').Current.IsEnabled } 'undo last stroke'
    Click 'Done'
    Wait-For { !(Named 'Sketch canvas') -and !(Named 'Edit sketch') } 'close empty sketch'
    $caption = 'Move this control along the blue line'
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue($caption)
    Open-SketchPad
    Click 'Blue pen'
    Draw-SketchLine
    Click 'Done'
    Wait-For { Named 'Edit sketch' } 'collapsed sketch'
    Click 'Edit sketch'
    Wait-For { (Named 'Undo stroke').Current.IsEnabled } 'editable saved ink'
    Click 'Done'
    [void](New-Item -ItemType File -Force (Join-Path $root 'record-inference'))
    [void](New-Item -ItemType File -Force (Join-Path $root 'small-talk'))
    Click 'Send message'
    Wait-For { (Named 'Enlarge sketch') -and (Named 'New chat').Current.IsEnabled } 'sent drawing in transcript'
    $wire = Get-ChildItem $root -Filter 'lead-*.json' | ForEach-Object {
        Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json
    } | Where-Object { $_.input[0].text -eq $caption } | Select-Object -First 1
    $images = @($wire.input | Where-Object type -eq 'localImage')
    if ($images.Count -ne 1) { throw 'Expected the caption and sketch in the same provider turn' }
    $bitmap = [Drawing.Bitmap]::new($images[0].path)
    try {
        if ($bitmap.Width -ne 960 -or $bitmap.Height -ne 480) { throw 'Unexpected sketch dimensions' }
        foreach ($fraction in @(0.27,0.5,0.73)) {
            $x = [int]($bitmap.Width * $fraction)
            $y = [int]($bitmap.Height * (0.75 - 0.5 * $fraction))
            $blueInk = $false
            foreach ($dy in -5..5) {
                $pixel = $bitmap.GetPixel($x, $y + $dy)
                if ($pixel.B -gt $pixel.R + 50) { $blueInk = $true }
            }
            if (!$blueInk) { throw "Missing blue ink near ($x,$y) in provider image" }
        }
    } finally { $bitmap.Dispose() }
    Click 'Enlarge sketch'
    Wait-For { Named 'Reduce sketch' } 'expanded sent drawing'
    Click 'Reduce sketch'
    Open-SketchPad
    Draw-SketchLine
    Click 'Done'
    Wait-For { Named 'Remove sketch' } 'removable sketch'
    Click 'Remove sketch'
    Wait-For { !(Named 'Send message').Current.IsEnabled } 'removed sketch'
    Open-SketchPad
    Draw-SketchLine
    Click 'Send message'
    Wait-For { (Get-ChildItem $root -Filter 'lead-*.json' | ForEach-Object {
        $request = Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json
        if ($request.input[0].text -eq 'Sketch' -and ($request.input | Where-Object type -eq 'localImage')) { $request }
    }) } 'sketch-only follow-up delivered'
    Wait-For { (Named 'New chat').Current.IsEnabled -and !(Named 'Stop response') } 'finished sketch follow-up'
    Click 'New chat'
    Wait-For { Named 'New conversation' } 'fresh chat before reopening the sketch history'
    Click $caption
    Wait-For { Named 'Enlarge sketch' } 'reopened drawing'
    # The reopened chat starts at the latest reply. Bring the earlier sketch back into view.
    $drawing = (Named 'Enlarge sketch').Current.BoundingRectangle
    $mouse = [WorkshopPointer+Point]::new()
    $mouse.X = [int]($drawing.X + $drawing.Width / 2)
    $mouse.Y = [int]($drawing.Y + $drawing.Height / 2)
    $screenPos = [IntPtr](($mouse.Y -shl 16) -bor ($mouse.X -band 65535))
    [void][WorkshopPointer]::ScreenToClient($appWindow, [ref]$mouse)
    $clientPos = [IntPtr](($mouse.Y -shl 16) -bor ($mouse.X -band 65535))
    [void][WorkshopPointer]::PostMessage($appWindow,0x200,0,$clientPos)
    [void][WorkshopPointer]::PostMessage($appWindow,0x20a,(240 -shl 16),$screenPos)
    Wait-For {
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
        $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'Enlarge sketch')
        $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition).Count -eq 2
    } 'both drawings restored with their messages'
    Wait-For { (Named 'New chat').Current.IsEnabled } 'start image feedback test'
    Click 'New chat'
    $reference = Join-Path $root 'chat-image.png'
    $portrait = [Drawing.Bitmap]::FromFile($env:WORKSHOP_TEST_CAPTURE_IMAGE)
    try {
        $portrait.RotateFlip([Drawing.RotateFlipType]::Rotate90FlipNone)
        $portrait.Save($reference,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $portrait.Dispose() }
    $originalHash = (Get-FileHash -LiteralPath $reference).Hash
    Set-Content (Join-Path $root 'image.hold') 'generating'
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Show the reference drawing')
    Click 'Send message'
    Wait-For { (Named 'Generating an image…') -and (Named 'Stop response') } 'native image generation status'
    Remove-Item -LiteralPath (Join-Path $root 'image.hold')
    Wait-For { (Named 'Enlarge image') -and (Named 'New chat').Current.IsEnabled } 'agent image reply'
    $refinement = 'Move the highlighted area to the right'
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue($refinement)
    Click 'Annotate'
    Wait-For { Named 'White pen' } 'image markup toolbar'
    $paper = (Named 'Sketch canvas').Current.BoundingRectangle
    if ([Math]::Abs(($paper.Width / $paper.Height) - 140/200) -gt 0.03) { throw 'Portrait image stretched on the canvas' }
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
    $priorSize = $window.Current.BoundingRectangle
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,800,650,0x16)
    Wait-For { $p=(Named 'Sketch canvas').Current.BoundingRectangle; $p.Height -gt 0 -and $p.Bottom -le (Named 'Message Codex').Current.BoundingRectangle.Top } 'narrow image pad and caption'
    Click 'Red pen'
    Draw-SketchLine
    Click 'Undo stroke'
    Wait-For { !(Named 'Undo stroke').Current.IsEnabled -and (Named 'Send message').Current.IsEnabled } 'undo keeps the source image'
    Draw-SketchLine
    Click 'Send message'
    $received = {
        Get-ChildItem $root -Filter 'lead-*.json' | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json } |
            Where-Object { $_.input[0].text -eq $refinement } | Select-Object -First 1
    }
    Wait-For { & $received } 'refinement request sent with annotations'
    $marked = @((& $received).input | Where-Object type -eq 'localImage')
    if ($marked.Count -ne 1 -or $marked[0].path -eq $reference -or (Get-FileHash -LiteralPath $reference).Hash -ne $originalHash) { throw 'Expected one independent marked-up copy' }
    $before = [Drawing.Bitmap]::new($reference)
    $after = [Drawing.Bitmap]::new($marked[0].path)
    try {
        if ($after.Size -ne $before.Size) { throw 'Annotation changed the source dimensions' }
        foreach ($pt in @([Drawing.Point]::new(1,1),[Drawing.Point]::new(100,45))) {
            if ($before.GetPixel($pt.X,$pt.Y).ToArgb() -ne $after.GetPixel($pt.X,$pt.Y).ToArgb()) { throw 'An unmarked source pixel changed' }
        }
        $red = $false
        foreach ($y in 95..105) {
            $pixel = $after.GetPixel(70,$y)
            if ($pixel.R -gt $pixel.B + 50) { $red = $true }
        }
        if (!$red) { throw 'Missing red mark on the image delivered for refinement' }
    } finally { $before.Dispose(); $after.Dispose() }
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int]$priorSize.Width,[int]$priorSize.Height,0x16)
    Wait-For {
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
        $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'Codex reply')
        $captions = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition) |
            ForEach-Object { $_.GetCurrentPattern($valuePattern).Current.Value }
        (Named 'Enlarge image') -and $captions -contains 'Revised layout'
    } 'revised project image returned to the chat'
    Click 'Compare'
    $divider = 'Image comparison divider'
    Wait-For { Named $divider } 'image comparison'
    $range = [System.Windows.Automation.RangeValuePattern]::Pattern
    Click $divider 0.25
    Wait-For { $v=(Named $divider).GetCurrentPattern($range).Current.Value; $v -gt 23 -and $v -lt 27 } 'pointer adjusts reveal'
    [void][WorkshopPointer]::PostMessage($appWindow,0x100,35,1)
    [void][WorkshopPointer]::PostMessage($appWindow,0x101,35,1)
    Wait-For { (Named $divider).GetCurrentPattern($range).Current.Value -eq 100 } 'End reveals the original'
    $clean = Join-Path $root 'comparison-clean.png'
    Save-PrivateWindow $clean
    Click 'Show markup'
    Wait-For { (Named 'Show markup').GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq 'On' } 'visible markup'
    Click 'Settings'
    Wait-For { Named 'Close settings' } 'settings from image comparison'
    Click 'Close settings'
    Wait-For { (Named $divider).GetCurrentPattern($range).Current.Value -eq 100 -and
        (Named 'Show markup').GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq 'On' } 'return to the same comparison and reveal position'
    $ink = Join-Path $root 'comparison-marked.png'
    Save-PrivateWindow $ink
    $cleanPixels = [Drawing.Bitmap]::new($clean)
    $inkPixels = [Drawing.Bitmap]::new($ink)
    try {
        $x = [int]($inkPixels.Width * 0.55)
        $changes = 0
        for ($y = [int]($inkPixels.Height/3); $y -lt $inkPixels.Height*0.75; $y++) {
            $color = $inkPixels.GetPixel($x,$y)
            if ($color.R -gt $color.B + 25 -and $color.ToArgb() -ne $cleanPixels.GetPixel($x,$y).ToArgb()) { $changes++ }
        }
        if ($changes -lt 2) { throw 'Markup toggle did not reveal the red annotation on the source' }
    } finally { $cleanPixels.Dispose(); $inkPixels.Dispose() }
    (Named $divider).GetCurrentPattern($range).SetValue(0)
    Wait-For { (Named $divider).GetCurrentPattern($range).Current.Value -eq 0 } 'full revision'
    $revision = Join-Path $root 'comparison-revision.png'
    Save-PrivateWindow $revision
    (Named $divider).GetCurrentPattern($range).SetValue(50)
    Wait-For { (Named $divider).GetCurrentPattern($range).Current.Value -eq 50 } 'accessible divider'
    Save-PrivateWindow (Join-Path $root 'comparison.png')
    $half = [Drawing.Bitmap]::new((Join-Path $root 'comparison.png'))
    $left = [Drawing.Bitmap]::new($ink)
    $right = [Drawing.Bitmap]::new($revision)
    try {
        foreach ($fraction in @(0.46,0.65)) {
            $side = if ($fraction -lt 0.5) { $left } else { $right }
            $x = [int]($half.Width * $fraction)
            for ($y = [int]($half.Height/3); $y -lt $half.Height*0.75; $y+=3) {
                if ($half.GetPixel($x,$y).ToArgb() -ne $side.GetPixel($x,$y).ToArgb()) { throw 'Moving the divider shifted or exposed the wrong source image' }
            }
        }
    } finally { $half.Dispose(); $left.Dispose(); $right.Dispose() }
    Click 'Annotate revision'
    Wait-For { Named 'Sketch canvas' } 'refine revision'
    $revisedPad = (Named 'Sketch canvas').Current.BoundingRectangle
    if ([Math]::Abs($revisedPad.Width / $revisedPad.Height - 200/140) -gt 0.03) { throw 'Wrong image selected for revision markup' }
    Click 'Done'
    Click 'Remove sketch'
    Click 'Compare'
    Wait-For { Named 'Annotate original' } 'comparison reopened'
    Click 'Annotate original'
    Wait-For { Named 'Sketch canvas' } 'refine original'
    $originalPad = (Named 'Sketch canvas').Current.BoundingRectangle
    if ([Math]::Abs($originalPad.Width / $originalPad.Height - 140/200) -gt 0.03) { throw 'Wrong image selected for original markup' }
    'Sketch drawing, annotated image refinement and comparison passed'
    Test-Attention
}
# A GUI-subsystem executable must accept automation arguments and finish with redirected output.
$command = [Diagnostics.ProcessStartInfo]::new($fixtureApp, "--headless --project `"$project`" open")
$command.UseShellExecute = $false
$command.CreateNoWindow = $true
$command.RedirectStandardOutput = $true
$command.RedirectStandardError = $true
$smoke = [Diagnostics.Process]::Start($command)
try {
    if (!$smoke.WaitForExit(10000)) { $smoke.Kill(); throw 'Headless command did not exit' }
    if ($smoke.ExitCode -ne 0 -or $smoke.StandardOutput.ReadToEnd() -notlike "*Opened $project*") {
        throw "Headless project open failed. $($smoke.StandardError.ReadToEnd())"
    }
} finally { $smoke.Dispose() }
# Exercise the real region picker against a known image without sampling the workstation.
Add-Type -AssemblyName System.Drawing
$env:WORKSHOP_TEST_CAPTURE_IMAGE = Join-Path $root 'capture-fixture.png'
$surface = [Drawing.Bitmap]::new(200, 140)
try {
    $graphics = [Drawing.Graphics]::FromImage($surface)
    try { $graphics.FillRectangle([Drawing.Brushes]::SteelBlue, 30, 25, 60, 40) } finally { $graphics.Dispose() }
    $surface.Save($env:WORKSHOP_TEST_CAPTURE_IMAGE, [Drawing.Imaging.ImageFormat]::Png)
} finally { $surface.Dispose() }
$automation = [Diagnostics.ProcessStartInfo]::new($fixtureApp, "--project `"$project`" list")
$automation.UseShellExecute = $false
$automation.CreateNoWindow = $true
$automation.RedirectStandardError = $true
$automation.EnvironmentVariables['WORKSHOP_HEADLESS'] = '1'
$headless = [Diagnostics.Process]::Start($automation)
try {
    if (!$headless.WaitForExit(10000)) { $headless.Kill(); throw 'Automation mode opened the desktop app' }
    if ($headless.ExitCode -ne 0) { throw $headless.StandardError.ReadToEnd() }
} finally { $headless.Dispose() }
if ($UsageOnly) { [void](New-Item -ItemType File -Path (Join-Path $root 'catalog.hold')) }
$proc = [TestDesktop]::StartApp($fixtureApp, $project)
try {
    $appWindow = [TestDesktop]::WaitForWindow($proc)
    if ($UsageOnly) {
        Wait-For { (Named '100%') -and (Named '88%') -and (Named '31%') } 'empty-chat context and account usage before model discovery finishes'
        Wait-For { (Named 'Chat model and effort').Current.IsEnabled } 'other providers ready while Codex discovery is blocked'
        Click 'Chat model and effort'
        Click 'Chat model'
        Wait-For { Named 'Claude Sonnet 5.5' } 'Claude selectable independently of Codex'
        [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
        [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
        Remove-Item -LiteralPath (Join-Path $root 'catalog.hold')
    }
    Wait-For { $el = Named 'Chat model and effort'; $el -and $el.Current.IsEnabled } 'model discovery'
    $rail = (Named 'Project tools').Current.BoundingRectangle
    $search = (Named 'Search chats').Current.BoundingRectangle
    if ($rail.Right -gt $search.Left) { throw 'Navigation rail overlaps the chat sidebar' }
    $valuePattern = [System.Windows.Automation.ValuePattern]::Pattern
    if ($NavigationOnly) { Test-NavigationLayout; return }
    if ($StudioOnly) { Test-ChatAssets; return }
    if ($GoalOnly) { Test-ChatGoals; return }
    if ($ProviderTasksOnly) {
        Test-ProviderTasks
        Write-Output "Provider task settings passed. Screenshot in $root"
        return
    }
    if ($UsageOnly) {
        $scale = [WorkshopPointer]::GetDpiForWindow($appWindow) / 96.0
        foreach ($width in @(1060,800)) {
            [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int]($width*$scale),[int](680*$scale),0x16)
            Wait-For { $l = Named '7d'; $v = Named '31%'; $l -and $v -and $v.Current.BoundingRectangle.X -ge $l.Current.BoundingRectangle.Right } 'account readings in the header'
            foreach ($pair in @(@('Context','100%'),@('5h','88%'),@('7d','31%'))) {
                $label = (Named $pair[0]).Current.BoundingRectangle
                $value = (Named $pair[1]).Current.BoundingRectangle
                $gap = ($value.Left - $label.Right) / $scale
                if ($gap -lt -1 -or $gap -gt 7 -or [math]::Abs(($value.Top+$value.Height/2) - ($label.Top+$label.Height/2)) -gt $scale) {
                    throw "Misaligned $($pair[0]) and $($pair[1]) at width $width"
                }
            }
            Click 'Context and usage'
            Wait-For { (Named '0 tokens used') -and (Named '5-hour limit') -and (Named 'Weekly limit') } 'new-chat quota details'
            Save-PrivateWindow (Join-Path $root "usage-$width.png")
            [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
            [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
            Wait-For { !(Named 'Context window') } 'closed usage details'
        }
        Click 'Chat model and effort'
        Click 'Chat model'
        Click 'Claude Sonnet 5.5'
        Wait-For { (Named '100%') -and (Named '75%') -and (Named '52%') -and !(Named '88%') } 'Claude quota before sending a message'
        [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
        [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
        (Named 'Message Claude').GetCurrentPattern($valuePattern).SetValue('Usage fixture greeting')
        Set-Content (Join-Path $root 'thinking.hold') 'thinking'
        Click 'Send message'
        Wait-For { (Named 'Thinking…') -and (Named 'Stop response') } 'live Claude thinking status'
        Save-PrivateWindow (Join-Path $root 'thinking.png')
        Remove-Item -LiteralPath (Join-Path $root 'thinking.hold')
        Wait-For { (Named 'New chat').Current.IsEnabled -and (Named '73%') } 'context after a Claude turn'
        Click 'New chat'
        Wait-For { (Named '100%') -and (Named '75%') -and (Named '52%') } 'fresh context with the same account usage'
        [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int](1060*$scale),[int](700*$scale),0x16)
        foreach ($provider in @('Antigravity','Gemini')) {
            Click 'Chat model and effort'
            Click 'Chat model'
            [void][WorkshopPointer]::PostMessage($appWindow,0x100,0x23,1)
            [void][WorkshopPointer]::PostMessage($appWindow,0x101,0x23,1)
            Wait-For { Named 'Gemini Fixture' } 'Google models visible'
            Click (@{Antigravity='Gemini Test (Antigravity)';Gemini='Gemini Fixture'}[$provider])
            Wait-For { Named "Message $provider" } "selected $provider"
            [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
            [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
            (Named "Message $provider").GetCurrentPattern($valuePattern).SetValue("Hello from $provider")
            Set-Content (Join-Path $root 'thinking.hold') 'thinking'
            Click 'Send message'
            Wait-For { Named 'Thinking…' } "$provider live thinking status"
            Remove-Item -LiteralPath (Join-Path $root 'thinking.hold')
            Wait-For { (Named "$provider reply") -and (Named 'New chat').Current.IsEnabled -and !(Named 'Thinking…') } "$provider completed response"
            Click 'New chat'
        }
        Click 'Usage fixture greeting'
        Wait-For { Named 'Message Claude' } 'back to the original Claude chat'
        (Named 'Message Claude').GetCurrentPattern($valuePattern).SetValue('Usage fixture greeting again')
        Click 'Send message'
        Wait-For { @(Get-Content (Join-Path $root 'claude-greetings')).Count -eq 2 -and (Named 'New chat').Current.IsEnabled } 'follow-up after switching providers'
        if (@(Get-Content (Join-Path $root 'claude-greetings') | Select-Object -Unique).Count -ne 1) {throw 'Returning to a recent chat restarted its provider'}
        "Header alignment, provider readiness, activity and warm follow-ups passed. Screenshots in $root"
        return
    }
    if ($DeliveryOnly) { Test-Delivery; return }
    if ($TranscriptOnly) {
        Save-PrivateWindow (Join-Path $root 'welcome.png')
        [void](New-Item -ItemType File -Path (Join-Path $root 'formatted-reply'))
        (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('Show me how the project title works.')
        Click 'Send message'
        Wait-For { (Named 'Copy code') -and (Named 'New chat').Current.IsEnabled } 'formatted agent response'
        Save-PrivateWindow (Join-Path $root 'transcript.png')
        $scale = [WorkshopPointer]::GetDpiForWindow($appWindow) / 96.0
        [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int](800*$scale),[int](650*$scale),0x16)
        Wait-For { (Named 'Show sidebar') -and (Named 'Copy code') } 'compact code block'
        Save-PrivateWindow (Join-Path $root 'transcript-compact.png')
        Click 'Settings'
        Wait-For { Named 'Settings sections' } 'compact settings'
        Click 'Settings sections'
        Wait-For { Named 'Appearance' } 'settings navigation'
        Click 'Appearance'
        Wait-For { Named 'Solarized Light' } 'theme choices'
        Click 'Solarized Light'
        Click 'Close settings'
        Wait-For { Named 'Copy code' } 'open transcript follows theme'
        Save-PrivateWindow (Join-Path $root 'transcript-light.png')
        $render = [Drawing.Bitmap]::new((Join-Path $root 'transcript-light.png'))
        try {
            $panelColor = [Drawing.ColorTranslator]::FromHtml('#e7dfcb').ToArgb()
            $panelFound = $false
            for ($y = [int]($render.Height/4); $y -lt $render.Height*0.8; $y += 8) {
                if ($render.GetPixel([int]($render.Width*0.6),$y).ToArgb() -eq $panelColor) { $panelFound = $true; break }
            }
            if (!$panelFound) { throw 'The open code panel did not switch to Solarized Light' }
        } finally { $render.Dispose() }
        "Formatted transcript and responsive theme checks passed. Screenshots in $root"
        return
    }
    if ($RecommendedOnly) {
        Click 'Settings'
        Wait-For { Named 'Plugins' } 'settings navigation'
        Click 'Plugins'
        Test-Recommendations
        return
    }
    if ($SketchOnly) { Test-Sketch; return }
    Click 'Explore project'
    Wait-For { (Named 'Message Codex').GetCurrentPattern($valuePattern).Current.Value -like 'Give me a tour*' } 'editable starter prompt'
    Click 'Hide sidebar'
    Wait-For { (Named 'Show sidebar') -and !(Named 'Search chats') } 'hidden chat sidebar'
    Click 'Show sidebar'
    Wait-For { Named 'Search chats' } 'restored chat sidebar'
    $normal = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow).Current.BoundingRectangle
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,800,650,0x16)
    Wait-For { Named 'Show sidebar' } 'compact layout'
    Click 'Show sidebar'
    Wait-For { Named 'Search chats' } 'chat drawer'
    [void][WorkshopPointer]::PostMessage($appWindow,0x100,27,1)
    [void][WorkshopPointer]::PostMessage($appWindow,0x101,27,1)
    Wait-For { (Named 'Show sidebar') -and !(Named 'Search chats') } 'Escape closes chat drawer'
    [void][WorkshopPointer]::SetWindowPos($appWindow,0,0,0,[int]$normal.Width,[int]$normal.Height,0x16)
    Wait-For { Named 'Search chats' } 'full-width chat sidebar'
    (Named 'Search chats').GetCurrentPattern($valuePattern).SetValue('missing chat')
    Wait-For { Named 'No matching chats' } 'empty chat search'
    (Named 'Search chats').GetCurrentPattern($valuePattern).SetValue('')
    (Named 'Message Codex').GetCurrentPattern($valuePattern).SetValue('')
    Click 'Message Codex'
    Click 'Chat model and effort'
    Wait-For { Named 'Chat reasoning effort' } 'first-click effort picker'
    Click 'Chat model'
    Wait-For { Named 'Select model' } 'model menu'
    Wait-For { (Named 'Claude Default · Sonnet 5.5') -and (Named 'Claude Sonnet 5.5') -and (Named 'Claude Haiku 4.5') } 'resolved Claude models'
    Save-PrivateWindow (Join-Path $root 'claude-models.png')
    [void][WorkshopPointer]::PostMessage($appWindow,0x100,0x23,1) # End scrolls the menu to the Google providers.
    [void][WorkshopPointer]::PostMessage($appWindow,0x101,0x23,1)
    Wait-For { Named 'Gemini Fixture' } 'last model visible'
    Click 'Gemini Test (Antigravity)'
    Wait-For { (Named 'Message Antigravity') -and (Named 'Chat reasoning effort').GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Maximum -eq 1 } 'Antigravity model and effort'
    Wait-For { (Named '100%') -and (Named '67%') -and (Named '42%') -and !(Named '88%') } 'Google quota on a new chat'
    Click 'Chat reasoning effort' 0.05
    Wait-For { (Named 'Chat reasoning effort').GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value -eq 0 } 'Antigravity low effort'
    Click 'Chat model'
    Click 'Claude Sonnet 5.5'
    Wait-For { Named 'Message Claude' } 'Claude composer'
    Click 'Chat model'
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
    Test-ProviderTasks
    Click 'Settings'
    Wait-For { Named 'Sign in to Codex' } 'provider accounts'
    Click 'Appearance'
    Wait-For { (Named 'Workshop Dark') -and (Named 'Catppuccin Mocha') } 'appearance presets'
    Click 'Solarized Light'
    Click 'New theme'
    Wait-For { Named 'Theme CSS editor' } 'theme editor'
    $source = (Named 'Theme CSS editor').GetCurrentPattern($valuePattern)
    if ($source.Current.Value -notlike '*--canvas: #fdf6e3*') { throw 'New theme did not start from selected preset' }
    $file = Join-Path $root 'Sea glass.css'
    Set-Content -LiteralPath $file -Encoding ascii -Value ':root { --accent: #9ddfc7; }'
    Click 'Load file…'
    Choose-Attachment 'Load theme CSS' $file
    Wait-For { (Named 'Theme name').GetCurrentPattern($valuePattern).Current.Value -eq 'Sea glass' } 'loaded theme name'
    $source.SetValue('--unknown: #123')
    Wait-For { !(Named 'Save theme').Current.IsEnabled } 'invalid theme blocked'
    $source.SetValue(':root { --accent: #9ddfc7; --canvas: #19232a; }')
    Wait-For { (Named 'Save theme').Current.IsEnabled } 'valid theme preview'
    Click 'Save theme'
    Wait-For { (Named 'Sea glass') -and (Named 'Edit theme') } 'saved custom theme'
    Click 'Edit theme'
    (Named 'Theme name').GetCurrentPattern($valuePattern).SetValue('Sea foam')
    Click 'Cancel'
    Wait-For { (Named 'Sea glass') -and !(Named 'Sea foam') } 'cancel leaves theme intact'
    Click 'Edit theme'
    Click 'Remove theme'
    Click 'Confirm removal'
    Wait-For { (Named 'Workshop Dark') -and !(Named 'Sea glass') -and !(Named 'Edit theme') } 'removed theme and restored default'
    Click 'Providers'
    Wait-For { Named 'Sign in to Codex' } 'back to provider accounts'
    Wait-For { Named 'Gemini CLI' } 'Gemini provider row'
    Click 'Trust project'
    Wait-For { Named 'Reset trust' } 'Gemini project tools enabled'
    Click 'Reset trust'
    Wait-For { Named 'Trust project' } 'Gemini project trust reset'
    Click 'Sign in to Codex'
    Wait-For { (Named 'Sign-in finished.') -and (Test-Path (Join-Path $root 'codex.session')) -and (Named 'Sign out of Codex') } 'Codex provider login result'
    Click 'Sign out of Codex'
    Wait-For { Named 'Confirm' } 'sign-out confirmation'
    Click 'Confirm'
    Wait-For { (Named 'Signed out.') -and !(Test-Path (Join-Path $root 'codex.session')) } 'provider logout result'
    Click 'Plugins'
    Test-Recommendations
    Click 'Installed plugins'
    Wait-For { Named 'No plugins installed.' } 'installed plugins'
    Click 'Browse plugins'
    Wait-For { (Named 'Calendar') -and !(Named 'Sample plugin') } 'official catalog'
    (Named 'Filter list').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('events')
    Wait-For { Named 'Calendar' } 'search by description'
    (Named 'Filter list').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('unknown-extension')
    Wait-For { Named 'No matches' } 'empty official search'
    (Named 'Filter list').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('')
    Click 'Official marketplaces'
    Wait-For { Named 'Sample plugin' } 'marketplace catalog'
    if (Named 'Plugin source') { throw 'Manual install form should start collapsed' }
    Click 'Install from source'
    Wait-For { Named 'Plugin source' } 'manual plugin install'
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
    Click 'Install from source'
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
    Click 'Tasks'
    Wait-For { Named 'Task defaults' } 'per-task defaults'
    Click 'fixture-deep'
    Click 'Decompilation'
    Wait-For { (Named 'Functions per run') -and (Named 'Generate an AI summary') } 'local analysis controls'
    Click 'Generate an AI summary'
    Wait-For { Named 'Maximum summary effort' } 'independent summary policy'
    Click 'Model design'
    Wait-For { (Named 'Maximum task effort') -and !(Named 'Generate an AI summary') } 'model design policy'
    Click 'Close settings'
    Wait-For { Named 'Message Codex' } 'return to composer'
    Test-ChatAssets
    Click 'New chat'
    function ProviderLogs {
        (Get-ChildItem -LiteralPath $env:WORKSHOP_HOME -Recurse -Filter 'events.jsonl' | Sort-Object FullName |
            ForEach-Object { "$($_.FullName) $($_.Length)" }) -join [Environment]::NewLine
    }
    $beforeScreenshot = ProviderLogs
    Click 'Outputs'
    Wait-For { Named 'Images, 3D models and screenshots from this chat appear here.' } 'empty chat outputs'
    Click 'Outputs'
    Wait-For { !(Named 'View all screenshots') } 'outputs dismissed'
    (Named 'Message Codex').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('/screenshot')
    Click 'Send message'
    function CaptureWindow {
        $helper = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($proc.Id) AND Name='powershell.exe'" | Select-Object -First 1
        if ($helper) {return [WorkshopPointer]::Dialog($helper.ProcessId,'Select a screenshot area')}
        return [IntPtr]::Zero
    }
    Wait-For { [WorkshopPointer]::IsWindowVisible((CaptureWindow)) } 'screen region picker'
    $overlay = CaptureWindow
    $down = (25 -shl 16) -bor 30
    $up = (65 -shl 16) -bor 90
    [void][WorkshopPointer]::PostMessage($overlay,0x201,1,$down)
    [void][WorkshopPointer]::PostMessage($overlay,0x200,1,$up)
    [void][WorkshopPointer]::PostMessage($overlay,0x202,0,$up)
    Wait-For { Named 'Sent screenshot' } 'local chat receipt'
    $saved = Get-ChildItem (Join-Path $env:WORKSHOP_HOME 'screenshots') -Recurse -Filter capture.png
    $bitmap = [Drawing.Bitmap]::FromFile($saved.FullName)
    try { if ($bitmap.Width -ne 60 -or $bitmap.Height -ne 40 -or $bitmap.GetPixel(0,0).ToArgb() -ne [Drawing.Color]::SteelBlue.ToArgb() -or $bitmap.GetPixel(59,39).ToArgb() -ne [Drawing.Color]::SteelBlue.ToArgb()) {throw 'Incorrect captured region'} } finally {$bitmap.Dispose()}
    Wait-For { Named 'Open screenshot 1' } 'new screenshot in the top-right panel'
    $header = (Named 'Outputs').Current.BoundingRectangle
    $thumbnail = (Named 'Open screenshot 1').Current.BoundingRectangle
    if ($thumbnail.Top -lt $header.Bottom -or [Math]::Abs($header.Right - $thumbnail.Right) -gt 40) {throw 'Outputs did not open under the top-right button'}
    Click 'Open screenshot 1'
    Wait-For { Named 'Back' } 'full-size image from Outputs'
    Click 'Back'
    Wait-For { Named 'Open screenshot 1' } 'return to Outputs'
    Click 'View all screenshots'
    Wait-For { Named 'project' } 'album opened'
    $albumWindow = [System.Windows.Automation.AutomationElement]::FromHandle($appWindow)
    Wait-For { $albumWindow.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.Name -like 'View screenshot *'} } 'saved album image'
    Click 'Screenshots'
    Wait-For { Named 'Sent screenshot' } 'return from album to chat'
    Click 'Sent screenshot'
    Wait-For { Named 'Back' } 'full-size screenshot'
    Click 'Annotate'
    Wait-For { (Named 'Sketch canvas') -and (Named 'White pen') } 'screenshot open for feedback'
    Click 'Done'
    Wait-For { Named 'Remove sketch' } 'screenshot markup draft'
    Click 'Remove sketch'
    Wait-For { !(Named 'Remove sketch') } 'discard screenshot markup'
    Wait-For { Named 'Outputs' } 'return from screenshot to chat'
    Click 'Outputs'
    Wait-For { Named 'Open screenshot 1' } 'reopened chat outputs'
    Click 'Outputs'
    Wait-For { !(Named 'View all screenshots') } 'closed chat outputs'
    (Named 'Message Codex').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('Unsent draft')
    Click 'Add to message'
    Wait-For { Named 'Take screenshot' } 'screenshot menu action'
    Click 'Take screenshot'
    Wait-For { [WorkshopPointer]::IsWindowVisible((CaptureWindow)) } 'second region picker'
    [void][WorkshopPointer]::PostMessage((CaptureWindow),0x100,27,1)
    Wait-For { (Named 'Send message').Current.IsEnabled } 'cancelled capture'
    if (Named 'View all screenshots') {throw 'Cancelled capture opened Outputs'}
    if (@(Get-ChildItem (Join-Path $env:WORKSHOP_HOME 'screenshots') -Recurse -Filter capture.png).Count -ne 1 -or
        (Named 'Message Codex').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -ne 'Unsent draft') {throw 'Cancel changed screenshot history or draft'}
    (Named 'Message Codex').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('')
    if ((ProviderLogs) -ne $beforeScreenshot) {throw 'Screenshot started a provider turn'}
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
    $earlierLeads = @(Get-ChildItem -LiteralPath $root -Filter 'lead-*.json' | Select-Object -ExpandProperty Name)
    [void](New-Item -ItemType File (Join-Path $root 'record-inference'))
    Click 'Send message'
    function AttachmentLead { Get-ChildItem -LiteralPath $root -Filter 'lead-*.json' | Where-Object { $_.Name -notin $earlierLeads } | Select-Object -First 1 }
    Wait-For { AttachmentLead } 'attachment-only turn'
    $wire = Get-Content -LiteralPath (AttachmentLead).FullName -Raw | ConvertFrom-Json
    if (!(($wire.input | Where-Object type -eq 'text').text -match 'Attachment transport check')) { throw 'Provider did not receive attached text' }
    if (($wire.input | Where-Object type -eq 'localImage').path -ne $image) { throw 'Provider did not receive the image' }
    if (($wire | ConvertTo-Json -Depth 12) -match 'capture\.png|/screenshot|shot-[A-Za-z0-9]+[/\\]') {throw 'Local screenshot entered model input'}
    if ($wire.input[0].text -notlike "*$project*") { throw 'Provider did not receive the selected folder' }
    Wait-For { Named 'Dispatched 2 subagents' } 'inline dispatch receipt'
    Click 'Activity'
    Wait-For { (Named 'Session activity') -and (Named 'Write left') } 'team activity panel'
    Click 'Write left'
    Wait-For { Named 'Acceptance · File contains its name' } 'worker acceptance criteria'
    Click 'Close activity'
    Wait-For { !(Named 'Session activity') } 'return to transcript'
    Test-Delivery
    Test-Sketch
    'Headless mode, navigation, composer, Studio, provider settings and agent activity passed'
} finally {
    [TestDesktop]::CloseApp($proc, $appWindow)
}
