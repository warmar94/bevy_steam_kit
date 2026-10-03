//! Is the Steam client PROCESS still alive? Answered from the operating system, never through
//! Steam (feature `steam`).
//!
//! Why: when the Steam client is killed or crashes it sends no shutdown callback, and the next call
//! into Steam's library (the next callback pump first of all) waits on the dead pipe. So the real
//! backend asks this module before every pump and before every feature access, and makes no Steam
//! call once the process is gone.
//!
//! - Windows: the client's pid from `HKCU\Software\Valve\Steam\ActiveProcess\pid` (where Steam
//!   writes it), opened ONCE with `SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION`; the image
//!   file name must be `steam.exe` (not `xsteam.exe`). The check is `WaitForSingleObject(handle, 0)`: never blocks,
//!   and the open handle keeps the process object, so a reused pid cannot be mistaken for Steam.
//! - Linux: the pid from `~/.steam/steam.pid` (or `~/.steam/steam/steam.pid`); `/proc/<pid>/stat`
//!   must name a process whose command is exactly `steam` (the client binary `ubuntu12_32/steam`; not
//!   `steamwebhelper` or a runtime helper). Its start time is kept; the check reads
//!   `/proc/<pid>/stat` again (procfs, never blocks): missing, a zombie, or another start time (a
//!   reused pid) = ended. A Flatpak or Snap Steam keeps its pid file elsewhere and in another pid
//!   namespace: the file is not found or names another process, and the check is off.
//! - Other systems (macOS): no check (off).
//!
//! Own `extern` declarations with `#[link(name = "kernel32")]` / `#[link(name = "advapi32")]` (system
//! libraries of every Windows): no dependency.

/// The watched Steam client process.
pub(crate) struct SteamProcess(imp::Watch);

impl SteamProcess {
    /// Find the running Steam client. `Err` says why the check is not available here.
    pub(crate) fn find() -> Result<Self, String> {
        imp::find().map(Self)
    }

    /// `true` once the watched process has ended. Never blocks, makes no Steam call.
    pub(crate) fn has_ended(&self) -> bool {
        self.0.has_ended()
    }

    /// Watch the process with this pid (tests: any process; `name`: the image / command name it
    /// must have, `None` = any).
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(crate) fn for_pid(pid: u32, name: Option<&str>) -> Result<Self, String> {
        imp::watch(pid, name).map(Self)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    type Handle = *mut c_void;

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const WAIT_TIMEOUT: u32 = 0x102;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    /// `HKEY_CURRENT_USER`: `(HKEY)(ULONG_PTR)(LONG)0x80000001`, sign-extended.
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
        fn QueryFullProcessImageNameW(handle: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: isize, sub_key: *const u16, value: *const u16, flags: u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) struct Watch(Handle);

    // SAFETY: a process handle is a kernel object handle, usable from any thread; it is only
    // waited on and closed once (in `Drop`).
    unsafe impl Send for Watch {}
    // SAFETY: as above (`WaitForSingleObject` on a shared handle is thread-safe).
    unsafe impl Sync for Watch {}

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: the handle came from `OpenProcess` and is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Watch {
        pub(super) fn has_ended(&self) -> bool {
            // SAFETY: a valid process handle; a zero timeout never blocks.
            unsafe { WaitForSingleObject(self.0, 0) != WAIT_TIMEOUT }
        }
    }

    pub(super) fn find() -> Result<Watch, String> {
        let sub_key = wide(r"Software\Valve\Steam\ActiveProcess");
        let value = wide("pid");
        let mut pid: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        // SAFETY: valid NUL-terminated wide strings; `pid` / `len` are a 4-byte buffer and its size.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, sub_key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut pid as *mut u32).cast(), &mut len)
        };
        if status != 0 || pid == 0 {
            return Err(format!("no Steam pid in the registry (status {status})"));
        }
        watch(pid, Some("steam.exe"))
    }

    pub(super) fn watch(pid: u32, name: Option<&str>) -> Result<Watch, String> {
        // SAFETY: plain call; a null result is handled.
        let handle = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return Err(format!("cannot open process {pid}"));
        }
        let watch = Watch(handle);
        if let Some(name) = name {
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            // SAFETY: `buf` holds `size` u16s; the handle has the query right.
            let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
            let path = String::from_utf16_lossy(&buf[..size.min(buf.len() as u32) as usize]).to_ascii_lowercase();
            if ok == 0 || !(path == name || path.ends_with(&format!("\\{name}"))) {
                return Err(format!("process {pid} is not {name}"));
            }
        }
        if watch.has_ended() {
            return Err(format!("process {pid} has already ended"));
        }
        Ok(watch)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;

    pub(super) struct Watch {
        pub(super) stat: PathBuf,
        pub(super) start_time: u64,
    }

