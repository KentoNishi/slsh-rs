using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

class WindowsTerminalSmoke
{
    const int STD_INPUT_HANDLE = -10;
    const ushort KEY_EVENT = 0x0001;
    const uint LEFT_CTRL_PRESSED = 0x0008;
    static string tracePath = "";

    [StructLayout(LayoutKind.Sequential)]
    struct KEY_EVENT_RECORD
    {
        [MarshalAs(UnmanagedType.Bool)]
        public bool bKeyDown;
        public ushort wRepeatCount;
        public ushort wVirtualKeyCode;
        public ushort wVirtualScanCode;
        public char UnicodeChar;
        public uint dwControlKeyState;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct INPUT_RECORD
    {
        public ushort EventType;
        public KEY_EVENT_RECORD KeyEvent;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int nStdHandle);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool WriteConsoleInputW(IntPtr hConsoleInput, INPUT_RECORD[] lpBuffer, uint nLength, out uint lpNumberOfEventsWritten);

    static int Main(string[] args)
    {
        bool loopbackPowerShell = args.Length > 0 && args[0] == "--loopback-powershell";
        int offset = loopbackPowerShell ? 1 : 0;
        if (args.Length < offset + 3)
        {
            Console.Error.WriteLine("usage: WindowsTerminalSmoke.exe [--loopback-powershell] <slsh.exe> <host> <result-path>");
            return 2;
        }

        string slshExe = args[offset];
        string host = args[offset + 1];
        string resultPath = args[offset + 2];
        tracePath = resultPath + ".trace";
        string stamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds().ToString();
        string markerDir = Path.Combine(Path.GetTempPath(), "slsh-wt-smoke-" + stamp);
        Directory.CreateDirectory(markerDir);
        string backspaceMarker = loopbackPowerShell ? Path.Combine(markerDir, "backspace") : "/tmp/slsh-wt-backspace-" + stamp;
        string backspaceWrongMarker = backspaceMarker + "x";
        string cancelledMarker = loopbackPowerShell ? Path.Combine(markerDir, "cancelled") : "/tmp/slsh-wt-cancelled-" + stamp;
        string ctrlMarker = loopbackPowerShell ? Path.Combine(markerDir, "ctrl") : "/tmp/slsh-wt-ctrl-" + stamp;
        string keyMarker = loopbackPowerShell ? Path.Combine(markerDir, "keys") : "/tmp/slsh-wt-keys-" + stamp;
        string renderMarker = loopbackPowerShell ? Path.Combine(markerDir, "render") : "/tmp/slsh-wt-render-" + stamp;
        string logPath = Path.Combine(Path.GetTempPath(), "slsh-wt-smoke-" + stamp + ".log");
        string ssh = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "System32", "OpenSSH", "ssh.exe");

