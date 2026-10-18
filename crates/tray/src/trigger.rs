//! The control socket: `taskstaka tray quick` connects, says `toggle`, and
//! the panel shows or hides. A thread accepts; requests are forwarded into
//! the event loop like every other input (see `taskstaka_client::tray`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

/// Removes the socket file when dropped (the Quit path; tao's `run` never
/// returns, so this is dropped by hand there). A stale file from a crash
/// is replaced on the next start.
pub struct Listener {
    path: PathBuf,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Start listening. `on_request` gets each line and says whether it was
/// understood; the caller answers `ok` or `unknown`.
pub fn listen(on_request: impl Fn(&str) -> bool + Send + 'static) -> std::io::Result<Listener> {
    let path = taskstaka_client::tray::socket_path();
    // One item per session: a stale or older listener's file goes.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let known = on_request(line.trim());
            let _ = writeln!(&stream, "{}", if known { "ok" } else { "unknown" });
        }
    });
    Ok(Listener { path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;
    use std::sync::mpsc;

    // `listen` binds the process-wide socket path; one test at a time.
    static ONE: Mutex<()> = Mutex::new(());

    #[test]
    fn a_toggle_request_reaches_the_callback_and_is_acknowledged() {
        let _guard = ONE.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded within the guarded section.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", tmp.path()) };
        let (tx, rx) = mpsc::channel();
        let l = listen(move |req| {
            tx.send(req.to_string()).unwrap();
            req == taskstaka_client::tray::TOGGLE
        })
        .unwrap();
        assert_eq!(
            taskstaka_client::tray::send("toggle").unwrap(),
            "ok",
            "the item answers"
        );
        assert_eq!(rx.recv().unwrap(), "toggle");
        assert_eq!(taskstaka_client::tray::send("dance").unwrap(), "unknown");
        let path = taskstaka_client::tray::socket_path();
        assert!(path.exists());
        drop(l);
        assert!(!path.exists(), "quit removes the socket file");
        assert!(UnixStream::connect(&path).is_err());
    }
}
