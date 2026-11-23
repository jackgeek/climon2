#![cfg(windows)]
//! The harness's Windows host (#320): `pwsh` in a pseudoconsole of the
//! pinned ConPTY under the Windows Terminal model, with the job shape set
//! separately (docs/for-humans/windows/qualification.md, "Host models").

use climon2_test_term::{
    ConPty, HostModel, HostSettings, JobShape, Key, Modifiers, Options, PWSH_PROMPT, Terminal,
    controlled_pwsh, pwsh,
};
use std::{fs, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(30);

/// `pwsh` at its prompt under `settings`, or `None` where it is missing locally.
fn shell(settings: HostSettings) -> Option<Terminal> {
    let path = controlled_pwsh()?;
    let mut terminal =
        Terminal::spawn_hosted(pwsh(&path), 30, 120, Options::default(), settings).unwrap();
    terminal.wait_for_text(PWSH_PROMPT, TIMEOUT).unwrap();
    Some(terminal)
}

/// Types `command` and presses Enter, as a user at the host's keyboard does.
fn run(terminal: &mut Terminal, command: &str) {
    terminal.type_text(command).unwrap();
    terminal.press(Key::Enter, Modifiers::NONE).unwrap();
}

#[test]
fn pwsh_starts_at_the_fixed_prompt_on_the_pinned_conpty_and_runs_typed_commands() {
    let Some(mut terminal) = shell(HostSettings::default()) else {
        return;
    };
    assert!(
        terminal.win32_input_mode(),
        "ConPTY asks its host for win32-input-mode and the model honours it"
    );
    let console = ConPty::pinned().unwrap().folder().join("OpenConsole.exe");
    run(
        &mut terminal,
        &format!(
            "'HOSTED=' + @(Get-CimInstance Win32_Process -Filter \"Name='OpenConsole.exe'\" \
             | Where-Object ExecutablePath -eq '{}').Count",
            console.display()
        ),
    );
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .lines()
                .any(|line| line.starts_with("HOSTED=") && line.trim_end() != "HOSTED=0")
        })
        .unwrap();
}

#[test]
fn shift_enter_reaches_the_program_as_a_key_record_with_shift() {
    let Some(mut terminal) = shell(HostSettings::default()) else {
        return;
    };
    // Plain VT has no Shift+Enter: it would arrive as Enter alone. PSReadLine
    // takes keys typed ahead while it still runs, so the key waits until the
    // command's own output shows it is running.
    run(
        &mut terminal,
        "'rea' + 'ding'; $k = [Console]::ReadKey($true); 'KEY=' + $k.Key + '+' + $k.Modifiers",
    );
    terminal.wait_for_text("reading", TIMEOUT).unwrap();
    terminal.press(Key::Enter, Modifiers::SHIFT).unwrap();
    terminal.wait_for_text("KEY=Enter+Shift", TIMEOUT).unwrap();
}

#[test]
fn ctrl_c_interrupts_the_running_command_and_the_shell_carries_on() {
    let Some(mut terminal) = shell(HostSettings::default()) else {
        return;
    };
    run(
        &mut terminal,
        "'sle' + 'eping'; Start-Sleep -Seconds 60; 'sl' + 'ept'",
    );
    terminal.wait_for_text("sleeping", TIMEOUT).unwrap();
    terminal.press(Key::Char('c'), Modifiers::CTRL).unwrap();
    // Keys typed before the next prompt may be discarded with the command.
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen.matches(PWSH_PROMPT).count() == 2)
        .unwrap();
    run(&mut terminal, "'after-' + 'ctrl-c'");
    terminal.wait_for_text("after-ctrl-c", TIMEOUT).unwrap();
    assert!(
        !terminal.screen().contains("slept"),
        "{}",
        terminal.screen()
    );
}

#[test]
fn a_resize_reaches_the_program() {
    let Some(mut terminal) = shell(HostSettings::default()) else {
        return;
    };
    terminal.resize(20, 100).unwrap();
    run(&mut terminal, "'W=' + $Host.UI.RawUI.WindowSize.Width");
    terminal.wait_for_text("W=100", TIMEOUT).unwrap();
}

#[test]
fn closing_the_pseudoconsole_ends_pwsh_with_the_close_status() {
    let Some(mut terminal) = shell(HostSettings {
        model: HostModel::WindowsTerminal,
        job: JobShape::None,
    }) else {
        return;
    };
    terminal.close();
    terminal.close();
    // STATUS_CONTROL_C_EXIT: pwsh's measured status for CTRL_CLOSE_EVENT.
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0xC000_013A);
}

