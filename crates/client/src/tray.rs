//! The running menu bar item's control socket. `taskstaka tray quick` asks
//! `taskstaka-tray` to toggle its quick-add panel over it: that is how a
//! desktop keybinding reaches the item where no process can grab a key
//! itself (Wayland), and the same path works on every OS (ADR 0015). One
//! line in, one line out. Signals were tried first: WebKit's JavaScript
//! engine owns SIGUSR1 for its garbage collector.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

/// The one request today.
pub const TOGGLE: &str = "toggle";

/// Where the item listens: the session's runtime dir, else a per-user file
/// in the temp dir.
pub fn socket_path() -> PathBuf {
    socket_path_in(
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        &std::env::var("USER").unwrap_or_else(|_| "user".into()),
        &std::env::temp_dir(),
    )
}

pub fn socket_path_in(runtime_dir: Option<&OsStr>, user: &str, tmp: &std::path::Path) -> PathBuf {
    match runtime_dir.filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("taskstaka-tray.sock"),
        None => tmp.join(format!("taskstaka-tray-{user}.sock")),
    }
}

/// Send one request; `Ok(reply)` when an item answered.
#[cfg(unix)]
pub fn send(request: &str) -> std::io::Result<String> {
    use std::os::unix::net::UnixStream;
    let mut s = UnixStream::connect(socket_path())?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(s, "{request}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

#[cfg(not(unix))]
pub fn send(_request: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the tray socket needs a Unix system",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn socket_lives_in_the_runtime_dir_else_per_user_in_tmp() {
        assert_eq!(
            socket_path_in(Some(OsStr::new("/run/user/1000")), "adw", Path::new("/tmp")),
            PathBuf::from("/run/user/1000/taskstaka-tray.sock")
        );
        assert_eq!(
            socket_path_in(Some(OsStr::new("")), "adw", Path::new("/tmp")),
            PathBuf::from("/tmp/taskstaka-tray-adw.sock")
        );
        assert_eq!(
            socket_path_in(None, "adw", Path::new("/var/folders/x")),
            PathBuf::from("/var/folders/x/taskstaka-tray-adw.sock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn nobody_listening_is_an_error_not_a_hang() {
        // Point the socket at an empty temp dir: connect fails fast.
        let tmp = tempfile::tempdir().unwrap();
        let path = socket_path_in(Some(tmp.path().as_os_str()), "t", tmp.path());
        assert!(std::os::unix::net::UnixStream::connect(path).is_err());
    }
}
