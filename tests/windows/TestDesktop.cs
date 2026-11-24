using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

// Runs native UI automation on a separate desktop without changing the input desktop.
public sealed class TestDesktop : IDisposable {
    IntPtr desktop, job, original;
    Process worker;
    private TestDesktop() {}

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct Startup {
        public int size;
        public string reserved, desktop, title;
        public int x, y, width, height, columns, rows, fill, flags;
        public short show, reservedSize;
        public IntPtr reservedBytes, stdin, stdout, stderr;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public int pid, tid; }
    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimits {
        public long processTime, jobTime;
        public uint flags;
        public UIntPtr minMemory, maxMemory;
        public uint activeProcesses;
        public UIntPtr affinity;
        public uint priority, schedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct JobLimits {
        public BasicLimits basic;
        public ulong readOps, writeOps, otherOps, readBytes, writeBytes, otherBytes;
        public UIntPtr processMemory, jobMemory, peakProcessMemory, peakJobMemory;
    }

    [DllImport("user32", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr mode, uint flags, uint access, IntPtr security);
    [DllImport("user32")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32")] static extern IntPtr GetThreadDesktop(uint tid);
    [DllImport("kernel32")] static extern uint GetCurrentThreadId();
    [DllImport("user32", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool GetUserObjectInformation(IntPtr handle, int index, StringBuilder value, int bytes, out int needed);
    [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool CreateProcess(string app, StringBuilder command, IntPtr pa, IntPtr ta, bool inherit, uint flags,
        IntPtr environment, string cwd, ref Startup startup, out ProcessInfo info);
    [DllImport("kernel32", SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr security, string name);
    [DllImport("kernel32", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job, int type, ref JobLimits limits, int size);
    [DllImport("kernel32", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32", SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32")] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32", SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32")] static extern bool CloseHandle(IntPtr handle);

    delegate bool WindowVisitor(IntPtr window, IntPtr data);
    [DllImport("user32")] static extern bool EnumDesktopWindows(IntPtr desktop, WindowVisitor visitor, IntPtr data);
    [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr window, out int pid);
    [DllImport("user32", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder title, int capacity);
    [DllImport("user32")] static extern bool PostMessage(IntPtr window, uint message, IntPtr wp, IntPtr lp);

    public static IntPtr WaitForWindow(Process process) {
        var deadline = Stopwatch.StartNew();
        var desktop = GetThreadDesktop(GetCurrentThreadId());
        while (deadline.Elapsed.TotalSeconds < 10 && !process.HasExited) {
            var found = IntPtr.Zero;
            EnumDesktopWindows(desktop, delegate(IntPtr window, IntPtr data) {
                int pid;
                GetWindowThreadProcessId(window, out pid);
                if (pid != process.Id) return true;
                var title = new StringBuilder(512);
                GetWindowText(window, title, title.Capacity);
                if (title.ToString() != "Workshop" && !title.ToString().EndsWith(" · Workshop", StringComparison.Ordinal)) return true;
                found = window;
                return false;
            }, IntPtr.Zero);
            if (found != IntPtr.Zero) return found;
            System.Threading.Thread.Sleep(50);
        }
        throw new InvalidOperationException("Workshop did not open its window on the test desktop.");
    }

    public static void CloseApp(Process process, IntPtr window) {
        if (process.HasExited) return;
        if (window != IntPtr.Zero) PostMessage(window, 0x0010, IntPtr.Zero, IntPtr.Zero);
        if (!process.WaitForExit(5000)) { process.Kill(); process.WaitForExit(); }
    }

    [DllImport("user32")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("kernel32")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32")] static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool member);

    // Check only ownership and visibility. Users can switch between their own windows during a run.
    static void CheckOriginalDesktop(IntPtr original, IntPtr job) {
        bool escaped = false;
        EnumDesktopWindows(original, delegate(IntPtr window, IntPtr data) {
            if (!IsWindowVisible(window)) return true;
            int pid;
            GetWindowThreadProcessId(window, out pid);
            IntPtr process = OpenProcess(0x1000, false, pid); // PROCESS_QUERY_LIMITED_INFORMATION
            if (process == IntPtr.Zero) return true;
            try {
                bool member;
                escaped = IsProcessInJob(process, job, out member) && member;
                return !escaped;
            } finally { CloseHandle(process); }
        }, IntPtr.Zero);
        if (escaped) throw new InvalidOperationException("A UI test window appeared on the calling desktop.");
    }

    public static string CurrentName() {
        var name = new StringBuilder(256);
        int needed;
        if (!GetUserObjectInformation(GetThreadDesktop(GetCurrentThreadId()), 2, name, name.Capacity * 2, out needed))
            throw new Win32Exception();
        return name.ToString();
    }

    // All callers pass file paths or base64 text. Windows paths cannot contain a quote.
    static string Quote(string value) {
        if (value.Contains("\"")) throw new ArgumentException("Unexpected quote in path");
        return "\"" + value + new string('\\', value.Length - value.TrimEnd('\\').Length) + "\"";
    }

    static Process Start(string name, string exe, string args, string cwd, IntPtr job) {
        var startup = new Startup { size = Marshal.SizeOf(typeof(Startup)), desktop = name };
        ProcessInfo child;
        // CREATE_NO_WINDOW | CREATE_SUSPENDED lets the supervisor own the tree before any code runs.
        if (!CreateProcess(exe, new StringBuilder(Quote(exe) + " " + args), IntPtr.Zero, IntPtr.Zero, false,
                0x08000004, IntPtr.Zero, cwd, ref startup, out child)) throw new Win32Exception();
        try {
            if (job != IntPtr.Zero && !AssignProcessToJobObject(job, child.process)) throw new Win32Exception();
            var process = Process.GetProcessById(child.pid);
            var handle = process.SafeHandle; // Keep the process queryable after a fast exit.
            if (ResumeThread(child.thread) == uint.MaxValue) throw new Win32Exception();
            return process;
        } catch {
            TerminateProcess(child.process, 1);
            throw;
        } finally {
            CloseHandle(child.thread);
            CloseHandle(child.process);
        }
    }

    public static Process StartApp(string exe, string project) {
        string name = CurrentName();
        if (!name.StartsWith("WorkshopTest-", StringComparison.Ordinal))
            throw new InvalidOperationException("Workshop UI tests require their private desktop.");
        return Start(name, exe, Quote(project), project, IntPtr.Zero);
    }

    public static TestDesktop Launch(string name, string shell, string encodedScript, string cwd) {
        var run = new TestDesktop();
        try {
            run.original = GetThreadDesktop(GetCurrentThreadId());
            const uint desktopAccess = 0x0083; // READOBJECTS | CREATEWINDOW | WRITEOBJECTS
            run.desktop = CreateDesktop(name, IntPtr.Zero, IntPtr.Zero, 0, desktopAccess, IntPtr.Zero);
            if (run.desktop == IntPtr.Zero) throw new Win32Exception();
            run.job = CreateJobObject(IntPtr.Zero, null);
            if (run.job == IntPtr.Zero) throw new Win32Exception();
            var limits = new JobLimits();
            limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if (!SetInformationJobObject(run.job, 9, ref limits, Marshal.SizeOf(typeof(JobLimits)))) throw new Win32Exception();
            run.worker = Start(name, shell, "-NoProfile -NonInteractive -EncodedCommand " + encodedScript, cwd, run.job);
            return run;
        } catch {
            run.Dispose();
            throw;
        }
    }

    public bool Finished() {
        CheckOriginalDesktop(original, job);
        return worker.HasExited;
    }

    public int ExitCode() {
        uint code;
        if (!GetExitCodeProcess(worker.SafeHandle.DangerousGetHandle(), out code)) throw new Win32Exception();
        return (int)code;
    }

    public void Dispose() {
        if (job != IntPtr.Zero) { CloseHandle(job); job = IntPtr.Zero; }
        if (worker != null) { worker.Dispose(); worker = null; }
        if (desktop != IntPtr.Zero) { CloseDesktop(desktop); desktop = IntPtr.Zero; }
    }
}
