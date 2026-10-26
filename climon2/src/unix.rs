use crate::{absolute, launch_error, nonempty, session::Config, Result};
use socket2::Socket;
use std::{env, fs::{self, File, OpenOptions}, io::{self, Read, Write}, os::unix::{fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt}, io::{AsRawFd, FromRawFd}, process::{CommandExt, ExitStatusExt}}, path::{Path, PathBuf}, process::{Child, Command, ExitStatus}};

pub fn launch_cli(mut command: Command, path: &Path) -> Result<i32> { Err(launch_error(path, command.exec())) }
pub fn prepare_daemon(command: &mut Command) {
    unsafe { command.pre_exec(|| { if libc::setsid() < 0 { return Err(io::Error::last_os_error()); } Ok(()) }); }
}
pub fn daemon_init() -> Result<()> {
    unsafe { libc::umask(0o077); }
    #[cfg(target_os = "linux")]
    unsafe { if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) < 0 { return Err(io::Error::last_os_error().to_string()); } }
    Ok(())
}
pub fn status(status: ExitStatus) -> i32 { status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)) }
pub fn runtime_dir() -> Result<PathBuf> {
    let path = match nonempty("XDG_RUNTIME_DIR") {
        Some(path) => absolute(path)?,
        None => PathBuf::from(format!("/tmp/climon2-{}", unsafe { libc::getuid() })),
    };
    match fs::DirBuilder::new().recursive(true).mode(0o700).create(&path) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(format!("runtime directory {}: {e}", path.display())),
    }
    let info = fs::metadata(&path).map_err(|e| format!("runtime directory {}: {e}", path.display()))?;
    if !info.is_dir() || info.uid() != unsafe { libc::getuid() } || info.mode() & 0o7777 != 0o700 {
        return Err(format!("unsafe runtime directory: {}", path.display()));
    }
    Ok(path)
}
pub fn private_socket(path: &Path) -> Result<()> { fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| format!("socket path {}: {e}", path.display())) }
pub fn private_file(path: &Path) -> io::Result<File> { OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) }
pub fn peer(socket: &Socket) -> bool {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
        libc::getsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut size) == 0 && cred.uid == libc::getuid()
    }
    #[cfg(not(target_os = "linux"))]
    unsafe {
        let (mut uid, mut gid) = (0, 0);
        libc::getpeereid(socket.as_raw_fd(), &mut uid, &mut gid) == 0 && uid == libc::getuid()
    }
}
pub fn verify_transport() -> Result<()> { Ok(()) }
pub fn check_executable(path: &Path) -> Result<()> {
    let meta = fs::metadata(path).map_err(|e| launch_error(path, e))?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 { return Err(launch_error(path, "not executable")); }
    Ok(())
}
pub fn resolve(command: &str, _args: &[String]) -> Result<PathBuf> {
    if command.contains('/') { return absolute(command); }
    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            let candidate = directory.join(command);
            if check_executable(&candidate).is_ok() { return absolute(candidate); }
        }
    }
    Err(launch_error(command, "not found in PATH"))
}
pub struct TerminalGuard { saved: libc::termios }
impl TerminalGuard {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut saved = std::mem::zeroed();
            if libc::tcgetattr(0, &mut saved) != 0 { return Err(io::Error::last_os_error().to_string()); }
            Ok(Self { saved })
        }
    }
    pub fn raw(&self) -> Result<()> {
        unsafe {
            let mut raw = self.saved;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 { return Err(io::Error::last_os_error().to_string()); }
        }
        Ok(())
    }
}
impl Drop for TerminalGuard { fn drop(&mut self) { unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.saved); } } }
pub fn size() -> (u16, u16) {
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(0, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0 && size.ws_row > 0 { (size.ws_col, size.ws_row) } else { (80, 24) }
    }
}
pub fn process_start() -> Result<String> {
    #[cfg(target_os = "linux")]
    let instant = {
        let stat = fs::read_to_string("/proc/self/stat").map_err(|e| e.to_string())?;
        let ticks: u64 = stat.rsplit_once(')').ok_or("cannot read process start time")?.1.split_whitespace().nth(19).ok_or("cannot read process start time")?.parse().map_err(|_| "invalid process start time")?;
        let stat = fs::read_to_string("/proc/stat").map_err(|e| e.to_string())?;
        let boot: i64 = stat.lines().find_map(|line| line.strip_prefix("btime ")).ok_or("cannot read boot time")?.parse().map_err(|_| "invalid boot time")?;
        let frequency = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if frequency <= 0 { return Err("cannot read clock frequency".into()); }
        chrono::DateTime::from_timestamp(boot + (ticks / frequency as u64) as i64, ((ticks % frequency as u64) * 1_000_000_000 / frequency as u64) as u32).ok_or("invalid process start time")?
    };
    #[cfg(target_os = "macos")]
    let instant = {
        #[repr(C)]
        struct BsdInfo { fields: [u32; 12], comm: [u8; 16], name: [u8; 32], details: [u32; 6], seconds: u64, micros: u64 }
        #[link(name = "proc")]
        extern "C" { fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut libc::c_void, size: i32) -> i32; }
        let mut info: BsdInfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<BsdInfo>() as i32;
        if unsafe { proc_pidinfo(std::process::id() as i32, 3, 0, (&mut info as *mut BsdInfo).cast(), size) } != size { return Err("cannot read process start time".into()); }
        chrono::DateTime::from_timestamp(info.seconds as i64, info.micros as u32 * 1000).ok_or("invalid process start time")?
    };
    Ok(instant.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
}
pub fn local_stamp(timestamp: i64) -> Result<String> {
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let time = timestamp as libc::time_t;
        if libc::localtime_r(&time, &mut tm).is_null() { return Err("cannot convert local timestamp".into()); }
        Ok(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year+1900, tm.tm_mon+1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec))
    }
}
pub fn is_pty_eof(error: &io::Error) -> bool { error.raw_os_error() == Some(libc::EIO) }
pub fn relay_output(output: &mut dyn Write, bytes: &[u8], _pending: &mut Vec<u8>) -> io::Result<()> { output.write_all(bytes).and_then(|_| output.flush()) }
pub struct Pty {
    pub reader: Option<Box<dyn Read + Send>>,
    pub writer: Box<dyn Write + Send>,
    master: File,
    child: Child,
    pid: i32,
}
impl Pty {
    pub fn spawn(config: &Config) -> Result<Self> {
        unsafe {
            let (mut master, mut slave) = (-1, -1);
            let mut size = libc::winsize { ws_col: config.cols, ws_row: config.rows, ws_xpixel: 0, ws_ypixel: 0 };
            if libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 { return Err(io::Error::last_os_error().to_string()); }
            let master = File::from_raw_fd(master);
            let slave = File::from_raw_fd(slave);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            let mut command = Command::new(&config.target);
            command.args(&config.args).current_dir(&config.cwd)
                .env("CLIMON2_SESSION_ID", &config.session_id).env("CLIMON2_SESSION_DEPTH", config.depth.to_string());
            if env::var_os("TERM").is_none() { command.env("TERM", "xterm-256color"); }
            command.stdin(slave.try_clone().map_err(|e| e.to_string())?).stdout(slave.try_clone().map_err(|e| e.to_string())?).stderr(slave);
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 { return Err(io::Error::last_os_error()); }
                Ok(())
            });
            let child = command.spawn().map_err(|e| launch_error(&config.target, e))?;
            let pid = child.id() as i32;
            Ok(Self { reader: Some(Box::new(master.try_clone().map_err(|e| e.to_string())?)), writer: Box::new(master.try_clone().map_err(|e| e.to_string())?), master, child, pid })
        }
    }
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = libc::winsize { ws_col: cols, ws_row: rows, ws_xpixel: 0, ws_ypixel: 0 };
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
    pub fn try_wait(&mut self) -> io::Result<Option<i32>> { self.child.try_wait().map(|s| s.map(status)) }
    pub fn terminate(&self) { unsafe { libc::kill(-self.pid, libc::SIGKILL); } }
}
impl Drop for Pty {
    fn drop(&mut self) {
        self.terminate();
        let _ = self.child.wait();
        #[cfg(target_os = "linux")]
        unsafe { while libc::waitpid(-self.pid, std::ptr::null_mut(), 0) > 0 {} }
    }
}
