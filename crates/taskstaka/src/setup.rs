//! `taskstaka setup claude`: install the skill and register the MCP server so
//! every Claude Code session has the same hands (MCP) and habits (skill).
//! Idempotent: run it again after upgrading and both are refreshed.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

pub const SKILL_MD: &str = include_str!("../assets/skill/SKILL.md");
pub const SKILL_NAME: &str = "taskstaka";
pub const MCP_NAME: &str = "taskstaka";

pub struct ClaudeSetup {
    /// Install into the current project (`.claude/skills`, MCP scope `project`)
    /// instead of the user's home (`~/.claude/skills`, MCP scope `user`).
    pub project: bool,
    pub skip_skill: bool,
    pub skip_mcp: bool,
    /// Leave CLAUDE.md alone.
    pub skip_claude_md: bool,
    /// Print what would be done without touching anything.
    pub dry_run: bool,
}

pub struct Report {
    pub skill_path: Option<PathBuf>,
    pub claude_md_path: Option<PathBuf>,
    pub mcp_command: Option<Vec<String>>,
    pub mcp_registered: bool,
    pub notes: Vec<String>,
}

pub fn skill_dir(project: bool) -> Result<PathBuf> {
    let base = if project {
        std::env::current_dir().context("current directory")?
    } else {
        home_dir()?
    };
    Ok(base.join(".claude").join("skills").join(SKILL_NAME))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .context("HOME is not set; cannot find ~/.claude")
}