        try
        {
            if (File.Exists(logPath)) File.Delete(logPath);
            if (File.Exists(resultPath)) File.Delete(resultPath);
            if (File.Exists(tracePath)) File.Delete(tracePath);
            Trace("start");
            if (loopbackPowerShell)
            {
                DeleteLocal(backspaceMarker, backspaceWrongMarker, cancelledMarker, ctrlMarker, keyMarker, renderMarker);
            }
            else
            {
                RunChecked(ssh, SshArgs(host,
                    "rm -f '" + backspaceMarker + "' '" + backspaceWrongMarker + "' '" + cancelledMarker + "' '" + ctrlMarker + "' '" + keyMarker + "' '" + renderMarker + "'"
                ), "prepare markers");
            }
            Trace("prepared markers");

            ProcessStartInfo start = new ProcessStartInfo(slshExe, Quote(host));
            start.WorkingDirectory = Path.GetDirectoryName(slshExe);
            start.UseShellExecute = false;
            start.EnvironmentVariables["SLSH_KEY_LOG"] = logPath;
            if (loopbackPowerShell)
            {
                start.EnvironmentVariables["SLSH_LOOPBACK"] = "1";
                start.EnvironmentVariables["COMSPEC"] = WindowsPowerShellPath();
            }

            using (Process process = Process.Start(start))
            {
                Trace("started slsh pid " + process.Id);
                Thread.Sleep(5000);

                Text(loopbackPowerShell
                    ? "New-Item -ItemType File -Force " + backspaceWrongMarker
                    : "touch " + backspaceWrongMarker);
                Backspace();
                Enter();
                RequireFile(loopbackPowerShell, ssh, host, backspaceMarker, 40, "backspace marker");
                RequireMissingFile(loopbackPowerShell, ssh, host, backspaceWrongMarker, "backspace wrong marker");
                Trace("backspace passed");

                Text(loopbackPowerShell
                    ? "Start-Sleep 10; New-Item -ItemType File -Force " + cancelledMarker
                    : "touch " + cancelledMarker);
                CtrlC();
                Thread.Sleep(500);
                Text(loopbackPowerShell
                    ? "New-Item -ItemType File -Force " + ctrlMarker
                    : "touch " + ctrlMarker);
                Enter();
                RequireFile(loopbackPowerShell, ssh, host, ctrlMarker, 40, "ctrl-c followup marker");
                RequireMissingFile(loopbackPowerShell, ssh, host, cancelledMarker, "ctrl-c cancelled marker");
                Trace("ctrl-c passed");

                if (loopbackPowerShell)
                {
                    CtrlLeft();
                    CtrlRight();
                    CtrlDelete();
                    CtrlX();
                    Up();
                    Down();
                    RequireLocalKeyLogBytes(logPath);
                    CtrlC();
                    Thread.Sleep(500);
                }
                else
                {
                    Text("printf '\\033[?1h'");
                    Enter();
                    Thread.Sleep(500);
                    Text("cat > " + keyMarker);
                    Enter();
                    Thread.Sleep(500);
                    CtrlLeft();
                    CtrlRight();
                    CtrlDelete();
                    CtrlX();
                    Up();
                    Down();
                    Enter();
                    CtrlD();
                    RequireRemoteFile(ssh, host, keyMarker, 40, "modified key marker");
                    RequireRemoteModifiedKeyBytes(ssh, host, keyMarker);
                    Text("printf '\\033[?1l'");
                    Enter();
                    Thread.Sleep(500);
                }
                Trace("modified keys passed");

                Text(loopbackPowerShell
                    ? "Write-Host SLSHWTRED -ForegroundColor Red; New-Item -ItemType File -Force " + renderMarker
                    : "printf '\\033[31mSLSHWT%s\\033[0m\\n\\033)0\\016lqk\\017\\n' RED; touch " + renderMarker);
                Enter();
                RequireFile(loopbackPowerShell, ssh, host, renderMarker, 40, "render marker");
                Trace("render marker passed");

                Text("exit");
                Enter();
                if (!process.WaitForExit(10000))
                    throw new TimeoutException("slsh did not exit after remote exit command");
                Trace("stopped slsh");
            }

            WriteResult(resultPath, "PASS windows Terminal smoke passed", logPath);
            return 0;
        }
        catch (Exception ex)
        {
            Trace("crashed " + ex.GetType().FullName + ": " + ex.Message);
            File.WriteAllText(resultPath, "FAIL windows Terminal smoke crashed: " + ex + Environment.NewLine);
            return 1;
        }
    }

    static void Text(string text)
    {
        foreach (char ch in text)
        {
            Key(0, 0, ch);
            Thread.Sleep(25);
        }
    }

    static void Enter()
    {
        Key(0x0D, 0x1C, '\r');
    }

    static void Backspace()
    {
        Key(0x08, 0x0E, '\b');
    }

    static void CtrlC()
    {
        Key(0x43, 0x2E, '\x03', LEFT_CTRL_PRESSED);
    }

    static void CtrlD()
    {
        Key(0x44, 0x20, '\x04', LEFT_CTRL_PRESSED);
    }

    static void CtrlX()
    {
        Key(0x58, 0x2D, '\x18', LEFT_CTRL_PRESSED);
    }

    static void CtrlLeft()
    {
        Key(0x25, 0x4B, '\0', LEFT_CTRL_PRESSED);
    }

    static void CtrlRight()
    {
        Key(0x27, 0x4D, '\0', LEFT_CTRL_PRESSED);
    }

    static void CtrlDelete()
    {
        Key(0x2E, 0x53, '\0', LEFT_CTRL_PRESSED);
    }

    static void Up()
    {
        Key(0x26, 0x48, '\0');
    }

    static void Down()
    {
        Key(0x28, 0x50, '\0');
    }

    static void Key(ushort vk, ushort scan, char ch)
    {
        Key(vk, scan, ch, 0);
    }

    static void Key(ushort vk, ushort scan, char ch, uint control)
    {
        IntPtr input = GetStdHandle(STD_INPUT_HANDLE);
        INPUT_RECORD down = new INPUT_RECORD
        {
            EventType = KEY_EVENT,
            KeyEvent = new KEY_EVENT_RECORD { bKeyDown = true, wRepeatCount = 1, wVirtualKeyCode = vk, wVirtualScanCode = scan, UnicodeChar = ch, dwControlKeyState = control }
        };
        INPUT_RECORD up = new INPUT_RECORD
        {
            EventType = KEY_EVENT,
            KeyEvent = new KEY_EVENT_RECORD { bKeyDown = false, wRepeatCount = 1, wVirtualKeyCode = vk, wVirtualScanCode = scan, UnicodeChar = ch, dwControlKeyState = control }
        };
        uint written;
        if (!WriteConsoleInputW(input, new INPUT_RECORD[] { down, up }, 2, out written) || written != 2)
        {
            int error = Marshal.GetLastWin32Error();
            throw new System.ComponentModel.Win32Exception(error, "WriteConsoleInputW failed");
        }
    }

