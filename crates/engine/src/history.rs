//! The history of a project (commands.md 9.1, architecture 12.5): one per project, held by its session, in memory. A step
//! has its author, its label, its time and, for each file it touched, the bytes before and after (`None` for a file made
//! or removed), shared from one step to the next. It is not kept on disk: closing the project ends it, and git keeps the
//! rest.

use std::path::PathBuf;
use std::sync::Arc;

/// At most this many steps *(proposed)*: the oldest go.
pub const MAX_STEPS: usize = 1_000;
/// At most this many bytes of texts *(proposed)*: the oldest steps go.
pub const MAX_BYTES: usize = 64 << 20;

/// Who made a step (commands.md 9.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Author {
    /// The window of the IDE, or a caller with no other name.
    User,
    /// An MCP client, by its name: two clients are two authors.
    Agent(String),
    /// A file that changed outside LabCraft.
    External,
}

impl Author {
    pub fn name(&self) -> &str {
        match self {
            Author::User => "user",
            Author::Agent(name) => name,
            Author::External => "external",
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Author::Agent(_))
    }
}

/// What a step did to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    /// The path as commands show it, from the root.
    pub shown: String,
    pub before: Option<Arc<[u8]>>,
    pub after: Option<Arc<[u8]>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub number: u64,
    pub author: Author,
    pub label: String,
    /// UTC, in nanoseconds since 1970.
    pub time_ns: i64,
    pub changes: Vec<Change>,
}

impl Step {
    fn bytes(&self) -> usize {
        self.changes
            .iter()
            .map(|change| change.before.as_ref().map_or(0, |bytes| bytes.len()) + change.after.as_ref().map_or(0, |bytes| bytes.len()))
            .sum()
    }
}

#[derive(Debug, Default)]
pub struct History {
    done: Vec<Step>,
    /// What undo took back, the last undone at the end, each with who undid it: an agent redoes only what it undid (9.5).
    undone: Vec<(Step, Author)>,
    /// The number of the last step made.
    last: u64,
}

impl History {
    /// A new step. It drops what redo would replay. Its number.
    pub fn record(&mut self, author: Author, label: String, time_ns: i64, changes: Vec<Change>) -> u64 {
        self.undone.clear();
        self.last += 1;
        self.done.push(Step { number: self.last, author, label, time_ns, changes });
        self.trim();
        self.last
    }

    fn trim(&mut self) {
        let mut bytes: usize = self.done.iter().map(Step::bytes).sum::<usize>() + self.undone.iter().map(|(step, _)| step.bytes()).sum::<usize>();
        while (self.done.len() + self.undone.len() > MAX_STEPS || bytes > MAX_BYTES) && !self.done.is_empty() {
            let oldest = self.done.remove(0);
            bytes = bytes.saturating_sub(oldest.bytes());
        }
    }

    /// The step that undo would take back.
    pub fn to_undo(&self) -> Option<&Step> {
        self.done.last()
    }

    /// The step that redo would replay, and who undid it.
    pub fn to_redo(&self) -> Option<&(Step, Author)> {
        self.undone.last()
    }

    /// Moves the last step to what redo replays.
    pub fn undo(&mut self, by: Author) -> Option<&Step> {
        let step = self.done.pop()?;
        self.undone.push((step, by));
        self.undone.last().map(|(step, _)| step)
    }

    /// Moves the last step undone back.
    pub fn redo(&mut self) -> Option<&Step> {
        let (step, _) = self.undone.pop()?;
        self.done.push(step);
        self.done.last()
    }

    /// The steps newest first, those that redo would replay first of all.
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.undone.iter().map(|(step, _)| step).chain(self.done.iter().rev())
    }

    pub fn len(&self) -> usize {
        self.done.len() + self.undone.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(before: &str, after: &str) -> Vec<Change> {
        vec![Change {
            path: PathBuf::from("f.lcf"),
            shown: "f.lcf".to_owned(),
            before: Some(Arc::from(before.as_bytes())),
            after: Some(Arc::from(after.as_bytes())),
        }]
    }

    #[test]
    fn undo_and_redo_move_steps_and_a_new_step_drops_redo() {
        let mut history = History::default();
        assert_eq!(history.record(Author::User, "a".to_owned(), 0, change("0", "1")), 1);
        assert_eq!(history.record(Author::Agent("claude".to_owned()), "b".to_owned(), 0, change("1", "2")), 2);
        assert_eq!(history.undo(Author::User).map(|step| step.number), Some(2));
        assert_eq!(history.to_redo().map(|(step, by)| (step.number, by.clone())), Some((2, Author::User)));
        assert_eq!(history.steps().map(|step| step.number).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(history.redo().map(|step| step.number), Some(2));
        history.undo(Author::User);
        assert_eq!(history.record(Author::User, "c".to_owned(), 0, change("1", "3")), 3, "numbers are never given twice");
        assert!(history.to_redo().is_none());
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn the_oldest_steps_go_past_the_bounds() {
        let mut history = History::default();
        for index in 0..MAX_STEPS + 5 {
            history.record(Author::User, index.to_string(), 0, change("x", "y"));
        }
        assert_eq!(history.len(), MAX_STEPS);
        assert_eq!(history.steps().last().map(|step| step.number), Some(6));
        let big = "x".repeat(MAX_BYTES / 3);
        let mut history = History::default();
        for _ in 0..3 {
            history.record(Author::User, "big".to_owned(), 0, change(&big, &big));
        }
        assert_eq!(history.len(), 1);
    }
}