pub fn claude(opts: &ClaudeSetup) -> Result<Report> {
    let mut report = Report {
        skill_path: None,
        claude_md_path: None,
        mcp_command: None,
        mcp_registered: false,
        notes: Vec::new(),
    };

    if !opts.skip_skill {
        let dir = skill_dir(opts.project)?;
        let path = dir.join("SKILL.md");
        if !opts.dry_run {
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            std::fs::write(&path, SKILL_MD)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        report.skill_path = Some(path);
    }

    if !opts.skip_claude_md {
        // A skill description is advisory; a memory line is not. This is what
        // makes the skill load *before* Claude starts investigating.
        let path = if opts.project {
            std::env::current_dir()
                .context("current directory")?
                .join("CLAUDE.md")
        } else {
            home_dir()?.join(".claude").join("CLAUDE.md")
        };
        if !opts.dry_run {
            upsert_claude_md(&path)?;
        }
        report.claude_md_path = Some(path);
    }

    if !opts.skip_mcp {
        let exe = std::env::current_exe().context("locating the taskstaka binary")?;
        let scope = if opts.project { "project" } else { "user" };
        let add: Vec<String> = [
            "mcp",
            "add",
            "--scope",
            scope,
            "--transport",
            "stdio",
            MCP_NAME,
            "--",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([exe.display().to_string(), "mcp".to_string()])
        .collect();
        report.mcp_command = Some(
            std::iter::once("claude".to_string())
                .chain(add.iter().cloned())
                .collect(),
        );

        if opts.dry_run {
            // nothing
        } else if which("claude").is_none() {
            report.notes.push(
                "the `claude` CLI is not on PATH; run the command above once it is (or paste it into a terminal where it is)".into(),
            );
        } else {
            // Re-register so a moved/upgraded binary path is picked up.
            let exists = Command::new("claude")
                .args(["mcp", "get", MCP_NAME])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if exists {
                let rm = Command::new("claude")
                    .args(["mcp", "remove", "--scope", scope, MCP_NAME])
                    .output()
                    .context("running claude mcp remove")?;
                if !rm.status.success() {
                    // It may live in another scope; adding will then fail with a clear message.
                    report.notes.push(format!(
                        "could not remove the existing `{MCP_NAME}` MCP entry in scope {scope}: {}",
                        String::from_utf8_lossy(&rm.stderr).trim()
                    ));
                }
            }
            let out = Command::new("claude")
                .args(&add)
                .output()
                .context("running claude mcp add")?;
            if out.status.success() {
                report.mcp_registered = true;
            } else {
                bail!(
                    "`claude mcp add` failed:\n{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }

    Ok(report)
}

pub const CLAUDE_MD_BEGIN: &str = "<!-- taskstaka:begin -->";
pub const CLAUDE_MD_END: &str = "<!-- taskstaka:end -->";
pub const CLAUDE_MD_BLOCK: &str = "\
<!-- taskstaka:begin -->
## taskstaka

Before starting any piece of work with more than one step — and before investigating or
asking clarifying questions about it — load the `taskstaka` skill and record the plan
there: one subtask per step under a story, your own steps tagged `agent`, the user's
manual steps with exact instructions in the notes, steps that depend on a person already
waiting on them. Also use it when the user asks what to work on, what's next, or mentions
waiting on someone. Do not change the status of tasks the user parked or ordered without
asking.
<!-- taskstaka:end -->
";

/// Add or replace the managed block in CLAUDE.md, leaving everything else as is.
fn upsert_claude_md(path: &std::path::Path) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let updated = match (existing.find(CLAUDE_MD_BEGIN), existing.find(CLAUDE_MD_END)) {
        (Some(b), Some(e)) if e > b => {
            let end = e + CLAUDE_MD_END.len();
            let tail = existing[end..].trim_start_matches('\n');
            format!("{}{}{}", &existing[..b], CLAUDE_MD_BLOCK, tail)
        }
        _ => {
            let mut s = existing.clone();
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(CLAUDE_MD_BLOCK);
            s
        }
    };
    if updated != existing {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(bin))
        .find(|p| p.is_file())
}

/// Human summary for the terminal.
pub fn describe(r: &Report, opts: &ClaudeSetup) -> String {
    let mut s = String::new();
    let verb = if opts.dry_run {
        "would install"
    } else {
        "installed"
    };
    if let Some(p) = &r.skill_path {
        s.push_str(&format!("{verb} skill   {}\n", p.display()));
    }
    if let Some(p) = &r.claude_md_path {
        s.push_str(&format!(
            "{verb} memory  {}  (managed block; the rest of the file is untouched)\n",
            p.display()
        ));
    }
    if let Some(cmd) = &r.mcp_command {
        let status = if opts.dry_run {
            "would run"
        } else if r.mcp_registered {
            "registered"
        } else {
            "run this"
        };
        s.push_str(&format!("{status:<10} MCP     {}\n", shell_join(cmd)));
    }
    for n in &r.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    if !opts.dry_run {
        s.push_str("\nRestart Claude Code (or run /mcp) to pick these up. Then try: \"what should I work on?\"\n");
    }
    s
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| {
            if p.chars().any(|c| c.is_whitespace()) {
                format!("'{p}'")
            } else {
                p.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_has_frontmatter_and_names_the_tools() {
        assert!(SKILL_MD.starts_with("---\nname: taskstaka\ndescription: "));
        for tool in [
            "next",
            "waiting",
            "add_task",
            "set_status",
            "record_reply",
            "chase",
        ] {
            assert!(
                SKILL_MD.contains(&format!("`{tool}`")),
                "skill mentions {tool}"
            );
        }
    }

    #[test]
    fn writes_the_skill_under_home() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module are the only readers of HOME here.
        unsafe { std::env::set_var("HOME", home.path()) };
        let r = claude(&ClaudeSetup {
            project: false,
            skip_skill: false,
            skip_mcp: true,
            skip_claude_md: false,
            dry_run: false,
        })
        .unwrap();
        let p = r.skill_path.unwrap();
        assert_eq!(p, home.path().join(".claude/skills/taskstaka/SKILL.md"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), SKILL_MD);
        // Idempotent, and CLAUDE.md gets exactly one managed block that is
        // replaced in place while the user's own text survives.
        let md = home.path().join(".claude/CLAUDE.md");
        std::fs::write(&md, "# mine\n\nkeep this\n").unwrap();
        claude(&ClaudeSetup {
            project: false,
            skip_skill: false,
            skip_mcp: true,
            skip_claude_md: false,
            dry_run: false,
        })
        .unwrap();
        claude(&ClaudeSetup {
            project: false,
            skip_skill: false,
            skip_mcp: true,
            skip_claude_md: false,
            dry_run: false,
        })
        .unwrap();
        let text = std::fs::read_to_string(&md).unwrap();
        assert!(text.starts_with("# mine\n\nkeep this\n"), "{text}");
        assert_eq!(text.matches(CLAUDE_MD_BEGIN).count(), 1, "{text}");
        assert!(text.contains("load the `taskstaka` skill"));
    }
}