/// The VS Code model sends plain VT keys, so Shift+Enter arrives as Enter
/// alone, and closes by ending the shell with status 1 once the
/// pseudoconsole is closed.
#[test]
fn the_vs_code_model_types_plain_vt_and_closes_by_ending_the_shell() {
    let vs_code = HostSettings {
        model: HostModel::VsCode,
        job: JobShape::None,
    };
    let Some(mut terminal) = shell(vs_code) else {
        return;
    };
    run(
        &mut terminal,
        "'rea' + 'ding'; $k = [Console]::ReadKey($true); 'KEY=' + $k.Key + '+' + $k.Modifiers",
    );
    terminal.wait_for_text("reading", TIMEOUT).unwrap();
    terminal.press(Key::Enter, Modifiers::SHIFT).unwrap();
    terminal.wait_for_text("KEY=Enter+", TIMEOUT).unwrap();
    assert!(
        !terminal.screen().contains("KEY=Enter+Shift"),
        "{}",
        terminal.screen()
    );
    terminal.close();
    terminal.close();
    let status = terminal.wait_exit(TIMEOUT).unwrap();
    // The shell may end on the close event before it is ended.
    assert!([1, 0xC000_013A].contains(&status), "{status:#x}");
}

#[test]
fn an_exit_status_passes_through_in_full() {
    let Some(mut terminal) = shell(HostSettings::default()) else {
        return;
    };
    run(&mut terminal, "exit 7");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
}

/// A PowerShell script that prints whether its process is in a job and the
/// error from starting a child with `CREATE_BREAKAWAY_FROM_JOB` (0 for none).
const JOB_PROBE: &str = r#"
Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Climon2JobProbe {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb; public IntPtr reserved, desktop, title;
        public int x, y, width, height, columns, rows, fill, flags;
        public short show, reserved2; public IntPtr reserved3, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation { public IntPtr process, thread; public int pid, tid; }
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcessW(string application, StringBuilder line, IntPtr processAttributes,
        IntPtr threadAttributes, bool inherit, uint flags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInformation information);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    public static string Probe() {
        bool inJob;
        IsProcessInJob(GetCurrentProcess(), IntPtr.Zero, out inJob);
        var startup = new StartupInfo();
        startup.cb = Marshal.SizeOf(startup);
        ProcessInformation information;
        var line = new StringBuilder(Environment.SystemDirectory + "\\cmd.exe /d /c exit 0");
        int error = 0;
        // CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW
        if (CreateProcessW(null, line, IntPtr.Zero, IntPtr.Zero, false, 0x01000000 | 0x08000000,
                IntPtr.Zero, null, ref startup, out information)) {
            WaitForSingleObject(information.process, 10000);
            CloseHandle(information.process);
            CloseHandle(information.thread);
        } else {
            error = Marshal.GetLastWin32Error();
        }
        return "JOB=" + inJob + " BREAKAWAY=" + error;
    }
}
'@
[Climon2JobProbe]::Probe()
"#;

/// What the job probe reports from inside `pwsh` started under `job`.
fn probe_job(job: JobShape) -> Option<(bool, u32)> {
    let mut terminal = shell(HostSettings {
        job,
        ..HostSettings::default()
    })?;
    let scratch = tempfile::tempdir().unwrap();
    let script = scratch.path().join("job-probe.ps1");
    fs::write(&script, JOB_PROBE).unwrap();
    run(
        &mut terminal,
        &format!(
            "& ([scriptblock]::Create((Get-Content -Raw -LiteralPath '{}')))",
            script.display()
        ),
    );
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen.contains(" BREAKAWAY="))
        .unwrap();
    let screen = terminal.screen();
    let report = screen
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("JOB="))
        .unwrap_or_else(|| panic!("no job report:\n{screen}"));
    let (in_job, breakaway) = report.split_once(" BREAKAWAY=").unwrap();
    Some((in_job == "True", breakaway.parse().unwrap()))
}

#[test]
fn a_job_that_allows_breakaway_holds_the_shell_and_lets_a_child_leave() {
    if let Some(report) = probe_job(JobShape::AllowsBreakaway) {
        assert_eq!(report, (true, 0));
    }
}

#[test]
fn a_job_that_refuses_breakaway_holds_the_shell_and_denies_a_child_leaving() {
    // ERROR_ACCESS_DENIED
    if let Some(report) = probe_job(JobShape::RefusesBreakaway) {
        assert_eq!(report, (true, 5));
    }
}

#[test]
fn no_job_leaves_the_shell_in_whatever_job_the_test_is_in() {
    if let Some((in_job, _)) = probe_job(JobShape::None) {
        assert_eq!(in_job, test_is_in_a_job());
    }
}

fn test_is_in_a_job() -> bool {
    use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
    let mut in_job = 0;
    // The pseudo-handle needs no closing; the call writes one BOOL.
    assert_ne!(
        unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) },
        0
    );
    in_job != 0
}
