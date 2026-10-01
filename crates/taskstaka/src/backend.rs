//! Where the CLI's commands go. `Direct` runs the store in-process (holding
//! the exclusive lock for the duration); `Http` talks to the daemon. Rendering
//! code never knows which one it has.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use jiff::Zoned;
use taskstaka_core::view::tree_from_views;
use taskstaka_core::{Command, Outcome, Store, StoreError, TaskId, Tree};

use crate::client::Client;

pub trait Backend {
    fn dir(&self) -> &Path;
    /// A snapshot of every task, closed ones included.
    fn tree(&mut self) -> Result<Tree>;
    fn resolve(&mut self, prefix: &str) -> Result<TaskId>;
    fn apply(&mut self, cmd: Command) -> Result<Outcome>;
    fn path_of(&mut self, id: &TaskId) -> Result<PathBuf>;
    /// Re-read every note from disk (after an external edit).
    fn reload(&mut self) -> Result<()>;
    fn reindex(&mut self) -> Result<()>;
    fn warnings(&mut self) -> Vec<String>;
    fn describe(&self) -> String;
}

pub struct Direct {
    store: Store,
}

impl Direct {
    pub fn open(dir: &Path) -> Result<Direct> {
        match Store::open(dir) {
            Ok(store) => Ok(Direct { store }),
            Err(StoreError::Locked(_)) => bail!(
                "{} is locked by another taskstaka process (the daemon?). Drop --direct, or stop it with `taskstaka stop`.",
                dir.display()
            ),
            Err(e) => Err(e.into()),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }
}

impl Backend for Direct {
    fn dir(&self) -> &Path {
        self.store.dir()
    }

    fn tree(&mut self) -> Result<Tree> {
        Ok(self.store.tree().clone())
    }

    fn resolve(&mut self, prefix: &str) -> Result<TaskId> {
        Ok(self.store.resolve(prefix)?)
    }

    fn apply(&mut self, cmd: Command) -> Result<Outcome> {
        Ok(self.store.apply(cmd, &Zoned::now())?)
    }

    fn path_of(&mut self, id: &TaskId) -> Result<PathBuf> {
        self.store
            .path_of(id)
            .map(Path::to_path_buf)
            .ok_or_else(|| StoreError::NotFound(id.to_string()).into())
    }

    fn reload(&mut self) -> Result<()> {
        self.store.reload()?;
        self.store.reindex(&Zoned::now())?;
        Ok(())
    }

    fn reindex(&mut self) -> Result<()> {
        Ok(self.store.reindex(&Zoned::now())?)
    }

    fn warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.load_warnings)
    }

    fn describe(&self) -> String {
        format!("direct ({})", self.store.dir().display())
    }
}

pub struct Http {
    client: Client,
    dir: PathBuf,
}

impl Http {
    pub fn connect(dir: &Path, port_hint: Option<u16>) -> Result<Http> {
        let client = Client::connect(dir, port_hint)?;
        Ok(Http {
            client,
            dir: dir.to_path_buf(),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

impl Backend for Http {
    fn dir(&self) -> &Path {
        &self.dir
    }

    fn tree(&mut self) -> Result<Tree> {
        Ok(tree_from_views(self.client.tasks(true)?))
    }

    fn resolve(&mut self, prefix: &str) -> Result<TaskId> {
        self.client.resolve(prefix)
    }

    fn apply(&mut self, cmd: Command) -> Result<Outcome> {
        self.client.apply(&cmd)
    }

    fn path_of(&mut self, id: &TaskId) -> Result<PathBuf> {
        self.client
            .task(id.as_str())?
            .path
            .ok_or_else(|| anyhow::anyhow!("daemon did not report a path for {id}"))
    }

    fn reload(&mut self) -> Result<()> {
        self.client.reload()
    }

    fn reindex(&mut self) -> Result<()> {
        self.client.reindex()
    }

    fn warnings(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn describe(&self) -> String {
        format!("daemon at {}", self.client.base())
    }
}
