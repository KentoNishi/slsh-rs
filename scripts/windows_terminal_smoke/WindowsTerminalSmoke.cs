using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

class WindowsTerminalSmoke
{
    const int STD_INPUT_HANDLE = -10;
    const ushort KEY_EVENT = 0x0001;

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
        if (args.Length < 3)
        {
            Console.Error.WriteLine("usage: WindowsTerminalSmoke.exe <slsh.exe> <host> <result-path>");
            return 2;
        }

        string slshExe = args[0];
        string host = args[1];
        string resultPath = args[2];
        string stamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds().ToString();
        string marker = "/tmp/slsh-wt-smoke-" + stamp;
        string logPath = Path.Combine(Path.GetTempPath(), "slsh-wt-smoke-" + stamp + ".log");
        string ssh = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "System32", "OpenSSH", "ssh.exe");

        try
        {
            if (File.Exists(logPath)) File.Delete(logPath);
            if (File.Exists(resultPath)) File.Delete(resultPath);
            RunChecked(ssh, "-o BatchMode=yes -o ConnectTimeout=5 " + Quote(host) + " " + Quote("rm -f '" + marker + "'"), "prepare marker");

            ProcessStartInfo start = new ProcessStartInfo(slshExe, Quote(host));
            start.WorkingDirectory = Path.GetDirectoryName(slshExe);
            start.UseShellExecute = false;
            start.EnvironmentVariables["SLSH_KEY_LOG"] = logPath;

            using (Process process = Process.Start(start))
            {
                Thread.Sleep(5000);
                Text("touch " + marker);
                Enter();

                bool ok = false;
                for (int i = 0; i < 40; i++)
                {
                    Thread.Sleep(500);
                    if (Run(ssh, "-o BatchMode=yes -o ConnectTimeout=5 " + Quote(host) + " " + Quote("test -f '" + marker + "'")) == 0)
                    {
                        ok = true;
                        break;
                    }
                }

                Text("exit");
                Enter();
                Thread.Sleep(1000);
                if (!process.HasExited) process.Kill();

                if (!ok)
                {
                    WriteResult(resultPath, "FAIL windows Terminal smoke failed: marker was not created: " + marker, logPath);
                    return 1;
                }
            }

            WriteResult(resultPath, "PASS windows Terminal smoke passed", logPath);
            return 0;
        }
        catch (Exception ex)
        {
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

    static void Key(ushort vk, ushort scan, char ch)
    {
        IntPtr input = GetStdHandle(STD_INPUT_HANDLE);
        INPUT_RECORD down = new INPUT_RECORD
        {
            EventType = KEY_EVENT,
            KeyEvent = new KEY_EVENT_RECORD { bKeyDown = true, wRepeatCount = 1, wVirtualKeyCode = vk, wVirtualScanCode = scan, UnicodeChar = ch, dwControlKeyState = 0 }
        };
        INPUT_RECORD up = new INPUT_RECORD
        {
            EventType = KEY_EVENT,
            KeyEvent = new KEY_EVENT_RECORD { bKeyDown = false, wRepeatCount = 1, wVirtualKeyCode = vk, wVirtualScanCode = scan, UnicodeChar = ch, dwControlKeyState = 0 }
        };
        uint written;
        if (!WriteConsoleInputW(input, new INPUT_RECORD[] { down, up }, 2, out written) || written != 2)
        {
            int error = Marshal.GetLastWin32Error();
            throw new System.ComponentModel.Win32Exception(error, "WriteConsoleInputW failed");
        }
    }

    static void WriteResult(string resultPath, string header, string logPath)
    {
        using (StreamWriter writer = new StreamWriter(resultPath, false))
        {
            writer.WriteLine(header);
            writer.WriteLine("Key log:");
            if (File.Exists(logPath)) writer.Write(ReadShared(logPath));
        }
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
            process.WaitForExit();
            return process.ExitCode;
        }
    }

    static string Quote(string value)
    {
        return "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"";
    }
}
