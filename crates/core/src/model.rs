use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Crockford base32 alphabet, lowercase, without the ambiguous i/l/o/u.
pub const ID_ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
pub const ID_LEN: usize = 6;

/// A short random identifier. Six Crockford base32 characters give ~1e9
/// values, which is plenty for one person and short enough to type.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(String);

impl TaskId {
    pub fn random() -> Self {
        use rand::Rng;
        let mut rng = rand::rng();
        let s = (0..ID_LEN)
            .map(|_| ID_ALPHABET[rng.random_range(0..ID_ALPHABET.len())] as char)
            .collect();
        TaskId(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TaskId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("empty id".into());
        }
        if let Some(c) = s.chars().find(|c| !ID_ALPHABET.contains(&(*c as u8))) {
            return Err(format!("invalid character {c:?} in id {s:?}"));
        }
        Ok(TaskId(s.to_string()))
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TaskId({})", self.0)
    }
}

/// Stored status: what the *user* said about a task. Facts derived from the
/// tree (blocked, actionable, resurfaced) are never stored; see `action`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    /// Waiting on something external. `waiting_on` says what; `until` says
    /// when to chase it.
    Waiting,
    /// Deliberately parked. Excluded from ranking, does not block its parent.
    Someday,
    Done,
    Dropped,
}

impl Status {
    pub fn is_closed(self) -> bool {
        matches!(self, Status::Done | Status::Dropped)
    }

    pub fn is_open(self) -> bool {
        !self.is_closed()
    }

    /// Does a child in this status block its parent from being actionable?
    pub fn is_blocking(self) -> bool {
        matches!(self, Status::Todo | Status::Doing | Status::Waiting)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Waiting => "waiting",
            Status::Someday => "someday",
            Status::Done => "done",
            Status::Dropped => "dropped",
        }
    }
}

impl FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "todo" => Status::Todo,
            "doing" => Status::Doing,
            "waiting" => Status::Waiting,
            "someday" => Status::Someday,
            "done" => Status::Done,
            "dropped" => Status::Dropped,
            other => return Err(format!("unknown status {other:?}")),
        })
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Context {
    #[default]
    Work,
    Personal,
}

impl Context {
    pub fn as_str(self) -> &'static str {
        match self {
            Context::Work => "work",
            Context::Personal => "personal",
        }
    }
}

impl FromStr for Context {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "work" | "w" => Context::Work,
            "personal" | "p" => Context::Personal,
            other => return Err(format!("unknown context {other:?} (work|personal)")),
        })
    }
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Optional priority. Absent means normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Low,
}

impl FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "high" | "h" => Priority::High,
            "low" | "l" => Priority::Low,
            other => return Err(format!("unknown priority {other:?} (high|low)")),
        })
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Priority::High => "high",
            Priority::Low => "low",
        })
    }
}

/// One task. Field order here is the key order written to frontmatter, so
/// keep it stable for readable diffs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TaskId>,
    /// Position among siblings (fractional; see `order_key`). Absent means
    /// "by creation time", which is what every task had before ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<f64>,
    pub status: Status,
    #[serde(default)]
    pub context: Context,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    pub created: Timestamp,
    /// Last user-visible edit or explicit `touch`. Never bumped by derived
    /// changes — it is the "time since touched" signal.
    pub updated: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Date>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Markdown body below the frontmatter. Not part of the YAML.
    #[serde(skip)]
    pub body: String,
}

impl Task {
    pub fn new(id: TaskId, title: impl Into<String>, context: Context, now: Timestamp) -> Self {
        Task {
            id,
            title: title.into(),
            parent: None,
            order: None,
            status: Status::Todo,
            context,
            priority: None,
            created: now,
            updated: now,
            started: None,
            completed: None,
            due: None,
            waiting_on: None,
            until: None,
            tags: Vec::new(),
            body: String::new(),
        }
    }

    /// Sort key among siblings: explicit `order`, else creation time in
    /// seconds, so unordered and ordered tasks interleave sensibly.
    pub fn order_key(&self) -> f64 {
        self.order.unwrap_or(self.created.as_second() as f64)
    }

    /// Filename slug, frozen at creation so Obsidian links keep resolving
    /// after a rename. `title` in frontmatter is the truth.
    pub fn slug(&self) -> String {
        slugify(&self.title)
    }

    pub fn file_stem(&self) -> String {
        let slug = self.slug();
        if slug.is_empty() {
            self.id.to_string()
        } else {
            format!("{}-{}", self.id, slug)
        }
    }
}

pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true; // suppress leading dash
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collapses_and_caps() {
        assert_eq!(slugify("Fix JWT expiry!"), "fix-jwt-expiry");
        assert_eq!(slugify("  --hello--  "), "hello");
        assert_eq!(slugify(""), "");
        assert!(slugify(&"a".repeat(100)).len() <= 40);
    }

    #[test]
    fn id_roundtrip_and_validation() {
        let id = TaskId::random();
        assert_eq!(id.as_str().len(), ID_LEN);
        assert_eq!(id.as_str().parse::<TaskId>().unwrap(), id);
        assert!("ILOU".parse::<TaskId>().is_err());
        assert!("".parse::<TaskId>().is_err());
    }
}
