use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Done,
    Archived,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }
}

impl TryFrom<&str> for TaskStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open" => Ok(Self::Open),
            "done" => Ok(Self::Done),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct List {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl List {
    pub fn inbox() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: "Inbox".to_owned(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub list_id: Uuid,
    pub title: String,
    pub note_markdown: String,
    pub status: TaskStatus,
    pub tags: Vec<String>,
    pub due_date: Option<NaiveDate>,
    pub sort_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoSnapshot {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub lists: Vec<List>,
    pub tasks: Vec<Task>,
}

impl TodoSnapshot {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new(lists: Vec<List>, tasks: Vec<Task>) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            exported_at: Utc::now(),
            lists,
            tasks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub note_markdown: String,
    pub tags: Vec<String>,
    pub due_date: Option<NaiveDate>,
}

impl NewTask {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            note_markdown: String::new(),
            tags: Vec::new(),
            due_date: None,
        }
    }
}
