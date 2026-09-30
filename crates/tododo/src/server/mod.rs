//! The daemon: one `Store` behind a mutex, an axum router, a revision
//! counter for long-polling, and the lifecycle around `daemon.json`.

pub mod error;
pub mod nudge;
pub mod outline;
pub mod routes;
pub mod web;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use axum::Router;
use jiff::Timestamp;
use tododo_core::nudge::{Nudge, NudgeConfig};
use tododo_core::{Store, StoreError};
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};
use tracing::info;

use crate::VERSION;
use crate::client::DaemonInfo;
use nudge::{DesktopNotifier, LogNotifier, Notifier, Nudger};

pub struct AppState {
    pub store: Mutex<Store>,
    pub rev: watch::Sender<u64>,
    pub shutdown: Notify,
    pub dir: PathBuf,
    pub started: Timestamp,
    /// The current nudge set, republished on every evaluation.
    pub nudges: Mutex<Vec<Nudge>>,
    /// Set once serving starts; routes use it for event-driven nudges.
    pub nudger: Mutex<Option<Arc<Nudger>>>,
}

impl AppState {
    pub fn open(dir: &Path) -> Result<Arc<AppState>, StoreError> {
        let store = Store::open(dir)?;
        let (rev, _) = watch::channel(1);
        Ok(Arc::new(AppState {
            store: Mutex::new(store),
            rev,
            shutdown: Notify::new(),
            dir: dir.to_path_buf(),
            started: Timestamp::now(),
            nudges: Mutex::new(Vec::new()),
            nudger: Mutex::new(None),
        }))
    }

    pub fn nudger(&self) -> Option<Arc<Nudger>> {
        self.nudger.lock().ok().and_then(|g| g.clone())
    }

    pub fn bump(&self) {
        self.rev.send_modify(|r| *r += 1);
    }

    pub fn current_rev(&self) -> u64 {
        *self.rev.borrow()
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    routes::router(state)
}

pub struct ServeOpts {
    pub dir: PathBuf,
    pub port: u16,
    /// Send desktop notifications (false: log only).
    pub notify: bool,
    pub quiet_hours: Option<(u8, u8)>,
}

/// A daemon bound and ready to serve; `run` and tests drive it.
pub struct Bound {
    pub state: Arc<AppState>,
    pub addr: SocketAddr,
    listener: TcpListener,
    nudger: Option<Arc<Nudger>>,
}

impl Bound {
    /// Take the lock, load the store, bind the port. Does not serve yet.
    pub async fn bind(dir: &Path, port: u16) -> Result<Bound> {
        // A predecessor may still be releasing the lock (restart, or a stop
        // that has closed its port but not yet exited): give it a moment.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let state = loop {
            match AppState::open(dir) {
                Ok(s) => break s,
                Err(StoreError::Locked(p)) if std::time::Instant::now() < deadline => {
                    tracing::debug!("lock held by another process at {}; retrying", p.display());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(StoreError::Locked(p)) => {
                    return Err(anyhow::anyhow!(
                        "another daemon already holds {}",
                        p.display()
                    ));
                }
                Err(other) => return Err(anyhow::Error::from(other)),
            }
        };
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .with_context(|| format!("binding 127.0.0.1:{port}"))?;
        let addr = listener.local_addr()?;
        Ok(Bound {
            state,
            addr,
            listener,
            nudger: None,
        })
    }

    /// Configure nudge delivery. Without this, nudges are only logged.
    pub fn with_nudger(mut self, cfg: NudgeConfig, notifier: Box<dyn Notifier>) -> Self {
        self.nudger = Some(Arc::new(Nudger::new(&self.state, cfg, notifier)));
        self
    }

    /// Serve until a shutdown request or signal. Writes and removes daemon.json.
    pub async fn serve(self) -> Result<()> {
        let Bound {
            state,
            addr,
            listener,
            nudger,
        } = self;
        let info = DaemonInfo {
            pid: std::process::id(),
            port: addr.port(),
            version: VERSION.to_string(),
            started: state.started.to_string(),
        };
        info.write(&state.dir).context("writing daemon.json")?;
        info!(%addr, dir = %state.dir.display(), "tododo daemon listening");

        let nudger = nudger.unwrap_or_else(|| {
            Arc::new(Nudger::new(
                &state,
                NudgeConfig::default(),
                Box::new(LogNotifier),
            ))
        });
        if let Ok(mut slot) = state.nudger.lock() {
            *slot = Some(nudger.clone());
        }
        nudge::spawn_scheduler(state.clone(), nudger, Duration::from_secs(300));

        let app = router(state.clone());
        let shutdown_state = state.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let ctrl_c = tokio::signal::ctrl_c();
                #[cfg(unix)]
                let term = async {
                    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    {
                        Ok(mut s) => {
                            s.recv().await;
                        }
                        Err(_) => std::future::pending::<()>().await,
                    }
                };
                #[cfg(not(unix))]
                let term = std::future::pending::<()>();
                tokio::select! {
                    _ = ctrl_c => info!("ctrl-c"),
                    _ = term => info!("SIGTERM"),
                    _ = shutdown_state.shutdown.notified() => info!("shutdown requested"),
                }
            })
            .await?;
        DaemonInfo::remove(&state.dir);
        info!("daemon stopped");
        Ok(())
    }
}

/// Foreground entry point for `tododo serve`.
pub fn run(opts: ServeOpts) -> Result<()> {
    init_tracing();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    rt.block_on(async move {
        let bound = match Bound::bind(&opts.dir, opts.port).await {
            Ok(b) => b,
            Err(e) if e.to_string().starts_with("another daemon") => {
                eprintln!("{e}; nothing to do");
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let cfg = NudgeConfig {
            quiet_hours: opts.quiet_hours,
            ..NudgeConfig::default()
        };
        let notifier: Box<dyn Notifier> = if opts.notify {
            Box::new(DesktopNotifier)
        } else {
            Box::new(LogNotifier)
        };
        bound.with_nudger(cfg, notifier).serve().await
    })
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}
