//! Changing where the notes live. The daemon is bound to one directory, so a
//! change is: validate (`plan`), stop the daemon, copy (`apply`), save the
//! setting, start a daemon on the new directory. The old directory is never
//! touched beyond reading it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use taskstaka_core::Store;
use taskstaka_core::store::{META_DIR, NOTES_DIR};

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    /// The target is the directory already in use.
    Unchanged,
    /// The target already holds a taskstaka directory: use it as it is.
    UseExisting,
    /// The target is empty or new: copy the current notes into it.
    CopyInto,
}

#[derive(Debug)]
pub struct Plan {
    pub target: PathBuf,
    pub kind: Kind,
}

/// `~` and `~/x` mean the home directory; anything else must be absolute.
pub fn expand(input: &str) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        bail!("give a folder, for example /home/you/notes");
    }
    let path = if input == "~" || input.starts_with("~/") {
        let home = std::env::var_os("HOME").context("cannot find your home directory")?;
        PathBuf::from(home).join(input.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(input)
    };
    if !path.is_absolute() {
        bail!("use a full path such as /home/you/notes (or ~/notes), not {input:?}");
    }
    Ok(path)
}

/// Check a requested directory without changing anything.
pub fn plan(current: &Path, input: &str) -> Result<Plan> {
    let target = expand(input)?;
    let current = fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
    // The target may not exist yet: resolve its nearest existing ancestor so
    // `..` and symlinks cannot hide a nesting.
    let resolved = resolve(&target);
    if resolved == current {
        return Ok(Plan {
            target: resolved,
            kind: Kind::Unchanged,
        });
    }
    if resolved.starts_with(&current) || current.starts_with(&resolved) {
        bail!(
            "{} and the current folder {} contain one another; pick a separate folder",
            resolved.display(),
            current.display()
        );
    }
    let kind = if resolved.join(META_DIR).is_dir() {
        Kind::UseExisting
    } else if !resolved.exists() {
        Kind::CopyInto
    } else if !resolved.is_dir() {
        bail!("{} is a file, not a folder", resolved.display());
    } else if fs::read_dir(&resolved)
        .with_context(|| format!("reading {}", resolved.display()))?
        .next()
        .is_none()
    {
        Kind::CopyInto
    } else {
        bail!(
            "{} is not empty and is not a taskstaka folder; pick an empty or new folder",
            resolved.display()
        );
    };
    Ok(Plan {
        target: resolved,
        kind,
    })
}

fn resolve(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// Carry out a plan. Call it with the daemon stopped so the copy is the final
/// state. Returns how many notes were copied.
pub fn apply(current: &Path, plan: &Plan) -> Result<usize> {
    if plan.kind != Kind::CopyInto {
        return Ok(0);
    }
    fs::create_dir_all(&plan.target)
        .with_context(|| format!("creating {}", plan.target.display()))?;
    // Lays out notes/ and .taskstaka/; the lock is released on drop.
    drop(
        Store::init(&plan.target)
            .with_context(|| format!("setting up {}", plan.target.display()))?,
    );
    let mut copied = 0;
    let src_notes = current.join(NOTES_DIR);
    if src_notes.is_dir() {
        for entry in fs::read_dir(&src_notes)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                fs::copy(
                    entry.path(),
                    plan.target.join(NOTES_DIR).join(entry.file_name()),
                )
                .with_context(|| format!("copying {}", entry.path().display()))?;
                copied += 1;
            }
        }
    }
    // The index is regenerated on open; the cooldown log is kept so a move
    // does not re-send every reminder.
    for (from, to) in [
        (
            current.join(META_DIR).join("nudges.json"),
            plan.target.join(META_DIR).join("nudges.json"),
        ),
        (current.join("index.md"), plan.target.join("index.md")),
    ] {
        if from.is_file() {
            fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_notes(n: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        drop(Store::init(dir.path()).unwrap());
        for i in 0..n {
            fs::write(
                dir.path().join(NOTES_DIR).join(format!("aaaaa{i}-x.md")),
                "---\nid: x\n---\n",
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn a_new_folder_gets_a_copy_and_the_old_one_is_untouched() {
        let cur = store_with_notes(3);
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("fresh").join("notes");
        let plan = plan(cur.path(), target.to_str().unwrap()).unwrap();
        assert_eq!(plan.kind, Kind::CopyInto);
        assert_eq!(apply(cur.path(), &plan).unwrap(), 3);
        assert_eq!(fs::read_dir(target.join(NOTES_DIR)).unwrap().count(), 3);
        assert_eq!(fs::read_dir(cur.path().join(NOTES_DIR)).unwrap().count(), 3);
    }

    #[test]
    fn an_empty_existing_folder_is_copied_into() {
        let cur = store_with_notes(1);
        let target = tempfile::tempdir().unwrap();
        let plan = plan(cur.path(), target.path().to_str().unwrap()).unwrap();
        assert_eq!(plan.kind, Kind::CopyInto);
    }

    #[test]
    fn an_existing_taskstaka_folder_is_used_as_is() {
        let cur = store_with_notes(1);
        let other = store_with_notes(2);
        let plan = plan(cur.path(), other.path().to_str().unwrap()).unwrap();
        assert_eq!(plan.kind, Kind::UseExisting);
        assert_eq!(apply(cur.path(), &plan).unwrap(), 0);
        assert_eq!(
            fs::read_dir(other.path().join(NOTES_DIR)).unwrap().count(),
            2
        );
    }

    #[test]
    fn refuses_unrelated_nonempty_nested_and_relative_folders() {
        let cur = store_with_notes(1);
        let junk = tempfile::tempdir().unwrap();
        fs::write(junk.path().join("photo.jpg"), "x").unwrap();
        let msg = |input: &str| plan(cur.path(), input).unwrap_err().to_string();
        assert!(msg(junk.path().to_str().unwrap()).contains("not empty"));
        assert!(msg(cur.path().join("sub").to_str().unwrap()).contains("contain one another"));
        assert!(
            msg(cur.path().parent().unwrap().to_str().unwrap()).contains("contain one another")
        );
        assert!(msg("notes").contains("full path"));
        assert!(msg("   ").contains("give a folder"));
    }

    #[test]
    fn the_current_folder_is_unchanged() {
        let cur = store_with_notes(1);
        let p = plan(cur.path(), cur.path().to_str().unwrap()).unwrap();
        assert_eq!(p.kind, Kind::Unchanged);
    }
}