    /// `(state, start time, command)` from the text of `/proc/<pid>/stat`.
    pub(crate) fn parse_stat(text: &str) -> Option<(char, u64, &str)> {
        let open = text.find('(')?;
        let close = text.rfind(')')?;
        let comm = text.get(open + 1..close)?;
        let mut rest = text.get(close + 1..)?.split_whitespace();
        let state = rest.next()?.chars().next()?;
        // Field 3 is the state; the start time is field 22, i.e. 19 fields further.
        let start_time = rest.nth(18)?.parse().ok()?;
        Some((state, start_time, comm))
    }

    impl Watch {
        pub(super) fn has_ended(&self) -> bool {
            match std::fs::read_to_string(&self.stat).ok().as_deref().and_then(parse_stat) {
                Some((state, start_time, _)) => start_time != self.start_time || matches!(state, 'Z' | 'X' | 'x'),
                None => true,
            }
        }
    }

    pub(super) fn find() -> Result<Watch, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let home = PathBuf::from(home);
        let pid = [home.join(".steam/steam.pid"), home.join(".steam/steam/steam.pid")]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok()?.trim().parse::<u32>().ok())
            .ok_or("no readable ~/.steam/steam.pid (a Flatpak or Snap Steam keeps it elsewhere)")?;
        watch(pid, Some("steam"))
    }

    pub(super) fn watch(pid: u32, name: Option<&str>) -> Result<Watch, String> {
        let stat = PathBuf::from(format!("/proc/{pid}/stat"));
        let text = std::fs::read_to_string(&stat).map_err(|e| format!("process {pid}: {e}"))?;
        let (state, start_time, comm) = parse_stat(&text).ok_or_else(|| format!("process {pid}: unreadable stat"))?;
        if name.is_some_and(|n| comm != n) {
            return Err(format!("process {pid} is not {}", name.unwrap_or_default()));
        }
        if matches!(state, 'Z' | 'X' | 'x') {
            return Err(format!("process {pid} has already ended"));
        }
        Ok(Watch { stat, start_time })
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub(super) struct Watch;

    impl Watch {
        pub(super) fn has_ended(&self) -> bool {
            false
        }
    }

    pub(super) fn find() -> Result<Watch, String> {
        Err("no Steam process check on this operating system".into())
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// A child process that lives until killed.
    fn sleeper() -> std::process::Child {
        #[cfg(windows)]
        let mut cmd = std::process::Command::new("ping");
        #[cfg(windows)]
        cmd.args(["-n", "120", "127.0.0.1"]);
        #[cfg(not(windows))]
        let mut cmd = std::process::Command::new("sleep");
        #[cfg(not(windows))]
        cmd.arg("120");
        cmd.stdout(std::process::Stdio::null()).spawn().expect("spawn a child process")
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_killed_process_is_seen_as_ended_at_once() {
        let mut child = sleeper();
        let watch = SteamProcess::for_pid(child.id(), None).expect("watch the child");
        assert!(!watch.has_ended());
        child.kill().expect("kill");
        child.wait().expect("reap");
        assert!(watch.has_ended(), "the dead pid is seen without any Steam call");
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_process_with_another_name_is_not_watched() {
        let mut child = sleeper();
        assert!(SteamProcess::for_pid(child.id(), Some("steam-is-not-this")).is_err());
        child.kill().ok();
        child.wait().ok();
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_check_is_cheap() {
        let mut child = sleeper();
        let watch = SteamProcess::for_pid(child.id(), None).expect("watch the child");
        let n = 10_000u32;
        let start = Instant::now();
        for _ in 0..n {
            assert!(!watch.has_ended());
        }
        let each = start.elapsed() / n;
        println!("SteamProcess::has_ended: {each:?} per check (debug build)");
        child.kill().ok();
        child.wait().ok();
        assert!(each < Duration::from_millis(1), "{each:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_is_parsed() {
        let line = "4242 (steam) S 1 4242 4242 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 30 0 987654 1000 100 18446744073709551615";
        assert_eq!(imp::parse_stat(line), Some(('S', 987_654, "steam")));
        let odd = "7 (a b) c) Z 1 7 7 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 55 0 0";
        assert_eq!(imp::parse_stat(odd), Some(('Z', 55, "a b) c")));
        assert_eq!(imp::parse_stat("garbage"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_zombie_another_start_time_or_a_missing_stat_counts_as_ended() {
        let dir = std::env::temp_dir().join(format!("bevy_steam_kit_stat_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let stat = dir.join("stat");
        let watch = SteamProcess(imp::Watch { stat: stat.clone(), start_time: 55 });
        let line = |state: char, start: u64| format!("7 (steam) {state} 1 7 7 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0");
        std::fs::write(&stat, line('S', 55)).unwrap();
        assert!(!watch.has_ended(), "same process, running");
        std::fs::write(&stat, line('Z', 55)).unwrap();
        assert!(watch.has_ended(), "zombie");
        std::fs::write(&stat, line('S', 56)).unwrap();
        assert!(watch.has_ended(), "pid reused (another start time)");
        std::fs::remove_file(&stat).unwrap();
        assert!(watch.has_ended(), "gone");
        std::fs::remove_dir_all(&dir).ok();
    }
}
