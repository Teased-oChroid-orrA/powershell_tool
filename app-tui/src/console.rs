//! The Windows console of a native window. `app-tui.exe` is a console program (the terminal toolboxes need a console),
//! so Windows gives a double-clicked `app-tui.exe` a console window of its own, which stayed open, blank, beside the
//! dashboard for as long as it ran. A native window lets go of a console that was made for it alone, and the
//! processes it starts are given no console window. No-ops elsewhere.

use std::process::Command;

/// Let go of this process's console when no other process shares it (Windows made it for this process at start-up).
/// A console shared with the shell that started us (`app-tui --gui` typed in a terminal) is kept.
pub fn release_own_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
            fn FreeConsole() -> i32;
        }
        let mut ids = [0u32; 2];
        // SAFETY: `ids` is a valid buffer of `ids.len()` process ids; the call returns the total number attached
        // (0 when there is no console), and FreeConsole only detaches this process.
        unsafe {
            if GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 {
                FreeConsole();
            }
        }
    }
}

/// Start `cmd` without a console window: a process started by a window that has no console would otherwise get a
/// new one.
pub fn without_console_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