    static void RequireRemoteFile(string ssh, string host, string path, int attempts, string label)
    {
        for (int i = 0; i < attempts; i++)
        {
            Thread.Sleep(500);
            if (Run(ssh, SshArgs(host, "test -f '" + path + "'")) == 0) return;
        }
        throw new InvalidOperationException(label + " was not created: " + path);
    }

    static void RequireMissingRemoteFile(string ssh, string host, string path, string label)
    {
        if (Run(ssh, SshArgs(host, "test ! -e '" + path + "'")) != 0)
            throw new InvalidOperationException(label + " unexpectedly exists: " + path);
    }

    static void RequireRemoteModifiedKeyBytes(string ssh, string host, string path)
    {
        string command = "python3 -c \"import pathlib,sys; data=pathlib.Path('" + path + "').read_bytes(); esc=bytes([27]); expected=esc+b'[1;5D'+esc+b'[1;5C'+esc+b'[3;5~'+bytes([24])+esc+b'OA'+esc+b'OB'; sys.exit(0 if expected in data else 1)\"";
        if (Run(ssh, SshArgs(host, command)) != 0)
            throw new InvalidOperationException("modified key bytes missing from " + path);
    }

    static void RequireFile(bool local, string ssh, string host, string path, int attempts, string label)
    {
        if (!local)
        {
            RequireRemoteFile(ssh, host, path, attempts, label);
            return;
        }

        for (int i = 0; i < attempts; i++)
        {
            Thread.Sleep(500);
            if (File.Exists(path)) return;
        }
        throw new InvalidOperationException(label + " was not created: " + path);
    }

    static void RequireMissingFile(bool local, string ssh, string host, string path, string label)
    {
        if (!local)
        {
            RequireMissingRemoteFile(ssh, host, path, label);
            return;
        }

        if (File.Exists(path))
            throw new InvalidOperationException(label + " unexpectedly exists: " + path);
    }

    static void RequireLocalKeyLogBytes(string logPath)
    {
        for (int i = 0; i < 20; i++)
        {
            Thread.Sleep(250);
            if (!File.Exists(logPath)) continue;
            string log = ReadShared(logPath);
            if (log.Contains("bytes [27, 91, 49, 59, 53, 68]")
                && log.Contains("bytes [27, 91, 49, 59, 53, 67]")
                && log.Contains("bytes [27, 91, 51, 59, 53, 126]")
                && log.Contains("bytes [24]")
                && log.Contains("bytes [27, 91, 65]")
                && log.Contains("bytes [27, 91, 66]"))
                return;
        }

        string text = File.Exists(logPath) ? ReadShared(logPath) : "";
        throw new InvalidOperationException("modified key bytes missing from key log:\n" + text);
    }

    static void DeleteLocal(params string[] paths)
    {
        foreach (string path in paths)
        {
            if (File.Exists(path)) File.Delete(path);
        }
    }

    static string WindowsPowerShellPath()
    {
        return Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.Windows),
            "System32",
            "WindowsPowerShell",
            "v1.0",
            "powershell.exe");
    }

    static void WriteResult(string resultPath, string header, string logPath)
    {
        using (StreamWriter writer = new StreamWriter(resultPath, false))
        {
            writer.WriteLine(header);
            writer.WriteLine("Key log:");
            if (File.Exists(logPath)) writer.Write(ReadShared(logPath));
            if (File.Exists(tracePath))
            {
                writer.WriteLine("Trace:");
                writer.Write(ReadShared(tracePath));
            }
        }
    }

    static void Trace(string text)
    {
        if (tracePath.Length == 0) return;
        File.AppendAllText(tracePath, DateTime.UtcNow.ToString("o") + " " + text + Environment.NewLine);
    }

    static string ReadShared(string path)
    {
        using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
        using (StreamReader reader = new StreamReader(stream))
            return reader.ReadToEnd();
    }

    static void RunChecked(string exe, string args, string label)
    {
        int code = Run(exe, args);
        if (code != 0) throw new InvalidOperationException(label + " failed with exit code " + code);
    }

    static int Run(string exe, string args)
    {
        using (Process process = Process.Start(new ProcessStartInfo(exe, args) { UseShellExecute = false }))
        {
            if (!process.WaitForExit(10000))
            {
                try { process.Kill(); } catch { }
                throw new TimeoutException(exe + " timed out: " + args);
            }
            return process.ExitCode;
        }
    }

    static string SshArgs(string host, string remoteCommand)
    {
        return "-n -o BatchMode=yes -o ConnectTimeout=5 " + Quote(host) + " " + Quote(remoteCommand);
    }

    static string Quote(string value)
    {
        return "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"";
    }
}
