using Microsoft.Win32.SafeHandles;
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

class ConptySmoke
{
    const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
    const uint CREATE_UNICODE_ENVIRONMENT = 0x00000400;
    const int STARTF_USESTDHANDLES = 0x00000100;
    static readonly IntPtr PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = new IntPtr(0x00020016);

    [StructLayout(LayoutKind.Sequential)]
    struct COORD { public short X; public short Y; }

    [StructLayout(LayoutKind.Sequential)]
    struct STARTUPINFO
    {
        public int cb;
        public IntPtr lpReserved;
        public IntPtr lpDesktop;
        public IntPtr lpTitle;
        public int dwX;
        public int dwY;
        public int dwXSize;
        public int dwYSize;
        public int dwXCountChars;
        public int dwYCountChars;
        public int dwFillAttribute;
        public int dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct STARTUPINFOEX
    {
        public STARTUPINFO StartupInfo;
        public IntPtr lpAttributeList;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CreatePipe(out IntPtr hReadPipe, out IntPtr hWritePipe, IntPtr lpPipeAttributes, int nSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr hObject);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern int CreatePseudoConsole(COORD size, IntPtr hInput, IntPtr hOutput, uint dwFlags, out IntPtr phPC);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern void ClosePseudoConsole(IntPtr hPC);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool InitializeProcThreadAttributeList(IntPtr lpAttributeList, int dwAttributeCount, int dwFlags, ref IntPtr lpSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool UpdateProcThreadAttribute(IntPtr lpAttributeList, uint dwFlags, IntPtr Attribute, IntPtr lpValue, IntPtr cbSize, IntPtr lpPreviousValue, IntPtr lpReturnSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern void DeleteProcThreadAttributeList(IntPtr lpAttributeList);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(
        string lpApplicationName,
        StringBuilder lpCommandLine,
        IntPtr lpProcessAttributes,
        IntPtr lpThreadAttributes,
        bool bInheritHandles,
        uint dwCreationFlags,
        IntPtr lpEnvironment,
        string lpCurrentDirectory,
        ref STARTUPINFOEX lpStartupInfo,
        out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr hHandle, uint dwMilliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr hProcess, uint uExitCode);

    static int Main(string[] args)
    {
        bool selfTest = args.Length > 0 && args[0] == "--self-test";
        bool startupDump = args.Length > 0 && args[0] == "--startup-dump";
        string exe = selfTest ? Environment.GetEnvironmentVariable("COMSPEC") : (args.Length > 0 ? args[startupDump ? 1 : 0] : "slsh.exe");
        string commandLine = selfTest ? Quote(exe) + " /k" : Quote(exe) + " " + (args.Length > (startupDump ? 2 : 1) ? args[startupDump ? 2 : 1] : "wsl");
        string workdir = Path.GetDirectoryName(exe);
        string marker = selfTest ? "CONPTYSELFOK" : "SLSHCONPTYOK";
        string inputText = selfTest ? "echo CONPTYSELFOK\r" : "echo SLSHCONPTYx\x7fOK\r";
        string logPath = Path.Combine(Path.GetTempPath(), "slsh-conpty-keys.log");
        if (File.Exists(logPath)) File.Delete(logPath);

        IntPtr inputRead, inputWrite, outputRead, outputWrite;
        Check(CreatePipe(out inputRead, out inputWrite, IntPtr.Zero, 0), "CreatePipe input");
        Check(CreatePipe(out outputRead, out outputWrite, IntPtr.Zero, 0), "CreatePipe output");

        IntPtr hpc;
        int hr = CreatePseudoConsole(new COORD { X = 100, Y = 30 }, inputRead, outputWrite, 0, out hpc);
        if (hr != 0) throw new InvalidOperationException("CreatePseudoConsole failed: 0x" + hr.ToString("x"));
        CloseHandle(inputRead);
        CloseHandle(outputWrite);

        IntPtr attrSize = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attrSize);
        IntPtr attrList = Marshal.AllocHGlobal(attrSize);
        Check(InitializeProcThreadAttributeList(attrList, 1, 0, ref attrSize), "InitializeProcThreadAttributeList");
        Check(UpdateProcThreadAttribute(attrList, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, hpc, new IntPtr(IntPtr.Size), IntPtr.Zero, IntPtr.Zero), "UpdateProcThreadAttribute");

        STARTUPINFOEX si = new STARTUPINFOEX();
        si.StartupInfo.cb = Marshal.SizeOf(typeof(STARTUPINFOEX));
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = new IntPtr(-1);
        si.StartupInfo.hStdOutput = new IntPtr(-1);
        si.StartupInfo.hStdError = new IntPtr(-1);
        si.lpAttributeList = attrList;

        PROCESS_INFORMATION pi;
        string envBlock = BuildEnvironmentBlock("SLSH_KEY_LOG", logPath);
        IntPtr env = Marshal.StringToHGlobalUni(envBlock);
        bool started = CreateProcessW(
            null,
            new StringBuilder(commandLine),
            IntPtr.Zero,
            IntPtr.Zero,
            false,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env,
            workdir,
            ref si,
            out pi);
        Marshal.FreeHGlobal(env);
        Check(started, "CreateProcessW");

        var input = new FileStream(new SafeFileHandle(inputWrite, true), FileAccess.Write);
        var output = new FileStream(new SafeFileHandle(outputRead, true), FileAccess.Read);
        var seen = new StringBuilder();
        bool sent = false;
        bool startupChecked = selfTest || startupDump;
        DateTime? startupSeenAt = null;
        DateTime sendAt = DateTime.UtcNow.AddSeconds(selfTest ? 1 : 3);
        DateTime deadline = DateTime.UtcNow.AddSeconds(selfTest ? 8 : 25);
        byte[] buffer = new byte[4096];

        Thread reader = new Thread(() =>
        {
            while (true)
            {
                int n;
                try { n = output.Read(buffer, 0, buffer.Length); }
                catch { break; }
                if (n <= 0) break;
                string chunk = Encoding.UTF8.GetString(buffer, 0, n);
                lock (seen) seen.Append(chunk);
                if (chunk.Contains("\x1b[6n")) Write(input, "\x1b[1;1R");
            }
        });
        reader.IsBackground = true;
        reader.Start();

        while (DateTime.UtcNow < deadline)
        {
            string text;
            lock (seen) text = seen.ToString();
            if (!startupChecked && LooksLikePrompt(text))
            {
                if (startupSeenAt == null) startupSeenAt = DateTime.UtcNow;
                if (DateTime.UtcNow - startupSeenAt.Value >= TimeSpan.FromMilliseconds(1200))
                {
                    string screen;
                    if (!StartupScreenOk(text, out screen))
                    {
                        TerminateProcess(pi.hProcess, 1);
                        Console.Error.WriteLine("windows ConPTY startup screen failed");
                        Console.Error.WriteLine("Screen:");
                        Console.Error.WriteLine(screen);
                        Console.Error.WriteLine("Captured:");
                        Console.Error.WriteLine(text.Replace("\x1b", "<ESC>"));
                        Cleanup(pi, attrList, hpc);
                        return 1;
                    }
                    startupChecked = true;
                }
            }
            if (startupDump && LooksLikePrompt(text))
            {
                if (startupSeenAt == null) startupSeenAt = DateTime.UtcNow;
                if (DateTime.UtcNow - startupSeenAt.Value >= TimeSpan.FromMilliseconds(2000))
                {
                    TerminateProcess(pi.hProcess, 1);
                    Console.WriteLine("Screen:");
                    Console.WriteLine(ReduceTerminal(text));
                    Console.WriteLine("Captured:");
                    Console.WriteLine(text.Replace("\x1b", "<ESC>"));
                    Cleanup(pi, attrList, hpc);
                    return 0;
                }
            }
            if (!startupDump && !sent && (startupChecked || DateTime.UtcNow >= sendAt))
            {
                Type(input, inputText);
                sent = true;
            }
            if (Count(text, marker) >= 1)
            {
                Write(input, "exit\r");
                Cleanup(pi, attrList, hpc);
                Console.WriteLine("windows ConPTY smoke passed");
                if (File.Exists(logPath)) Console.WriteLine(ReadShared(logPath));
                return 0;
            }
            Thread.Sleep(50);
        }

        TerminateProcess(pi.hProcess, 1);
        Console.Error.WriteLine("windows ConPTY smoke failed");
        Console.Error.WriteLine("Captured:");
        lock (seen) Console.Error.WriteLine(seen.ToString().Replace("\x1b", "<ESC>"));
        if (File.Exists(logPath))
        {
            Console.Error.WriteLine("Key log:");
            Console.Error.WriteLine(ReadShared(logPath));
        }
        Cleanup(pi, attrList, hpc);
        return 1;
    }

    static void Cleanup(PROCESS_INFORMATION pi, IntPtr attrList, IntPtr hpc)
    {
        WaitForSingleObject(pi.hProcess, 3000);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        DeleteProcThreadAttributeList(attrList);
        Marshal.FreeHGlobal(attrList);
        ClosePseudoConsole(hpc);
    }

    static string BuildEnvironmentBlock(string name, string value)
    {
        var env = Environment.GetEnvironmentVariables();
        var pairs = new System.Collections.Generic.List<string>();
        foreach (System.Collections.DictionaryEntry entry in env)
        {
            if (!string.Equals((string)entry.Key, name, StringComparison.OrdinalIgnoreCase))
                pairs.Add((string)entry.Key + "=" + (string)entry.Value);
        }
        pairs.Add(name + "=" + value);
        pairs.Sort(StringComparer.OrdinalIgnoreCase);
        return string.Join("\0", pairs.ToArray()) + "\0\0";
    }

    static string Quote(string value) { return "\"" + value.Replace("\"", "\\\"") + "\""; }

    static void Write(Stream stream, string text)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        stream.Write(bytes, 0, bytes.Length);
        stream.Flush();
    }

    static void Type(Stream stream, string text)
    {
        foreach (char ch in text)
        {
            Write(stream, ch.ToString());
            Thread.Sleep(25);
        }
    }

    static string ReadShared(string path)
    {
        using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
        using (var reader = new StreamReader(stream))
            return reader.ReadToEnd();
    }

    static bool LooksLikePrompt(string text) { return text.Contains("$ ") || text.Contains("# ") || text.Contains("> "); }

    static bool StartupScreenOk(string text, out string screen)
    {
        screen = ReduceTerminal(text);
        return !screen.Contains("exec tmux -CC") && PromptLineCount(screen) <= 1;
    }

    static string ReduceTerminal(string text)
    {
        const int rows = 30, cols = 100;
        char[,] screen = new char[rows, cols];
        for (int r = 0; r < rows; r++)
            for (int c = 0; c < cols; c++)
                screen[r, c] = ' ';

        int row = 0, col = 0;
        for (int i = 0; i < text.Length; i++)
        {
            char ch = text[i];
            if (ch == '\x1b')
            {
                i = ApplyEscape(text, i, screen, ref row, ref col);
                continue;
            }
            if (ch == '\r')
            {
                col = 0;
            }
            else if (ch == '\n')
            {
                row++;
                if (row >= rows)
                {
                    Scroll(screen);
                    row = rows - 1;
                }
            }
            else if (ch >= ' ')
            {
                if (row >= 0 && row < rows && col >= 0 && col < cols)
                    screen[row, col] = ch;
                if (col + 1 < cols) col++;
            }
        }

        var builder = new StringBuilder();
        for (int r = 0; r < rows; r++)
        {
            int end = cols;
            while (end > 0 && screen[r, end - 1] == ' ') end--;
            builder.Append(new string(RowChars(screen, r, end)));
            builder.Append('\n');
        }
        return builder.ToString();
    }

    static int ApplyEscape(string text, int index, char[,] screen, ref int row, ref int col)
    {
        if (index + 1 >= text.Length || text[index + 1] != '[') return index;
        int end = index + 2;
        while (end < text.Length && !char.IsLetter(text[end])) end++;
        if (end >= text.Length) return text.Length - 1;

        string body = text.Substring(index + 2, end - index - 2);
        char action = text[end];
        if (action == 'J' && body.EndsWith("2", StringComparison.Ordinal))
            Clear(screen);
        else if (action == 'K')
            for (int c = col; c < screen.GetLength(1); c++) screen[row, c] = ' ';
        else if (action == 'H')
        {
            string[] parts = body.Split(';');
            if (body.Length == 0)
            {
                row = 0;
                col = 0;
            }
            else if (parts.Length >= 2)
            {
                int r, c;
                if (int.TryParse(parts[0], out r) && int.TryParse(parts[1], out c))
                {
                    row = Math.Max(0, Math.Min(screen.GetLength(0) - 1, r - 1));
                    col = Math.Max(0, Math.Min(screen.GetLength(1) - 1, c - 1));
                }
            }
        }
        return end;
    }

    static int PromptLineCount(string screen)
    {
        int count = 0;
        foreach (string line in screen.Split('\n'))
        {
            string trimmed = line.TrimEnd();
            if (trimmed.EndsWith("$", StringComparison.Ordinal) ||
                trimmed.EndsWith("#", StringComparison.Ordinal) ||
                trimmed.EndsWith(">", StringComparison.Ordinal))
                count++;
        }
        return count;
    }

    static char[] RowChars(char[,] screen, int row, int len)
    {
        char[] chars = new char[len];
        for (int i = 0; i < len; i++) chars[i] = screen[row, i];
        return chars;
    }

    static void Clear(char[,] screen)
    {
        for (int r = 0; r < screen.GetLength(0); r++)
            for (int c = 0; c < screen.GetLength(1); c++)
                screen[r, c] = ' ';
    }

    static void Scroll(char[,] screen)
    {
        int rows = screen.GetLength(0), cols = screen.GetLength(1);
        for (int r = 0; r < rows - 1; r++)
            for (int c = 0; c < cols; c++)
                screen[r, c] = screen[r + 1, c];
        for (int c = 0; c < cols; c++)
            screen[rows - 1, c] = ' ';
    }

    static int Count(string text, string needle)
    {
        int count = 0, index = 0;
        while ((index = text.IndexOf(needle, index, StringComparison.Ordinal)) >= 0)
        {
            count++;
            index += needle.Length;
        }
        return count;
    }

    static void Check(bool ok, string label)
    {
        if (!ok)
        {
            int error = Marshal.GetLastWin32Error();
            throw new System.ComponentModel.Win32Exception(error, label + " failed with " + error);
        }
    }
}
