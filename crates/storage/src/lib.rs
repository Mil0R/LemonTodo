use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use lemontodo_core::{
    List, NewTask, ObjectType, Operation, OperationType, Task, TaskStatus, TodoSnapshot,
};
use lemontodo_crypto::EncryptedVaultKey;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use uuid::Uuid;

const TUI_CURRENT_PROJECT_KEY: &str = "tui.current_project";

pub struct TodoStore {
    conn: Connection,
}

impl TodoStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data directory {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        store.ensure_inbox()?;
        Ok(store)
    }

    pub fn add_task(&self, new_task: NewTask) -> Result<Task> {
        self.add_task_to_project(new_task, None)
    }

    pub fn add_task_to_project(
        &self,
        new_task: NewTask,
        project_name: Option<&str>,
    ) -> Result<Task> {
        let title = new_task.title.trim();
        if title.is_empty() {
            bail!("task title cannot be empty");
        }

        let list_id = match project_name {
            Some(project_name) => self.find_project_by_name(project_name)?.id,
            None => self.inbox_id()?,
        };
        let now = Utc::now();
        let task = Task {
            id: Uuid::new_v4(),
            list_id,
            revision: 1,
            title: title.to_owned(),
            note_markdown: new_task.note_markdown,
            status: TaskStatus::Open,
            tags: normalize_tags(new_task.tags),
            due_date: new_task.due_date,
            sort_key: format!("{:020}", now.timestamp_millis()),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };

        self.conn.execute(
            "INSERT INTO tasks (
                id, list_id, revision, title, note_markdown, status, tags, due_date,
                sort_key, created_at, updated_at, deleted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.id.to_string(),
                task.list_id.to_string(),
                task.revision,
                task.title,
                task.note_markdown,
                task.status.as_str(),
                serde_json::to_string(&task.tags)?,
                task.due_date.map(|date| date.to_string()),
                task.sort_key,
                task.created_at.to_rfc3339(),
                task.updated_at.to_rfc3339(),
                task.deleted_at.map(|date| date.to_rfc3339()),
            ],
        )?;

        self.record_operation(
            task.id,
            ObjectType::Task,
            OperationType::Create,
            json!({ "task": task }),
        )?;

        Ok(task)
    }

    pub fn create_project(&self, name: &str) -> Result<List> {
        let name = normalize_project_name(name)?;
        if let Some(existing) = self.find_list_by_name(&name)? {
            return Ok(existing);
        }

        let now = Utc::now();
        let project = List {
            id: Uuid::new_v4(),
            name,
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        self.conn.execute(
            "INSERT INTO lists (id, name, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project.id.to_string(),
                project.name,
                project.revision,
                project.created_at.to_rfc3339(),
                project.updated_at.to_rfc3339(),
            ],
        )?;
        self.record_operation(
            project.id,
            ObjectType::List,
            OperationType::Create,
            json!({ "list": project }),
        )?;

        Ok(project)
    }

    pub fn projects(&self) -> Result<Vec<List>> {
        self.list_lists()
    }

    pub fn export_snapshot(&self) -> Result<TodoSnapshot> {
        Ok(TodoSnapshot::new(
            self.list_lists()?,
            self.list_all_tasks()?,
        ))
    }

    pub fn import_snapshot(&mut self, snapshot: TodoSnapshot) -> Result<()> {
        if snapshot.version != TodoSnapshot::CURRENT_VERSION {
            bail!(
                "unsupported snapshot version {}, expected {}",
                snapshot.version,
                TodoSnapshot::CURRENT_VERSION
            );
        }

        let tx = self.conn.transaction()?;
        let mut list_id_map = HashMap::new();
        for list in snapshot.lists {
            let existing_id = tx
                .query_row(
                    "SELECT id FROM lists WHERE name = ?1",
                    params![list.name],
                    |row| parse_uuid(row.get::<_, String>(0)?),
                )
                .optional()?;

            let target_id = if let Some(existing_id) = existing_id {
                tx.execute(
                    "UPDATE lists SET revision = max(revision, ?1), updated_at = ?2 WHERE id = ?3",
                    params![
                        list.revision,
                        list.updated_at.to_rfc3339(),
                        existing_id.to_string()
                    ],
                )?;
                existing_id
            } else {
                tx.execute(
                    "INSERT INTO lists (id, name, revision, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        list.id.to_string(),
                        list.name,
                        list.revision,
                        list.created_at.to_rfc3339(),
                        list.updated_at.to_rfc3339(),
                    ],
                )?;
                list.id
            };
            list_id_map.insert(list.id, target_id);
        }

        for task in snapshot.tasks {
            let list_id = list_id_map
                .get(&task.list_id)
                .copied()
                .unwrap_or(task.list_id);
            tx.execute(
                "INSERT INTO tasks (
                    id, list_id, revision, title, note_markdown, status, tags, due_date,
                    sort_key, created_at, updated_at, deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(id) DO UPDATE SET
                    list_id = excluded.list_id,
                    revision = excluded.revision,
                    title = excluded.title,
                    note_markdown = excluded.note_markdown,
                    status = excluded.status,
                    tags = excluded.tags,
                    due_date = excluded.due_date,
                    sort_key = excluded.sort_key,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at,
                    deleted_at = excluded.deleted_at",
                params![
                    task.id.to_string(),
                    list_id.to_string(),
                    task.revision,
                    task.title,
                    task.note_markdown,
                    task.status.as_str(),
                    serde_json::to_string(&task.tags)?,
                    task.due_date.map(|date| date.to_string()),
                    task.sort_key,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                    task.deleted_at.map(|date| date.to_rfc3339()),
                ],
            )?;
        }

        tx.commit()?;
        self.ensure_inbox()?;
        self.record_operation(
            Uuid::new_v4(),
            ObjectType::Snapshot,
            OperationType::ImportSnapshot,
            json!({ "imported_at": Utc::now() }),
        )?;
        Ok(())
    }

    pub fn pending_operations(&self) -> Result<Vec<Operation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, object_id, object_revision, object_type, operation_type, payload, created_at, synced_at
             FROM operations
             WHERE synced_at IS NULL
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_operation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list pending operations")
    }

    pub fn save_encrypted_vault_key(&self, encrypted_vault_key: &EncryptedVaultKey) -> Result<()> {
        self.set_sync_state(
            "encrypted_vault_key",
            &serde_json::to_string(encrypted_vault_key)?,
        )
    }

    pub fn encrypted_vault_key(&self) -> Result<Option<EncryptedVaultKey>> {
        self.get_sync_state("encrypted_vault_key")?
            .map(|value| {
                serde_json::from_str(&value).context("failed to parse encrypted vault key")
            })
            .transpose()
    }

    pub fn save_tui_current_project(&self, project_name: &str) -> Result<()> {
        self.set_sync_state(TUI_CURRENT_PROJECT_KEY, project_name)
    }

    pub fn tui_current_project(&self) -> Result<Option<String>> {
        self.get_sync_state(TUI_CURRENT_PROJECT_KEY)
    }

    pub fn list_tasks(&self, include_done: bool) -> Result<Vec<Task>> {
        self.list_tasks_for_project(include_done, None)
    }

    pub fn list_tasks_for_project(
        &self,
        include_done: bool,
        project_name: Option<&str>,
    ) -> Result<Vec<Task>> {
        let status_filter = if include_done {
            "deleted_at IS NULL AND status != 'archived'"
        } else {
            "deleted_at IS NULL AND status = 'open'"
        };

        let Some(project_name) = project_name else {
            let sql = format!(
                "{} WHERE {} ORDER BY status = 'done', sort_key ASC",
                select_task_sql(),
                status_filter
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], row_to_task)?;
            return rows
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("failed to list tasks");
        };

        let project = self.find_project_by_name(project_name)?;
        let sql = format!(
            "{} WHERE {} AND list_id = ?1 ORDER BY status = 'done', sort_key ASC",
            select_task_sql(),
            status_filter
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project.id.to_string()], row_to_task)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list tasks")
    }

    fn list_lists(&self) -> Result<Vec<List>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, revision, created_at, updated_at FROM lists ORDER BY name ASC",
        )?;
        let rows = stmt.query_map([], row_to_list)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list lists")
    }

    fn list_all_tasks(&self) -> Result<Vec<Task>> {
        let sql = format!(
            "{} WHERE deleted_at IS NULL ORDER BY status = 'done', sort_key ASC",
            select_task_sql()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_task)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list all tasks")
    }

    pub fn search_tasks(&self, query: &str) -> Result<Vec<Task>> {
        let query = query.trim();
        if query.is_empty() {
            return self.list_tasks(true);
        }

        let pattern = format!("%{}%", query.to_lowercase());
        let sql = format!(
            "{} WHERE deleted_at IS NULL
                AND status != 'archived'
                AND (
                    lower(title) LIKE ?1
                    OR lower(note_markdown) LIKE ?1
                    OR lower(tags) LIKE ?1
                )
             ORDER BY status = 'done', sort_key ASC",
            select_task_sql()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern], row_to_task)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to search tasks")
    }

    pub fn move_task_to_project(&self, id_prefix: &str, project_name: &str) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        let project = self.find_project_by_name(project_name)?;
        self.move_task_to_project_by_id(task.id, project.id)
    }

    pub fn move_task_to_project_by_id(&self, id: Uuid, list_id: Uuid) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET list_id = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                list_id.to_string(),
                revision,
                now.to_rfc3339(),
                task.id.to_string()
            ],
        )?;

        let updated = Task {
            list_id,
            revision,
            updated_at: now,
            ..task
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            OperationType::Update,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    pub fn mark_done(&self, id_prefix: &str) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.set_task_status(task, TaskStatus::Done)
    }

    pub fn archive_task(&self, id_prefix: &str) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.set_task_status(task, TaskStatus::Archived)
    }

    pub fn archive_task_by_id(&self, id: Uuid) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        self.set_task_status(task, TaskStatus::Archived)
    }

    pub fn toggle_done(&self, id: Uuid) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        let status = match task.status {
            TaskStatus::Open => TaskStatus::Done,
            TaskStatus::Done | TaskStatus::Archived => TaskStatus::Open,
        };
        self.set_task_status(task, status)
    }

    pub fn update_task_title(&self, id_prefix: &str, title: &str) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.update_task_title_by_id(task.id, title)
    }

    pub fn update_task_title_by_id(&self, id: Uuid, title: &str) -> Result<Task> {
        let title = title.trim();
        if title.is_empty() {
            bail!("task title cannot be empty");
        }

        let task = self.find_task_by_id(id)?;
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET title = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![title, revision, now.to_rfc3339(), task.id.to_string()],
        )?;

        let updated = Task {
            title: title.to_owned(),
            revision,
            updated_at: now,
            ..task
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            OperationType::Update,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    pub fn update_task_note(&self, id_prefix: &str, note_markdown: &str) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.update_task_note_by_id(task.id, note_markdown)
    }

    pub fn update_task_note_by_id(&self, id: Uuid, note_markdown: &str) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET note_markdown = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                note_markdown,
                revision,
                now.to_rfc3339(),
                task.id.to_string()
            ],
        )?;

        let updated = Task {
            note_markdown: note_markdown.to_owned(),
            revision,
            updated_at: now,
            ..task
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            OperationType::Update,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    pub fn update_task_due_date(
        &self,
        id_prefix: &str,
        due_date: Option<NaiveDate>,
    ) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.update_task_due_date_by_id(task.id, due_date)
    }

    pub fn update_task_due_date_by_id(
        &self,
        id: Uuid,
        due_date: Option<NaiveDate>,
    ) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET due_date = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                due_date.map(|date| date.to_string()),
                revision,
                now.to_rfc3339(),
                task.id.to_string()
            ],
        )?;

        let updated = Task {
            due_date,
            revision,
            updated_at: now,
            ..task
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            OperationType::Update,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    pub fn update_task_tags(&self, id_prefix: &str, tags: Vec<String>) -> Result<Task> {
        let task = self.find_task_by_prefix(id_prefix)?;
        self.update_task_tags_by_id(task.id, tags)
    }

    pub fn update_task_tags_by_id(&self, id: Uuid, tags: Vec<String>) -> Result<Task> {
        let task = self.find_task_by_id(id)?;
        let tags = normalize_tags(tags);
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET tags = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                serde_json::to_string(&tags)?,
                revision,
                now.to_rfc3339(),
                task.id.to_string()
            ],
        )?;

        let updated = Task {
            tags,
            revision,
            updated_at: now,
            ..task
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            OperationType::Update,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    fn set_task_status(&self, task: Task, status: TaskStatus) -> Result<Task> {
        let now = Utc::now();
        let revision = task.revision + 1;
        self.conn.execute(
            "UPDATE tasks SET status = ?1, revision = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                status.as_str(),
                revision,
                now.to_rfc3339(),
                task.id.to_string()
            ],
        )?;

        let updated = Task {
            status,
            revision,
            updated_at: now,
            ..task
        };
        let operation_type = if updated.status == TaskStatus::Archived {
            OperationType::Archive
        } else {
            OperationType::Update
        };
        self.record_operation(
            updated.id,
            ObjectType::Task,
            operation_type,
            json!({ "task": updated }),
        )?;

        Ok(updated)
    }

    fn record_operation(
        &self,
        object_id: Uuid,
        object_type: ObjectType,
        operation_type: OperationType,
        payload: serde_json::Value,
    ) -> Result<Operation> {
        let operation = Operation {
            id: Uuid::new_v4(),
            object_id,
            object_revision: payload_revision(&payload).unwrap_or(1),
            object_type,
            operation_type,
            payload,
            created_at: Utc::now(),
            synced_at: None,
        };

        self.conn.execute(
            "INSERT INTO operations (
                id, object_id, object_revision, object_type, operation_type, payload, created_at, synced_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                operation.id.to_string(),
                operation.object_id.to_string(),
                operation.object_revision,
                operation.object_type.as_str(),
                operation.operation_type.as_str(),
                serde_json::to_string(&operation.payload)?,
                operation.created_at.to_rfc3339(),
                operation.synced_at.map(|date| date.to_rfc3339()),
            ],
        )?;

        Ok(operation)
    }

    fn set_sync_state(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn get_sync_state(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("failed to get sync state")
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS lists (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                revision INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                list_id TEXT NOT NULL REFERENCES lists(id),
                revision INTEGER NOT NULL DEFAULT 1,
                title TEXT NOT NULL,
                note_markdown TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL CHECK(status IN ('open', 'done', 'archived')),
                tags TEXT NOT NULL DEFAULT '[]',
                due_date TEXT,
                sort_key TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_tasks_status_sort
                ON tasks(status, sort_key);

            CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY,
                object_id TEXT NOT NULL,
                object_revision INTEGER NOT NULL DEFAULT 1,
                object_type TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                synced_at TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_operations_synced
                ON operations(synced_at, created_at);

            CREATE TABLE IF NOT EXISTS sync_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
        )?;
        add_column_if_missing(
            &self.conn,
            "lists",
            "revision",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        add_column_if_missing(
            &self.conn,
            "tasks",
            "revision",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        add_column_if_missing(
            &self.conn,
            "operations",
            "object_revision",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        Ok(())
    }

    fn ensure_inbox(&self) -> Result<List> {
        if let Some(list) = self.find_list_by_name("Inbox")? {
            return Ok(list);
        }

        let list = List::inbox();

        self.conn.execute(
            "INSERT INTO lists (id, name, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                list.id.to_string(),
                list.name,
                list.revision,
                list.created_at.to_rfc3339(),
                list.updated_at.to_rfc3339(),
            ],
        )?;

        Ok(list)
    }

    fn inbox_id(&self) -> Result<Uuid> {
        self.find_list_by_name("Inbox")?
            .map(|list| list.id)
            .context("Inbox list is missing")
    }

    fn find_project_by_name(&self, name: &str) -> Result<List> {
        let name = normalize_project_name(name)?;
        self.find_list_by_name(&name)?
            .with_context(|| format!("project not found: {name}"))
    }

    fn find_list_by_name(&self, name: &str) -> Result<Option<List>> {
        self.conn
            .query_row(
                "SELECT id, name, revision, created_at, updated_at FROM lists WHERE name = ?1",
                params![name],
                row_to_list,
            )
            .optional()
            .context("failed to query list")
    }

    fn find_task_by_prefix(&self, id_prefix: &str) -> Result<Task> {
        let prefix = id_prefix.trim();
        if prefix.is_empty() {
            bail!("task id prefix cannot be empty");
        }

        let like_pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(&format!(
            "{}
             FROM tasks
             WHERE id LIKE ?1 AND deleted_at IS NULL",
            select_task_columns()
        ))?;

        let matches = stmt
            .query_map(params![like_pattern], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        match matches.len() {
            0 => bail!("no task matches id prefix {prefix}"),
            1 => Ok(matches.into_iter().next().expect("checked length")),
            _ => bail!("multiple tasks match id prefix {prefix}"),
        }
    }

    fn find_task_by_id(&self, id: Uuid) -> Result<Task> {
        self.conn
            .query_row(
                &format!(
                    "{}
                 FROM tasks
                 WHERE id = ?1 AND deleted_at IS NULL",
                    select_task_columns()
                ),
                params![id.to_string()],
                row_to_task,
            )
            .optional()?
            .with_context(|| format!("no task matches id {id}"))
    }
}

fn row_to_list(row: &rusqlite::Row<'_>) -> rusqlite::Result<List> {
    Ok(List {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        name: row.get(1)?,
        revision: row.get(2)?,
        created_at: parse_datetime(row.get::<_, String>(3)?)?,
        updated_at: parse_datetime(row.get::<_, String>(4)?)?,
    })
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let object_type = row.get::<_, String>(3)?;
    let operation_type = row.get::<_, String>(4)?;
    let payload = row.get::<_, String>(5)?;
    let synced_at = row.get::<_, Option<String>>(7)?;

    Ok(Operation {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        object_id: parse_uuid(row.get::<_, String>(1)?)?,
        object_revision: row.get(2)?,
        object_type: ObjectType::try_from(object_type.as_str()).map_err(|error| {
            to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?,
        operation_type: OperationType::try_from(operation_type.as_str()).map_err(|error| {
            to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?,
        payload: serde_json::from_str(&payload).map_err(to_sql_error)?,
        created_at: parse_datetime(row.get::<_, String>(6)?)?,
        synced_at: synced_at.map(parse_datetime).transpose()?,
    })
}

fn select_task_sql() -> &'static str {
    "SELECT id, list_id, revision, title, note_markdown, status, tags, due_date,
            sort_key, created_at, updated_at, deleted_at
     FROM tasks"
}

fn select_task_columns() -> &'static str {
    "SELECT id, list_id, revision, title, note_markdown, status, tags, due_date,
            sort_key, created_at, updated_at, deleted_at"
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status = row.get::<_, String>(5)?;
    let tags = row.get::<_, String>(6)?;
    let due_date = row.get::<_, Option<String>>(7)?;
    let deleted_at = row.get::<_, Option<String>>(11)?;

    Ok(Task {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        list_id: parse_uuid(row.get::<_, String>(1)?)?,
        revision: row.get(2)?,
        title: row.get(3)?,
        note_markdown: row.get(4)?,
        status: TaskStatus::try_from(status.as_str()).map_err(|error| {
            to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?,
        tags: serde_json::from_str(&tags).map_err(to_sql_error)?,
        due_date: due_date
            .map(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(to_sql_error))
            .transpose()?,
        sort_key: row.get(8)?,
        created_at: parse_datetime(row.get::<_, String>(9)?)?,
        updated_at: parse_datetime(row.get::<_, String>(10)?)?,
        deleted_at: deleted_at.map(parse_datetime).transpose()?,
    })
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn payload_revision(payload: &serde_json::Value) -> Option<i64> {
    payload
        .get("task")
        .or_else(|| payload.get("list"))
        .and_then(|object| object.get("revision"))
        .and_then(serde_json::Value::as_i64)
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut tags = tags
        .into_iter()
        .map(|tag| tag.trim().trim_start_matches('#').to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

fn normalize_project_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("project name cannot be empty");
    }
    Ok(name.to_owned())
}

fn parse_uuid(value: String) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value).map_err(to_sql_error)
}

fn parse_datetime(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(to_sql_error)
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_and_completes_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();

        let task = store.add_task(NewTask::new("Ship MVP")).unwrap();
        assert_eq!(task.status, TaskStatus::Open);
        assert_eq!(task.revision, 1);
        assert_eq!(store.pending_operations().unwrap().len(), 1);
        assert_eq!(store.pending_operations().unwrap()[0].object_revision, 1);

        let open_tasks = store.list_tasks(false).unwrap();
        assert_eq!(open_tasks.len(), 1);
        assert_eq!(open_tasks[0].title, "Ship MVP");

        let done = store.mark_done(&task.id.to_string()[..8]).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.revision, 2);

        assert!(store.list_tasks(false).unwrap().is_empty());
        assert_eq!(store.list_tasks(true).unwrap().len(), 1);

        let reopened = store.toggle_done(task.id).unwrap();
        assert_eq!(reopened.status, TaskStatus::Open);
        assert_eq!(reopened.revision, 3);
        assert_eq!(store.list_tasks(false).unwrap().len(), 1);

        let edited = store
            .update_task_title_by_id(task.id, "Ship edited MVP")
            .unwrap();
        assert_eq!(edited.title, "Ship edited MVP");
        assert_eq!(edited.revision, 4);
        assert_eq!(store.search_tasks("edited").unwrap().len(), 1);

        let noted = store
            .update_task_note_by_id(task.id, "Review sync protocol")
            .unwrap();
        assert_eq!(noted.note_markdown, "Review sync protocol");
        assert_eq!(store.search_tasks("protocol").unwrap().len(), 1);

        let due_date = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        let scheduled = store
            .update_task_due_date_by_id(task.id, Some(due_date))
            .unwrap();
        assert_eq!(scheduled.due_date, Some(due_date));

        let tagged = store
            .update_task_tags_by_id(task.id, vec!["MVP".to_owned(), "#mvp".to_owned()])
            .unwrap();
        assert_eq!(tagged.tags, vec!["mvp"]);
        assert_eq!(store.search_tasks("mvp").unwrap().len(), 1);

        let archived = store.archive_task_by_id(task.id).unwrap();
        assert_eq!(archived.status, TaskStatus::Archived);
        assert!(store.list_tasks(true).unwrap().is_empty());

        let operations = store.pending_operations().unwrap();
        assert!(operations.len() >= 7);
        assert_eq!(operations[0].operation_type, OperationType::Create);
        assert_eq!(
            operations.last().unwrap().object_revision,
            archived.revision
        );
        assert!(
            operations
                .iter()
                .any(|operation| operation.operation_type == OperationType::Archive)
        );
    }

    #[test]
    fn exports_and_imports_snapshot() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = TodoStore::open(source_dir.path().join("source.db")).unwrap();
        let mut task = NewTask::new("Export me");
        task.note_markdown = "Snapshot note".to_owned();
        task.tags = vec!["backup".to_owned()];
        source.add_task(task).unwrap();

        let snapshot = source.export_snapshot().unwrap();
        assert_eq!(snapshot.version, TodoSnapshot::CURRENT_VERSION);
        assert_eq!(snapshot.tasks.len(), 1);

        let target_dir = tempfile::tempdir().unwrap();
        let mut target = TodoStore::open(target_dir.path().join("target.db")).unwrap();
        target.import_snapshot(snapshot).unwrap();

        let imported = target.search_tasks("backup").unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].title, "Export me");
        assert_eq!(imported[0].note_markdown, "Snapshot note");
        assert_eq!(target.pending_operations().unwrap().len(), 1);
        assert_eq!(
            target.pending_operations().unwrap()[0].operation_type,
            OperationType::ImportSnapshot
        );
    }

    #[test]
    fn manages_projects_and_moves_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();

        let project = store.create_project("LemonTodo").unwrap();
        assert_eq!(project.name, "LemonTodo");
        assert!(
            store
                .pending_operations()
                .unwrap()
                .iter()
                .any(|operation| operation.object_type == ObjectType::List)
        );

        let task = store
            .add_task_to_project(NewTask::new("Project task"), Some("LemonTodo"))
            .unwrap();
        assert_eq!(task.list_id, project.id);
        assert_eq!(
            store
                .list_tasks_for_project(true, Some("LemonTodo"))
                .unwrap()
                .len(),
            1
        );

        let inbox_task = store.add_task(NewTask::new("Inbox task")).unwrap();
        let moved = store
            .move_task_to_project(&inbox_task.id.to_string()[..8], "LemonTodo")
            .unwrap();
        assert_eq!(moved.list_id, project.id);
        assert_eq!(
            store
                .list_tasks_for_project(true, Some("LemonTodo"))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn persists_tui_current_project() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lemontodo.db");
        let store = TodoStore::open(&db_path).unwrap();

        assert_eq!(store.tui_current_project().unwrap(), None);
        store.save_tui_current_project("LemonTodo").unwrap();
        assert_eq!(
            store.tui_current_project().unwrap(),
            Some("LemonTodo".to_owned())
        );

        let reopened = TodoStore::open(db_path).unwrap();
        assert_eq!(
            reopened.tui_current_project().unwrap(),
            Some("LemonTodo".to_owned())
        );
    }
}
