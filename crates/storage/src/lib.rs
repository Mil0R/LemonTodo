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
const DEVICE_ID_KEY: &str = "sync.device_id";
const LAST_SYNC_CURSOR_KEY: &str = "sync.last_cursor";
const SYNC_SERVER_URL_KEY: &str = "sync.server_url";

pub struct TodoStore {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOperation {
    pub id: Uuid,
    pub operation: Operation,
    pub server_cursor: String,
    pub pulled_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
    pub apply_status: String,
    pub apply_reason: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RemoteApplySummary {
    pub applied: usize,
    pub skipped: usize,
    pub conflicts: usize,
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
        store.ensure_device_id()?;
        store.backfill_operation_device_ids()?;
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
            "SELECT id, device_id, object_id, object_revision, object_type, operation_type, payload, created_at, synced_at
             FROM operations
             WHERE synced_at IS NULL
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_operation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list pending operations")
    }

    pub fn mark_operations_synced(
        &self,
        operation_ids: &[Uuid],
        cursor: Option<&str>,
    ) -> Result<usize> {
        if operation_ids.is_empty() {
            return Ok(0);
        }

        let synced_at = Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction()?;
        let mut updated = 0;
        {
            let mut stmt = tx.prepare(
                "UPDATE operations SET synced_at = ?1 WHERE id = ?2 AND synced_at IS NULL",
            )?;
            for operation_id in operation_ids {
                updated += stmt.execute(params![synced_at, operation_id.to_string()])?;
            }
        }
        if let Some(cursor) = cursor {
            set_sync_state_on_connection(&tx, LAST_SYNC_CURSOR_KEY, cursor)?;
        }
        tx.commit()?;
        Ok(updated)
    }

    pub fn mark_pending_operations_synced(&self, cursor: Option<&str>) -> Result<usize> {
        let operation_ids = self
            .pending_operations()?
            .into_iter()
            .map(|operation| operation.id)
            .collect::<Vec<_>>();
        self.mark_operations_synced(&operation_ids, cursor)
    }

    pub fn operation_ids_by_prefixes(&self, prefixes: &[String]) -> Result<Vec<Uuid>> {
        prefixes
            .iter()
            .map(|prefix| self.find_operation_id_by_prefix(prefix))
            .collect()
    }

    pub fn device_id(&self) -> Result<Uuid> {
        let value = self
            .get_sync_state(DEVICE_ID_KEY)?
            .context("device id is missing")?;
        Uuid::parse_str(&value).context("failed to parse local device id")
    }

    pub fn last_sync_cursor(&self) -> Result<Option<String>> {
        self.get_sync_state(LAST_SYNC_CURSOR_KEY)
    }

    pub fn save_sync_server_url(&self, server_url: &str) -> Result<()> {
        self.set_sync_state(SYNC_SERVER_URL_KEY, normalize_server_url(server_url)?)
    }

    pub fn sync_server_url(&self) -> Result<Option<String>> {
        self.get_sync_state(SYNC_SERVER_URL_KEY)
    }

    pub fn save_remote_operations(
        &self,
        operations: &[Operation],
        server_cursor: &str,
    ) -> Result<usize> {
        if operations.is_empty() {
            return Ok(0);
        }

        let pulled_at = Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction()?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO remote_operations (
                    id, operation_id, device_id, object_id, object_revision,
                    object_type, operation_type, operation_json, server_cursor, pulled_at,
                    applied_at, apply_status, apply_reason
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, 'pending', NULL)",
            )?;
            for operation in operations {
                inserted += stmt.execute(params![
                    Uuid::new_v4().to_string(),
                    operation.id.to_string(),
                    operation.device_id.to_string(),
                    operation.object_id.to_string(),
                    operation.object_revision,
                    operation.object_type.as_str(),
                    operation.operation_type.as_str(),
                    serde_json::to_string(operation)?,
                    server_cursor,
                    pulled_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn pending_remote_operations(&self) -> Result<Vec<RemoteOperation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, operation_json, server_cursor, pulled_at, applied_at, apply_status, apply_reason
             FROM remote_operations
             WHERE applied_at IS NULL
             ORDER BY pulled_at ASC, operation_id ASC",
        )?;
        let rows = stmt.query_map([], row_to_remote_operation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list pending remote operations")
    }

    pub fn pending_remote_operation_count(&self) -> Result<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM remote_operations WHERE applied_at IS NULL",
                [],
                |row| row.get::<_, usize>(0),
            )
            .context("failed to count pending remote operations")
    }

    pub fn apply_pending_remote_creates(&self) -> Result<RemoteApplySummary> {
        let mut pending = self.pending_remote_operations()?;
        pending.sort_by_key(|remote| match remote.operation.object_type {
            ObjectType::List => 0,
            ObjectType::Task => 1,
            ObjectType::Snapshot => 2,
        });
        let mut summary = RemoteApplySummary::default();

        for remote in pending {
            let result = match (
                remote.operation.object_type,
                remote.operation.operation_type,
            ) {
                (ObjectType::List, OperationType::Create) => self.apply_remote_list_create(&remote),
                (ObjectType::Task, OperationType::Create) => self.apply_remote_task_create(&remote),
                _ => {
                    self.mark_remote_operation_blocked(
                        remote.id,
                        "skipped",
                        "only create operations can be applied automatically",
                    )?;
                    summary.skipped += 1;
                    continue;
                }
            }?;

            match result {
                RemoteApplyResult::Applied => summary.applied += 1,
                RemoteApplyResult::Skipped(reason) => {
                    self.mark_remote_operation_blocked(remote.id, "skipped", reason)?;
                    summary.skipped += 1;
                }
                RemoteApplyResult::Conflict(reason) => {
                    self.mark_remote_operation_blocked(remote.id, "conflict", reason)?;
                    summary.conflicts += 1;
                }
            }
        }

        Ok(summary)
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
            device_id: self.device_id()?,
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
                id, device_id, object_id, object_revision, object_type, operation_type, payload, created_at, synced_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.id.to_string(),
                operation.device_id.to_string(),
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
                device_id TEXT,
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

            CREATE TABLE IF NOT EXISTS remote_operations (
                id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL UNIQUE,
                device_id TEXT NOT NULL,
                object_id TEXT NOT NULL,
                object_revision INTEGER NOT NULL,
                object_type TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                operation_json TEXT NOT NULL,
                server_cursor TEXT NOT NULL,
                pulled_at TEXT NOT NULL,
                applied_at TEXT,
                apply_status TEXT NOT NULL DEFAULT 'pending',
                apply_reason TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_remote_operations_pending
                ON remote_operations(applied_at, pulled_at);

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
        add_column_if_missing(&self.conn, "operations", "device_id", "TEXT")?;
        add_column_if_missing(
            &self.conn,
            "operations",
            "object_revision",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        add_column_if_missing(
            &self.conn,
            "remote_operations",
            "apply_status",
            "TEXT NOT NULL DEFAULT 'pending'",
        )?;
        add_column_if_missing(&self.conn, "remote_operations", "apply_reason", "TEXT")?;
        Ok(())
    }

    fn ensure_device_id(&self) -> Result<Uuid> {
        if let Some(value) = self.get_sync_state(DEVICE_ID_KEY)? {
            return Uuid::parse_str(&value).context("failed to parse local device id");
        }

        let device_id = Uuid::new_v4();
        self.set_sync_state(DEVICE_ID_KEY, &device_id.to_string())?;
        Ok(device_id)
    }

    fn backfill_operation_device_ids(&self) -> Result<()> {
        let device_id = self.device_id()?;
        self.conn.execute(
            "UPDATE operations SET device_id = ?1 WHERE device_id IS NULL OR device_id = ''",
            params![device_id.to_string()],
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

    fn find_operation_id_by_prefix(&self, id_prefix: &str) -> Result<Uuid> {
        let prefix = id_prefix.trim();
        if prefix.is_empty() {
            bail!("operation id prefix cannot be empty");
        }

        let like_pattern = format!("{prefix}%");
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM operations WHERE id LIKE ?1")?;
        let matches = stmt
            .query_map(params![like_pattern], |row| {
                parse_uuid(row.get::<_, String>(0)?)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        match matches.len() {
            0 => bail!("no operation matches id prefix {prefix}"),
            1 => Ok(matches.into_iter().next().expect("checked length")),
            _ => bail!("multiple operations match id prefix {prefix}"),
        }
    }

    fn apply_remote_list_create(&self, remote: &RemoteOperation) -> Result<RemoteApplyResult> {
        let list = remote_operation_payload::<List>(&remote.operation, "list")?;
        if self.find_list_by_id(list.id)?.is_some() || self.find_list_by_name(&list.name)?.is_some()
        {
            return Ok(RemoteApplyResult::Conflict(
                "list id or name already exists locally",
            ));
        }

        self.conn.execute(
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
        self.mark_remote_operation_applied(remote.id)?;
        Ok(RemoteApplyResult::Applied)
    }

    fn apply_remote_task_create(&self, remote: &RemoteOperation) -> Result<RemoteApplyResult> {
        let task = remote_operation_payload::<Task>(&remote.operation, "task")?;
        if self.find_task_any_by_id(task.id)?.is_some() {
            return Ok(RemoteApplyResult::Conflict(
                "task id already exists locally",
            ));
        }
        if self.find_list_by_id(task.list_id)?.is_none() {
            return Ok(RemoteApplyResult::Skipped(
                "task list does not exist locally yet",
            ));
        }

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
        self.mark_remote_operation_applied(remote.id)?;
        Ok(RemoteApplyResult::Applied)
    }

    fn mark_remote_operation_applied(&self, id: Uuid) -> Result<()> {
        self.conn.execute(
            "UPDATE remote_operations
             SET applied_at = ?1, apply_status = 'applied', apply_reason = NULL
             WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(())
    }

    fn mark_remote_operation_blocked(&self, id: Uuid, status: &str, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE remote_operations
             SET apply_status = ?1, apply_reason = ?2
             WHERE id = ?3 AND applied_at IS NULL",
            params![status, reason, id.to_string()],
        )?;
        Ok(())
    }

    fn find_list_by_id(&self, id: Uuid) -> Result<Option<List>> {
        self.conn
            .query_row(
                "SELECT id, name, revision, created_at, updated_at FROM lists WHERE id = ?1",
                params![id.to_string()],
                row_to_list,
            )
            .optional()
            .context("failed to query list by id")
    }

    fn find_task_any_by_id(&self, id: Uuid) -> Result<Option<Task>> {
        self.conn
            .query_row(
                &format!("{} FROM tasks WHERE id = ?1", select_task_columns()),
                params![id.to_string()],
                row_to_task,
            )
            .optional()
            .context("failed to query task by id")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteApplyResult {
    Applied,
    Skipped(&'static str),
    Conflict(&'static str),
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
    let object_type = row.get::<_, String>(4)?;
    let operation_type = row.get::<_, String>(5)?;
    let payload = row.get::<_, String>(6)?;
    let synced_at = row.get::<_, Option<String>>(8)?;

    Ok(Operation {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        device_id: parse_uuid(row.get::<_, String>(1)?)?,
        object_id: parse_uuid(row.get::<_, String>(2)?)?,
        object_revision: row.get(3)?,
        object_type: ObjectType::try_from(object_type.as_str()).map_err(|error| {
            to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?,
        operation_type: OperationType::try_from(operation_type.as_str()).map_err(|error| {
            to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?,
        payload: serde_json::from_str(&payload).map_err(to_sql_error)?,
        created_at: parse_datetime(row.get::<_, String>(7)?)?,
        synced_at: synced_at.map(parse_datetime).transpose()?,
    })
}

fn row_to_remote_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteOperation> {
    let operation_json = row.get::<_, String>(1)?;
    let applied_at = row.get::<_, Option<String>>(4)?;
    Ok(RemoteOperation {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        operation: serde_json::from_str(&operation_json).map_err(to_sql_error)?,
        server_cursor: row.get(2)?,
        pulled_at: parse_datetime(row.get::<_, String>(3)?)?,
        applied_at: applied_at.map(parse_datetime).transpose()?,
        apply_status: row.get(5)?,
        apply_reason: row.get(6)?,
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

fn remote_operation_payload<T: serde::de::DeserializeOwned>(
    operation: &Operation,
    key: &str,
) -> Result<T> {
    let value = operation.payload.get(key).with_context(|| {
        format!(
            "remote operation {} missing payload key {key}",
            operation.id
        )
    })?;
    serde_json::from_value(value.clone())
        .with_context(|| format!("failed to parse remote operation {} payload", operation.id))
}

fn set_sync_state_on_connection(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at",
        params![key, value, Utc::now().to_rfc3339()],
    )?;
    Ok(())
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

fn normalize_server_url(url: &str) -> Result<&str> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        bail!("sync server URL cannot be empty");
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("sync server URL must start with http:// or https://");
    }
    Ok(url)
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

    #[test]
    fn tracks_sync_device_and_acknowledges_operations() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lemontodo.db");
        let store = TodoStore::open(&db_path).unwrap();
        let device_id = store.device_id().unwrap();

        let task = store.add_task(NewTask::new("Sync me")).unwrap();
        let updated = store
            .update_task_title_by_id(task.id, "Synced title")
            .unwrap();
        assert_eq!(updated.revision, 2);

        let operations = store.pending_operations().unwrap();
        assert_eq!(operations.len(), 2);
        assert!(
            operations
                .iter()
                .all(|operation| operation.device_id == device_id)
        );
        assert_eq!(operations[0].object_revision, 1);
        assert_eq!(operations[1].object_revision, 2);

        let acknowledged = store
            .mark_operations_synced(&[operations[0].id], Some("server-cursor-1"))
            .unwrap();
        assert_eq!(acknowledged, 1);
        assert_eq!(
            store.last_sync_cursor().unwrap(),
            Some("server-cursor-1".to_owned())
        );
        assert_eq!(store.pending_operations().unwrap().len(), 1);

        let acknowledged = store
            .mark_pending_operations_synced(Some("server-cursor-2"))
            .unwrap();
        assert_eq!(acknowledged, 1);
        assert!(store.pending_operations().unwrap().is_empty());
        assert_eq!(
            store.last_sync_cursor().unwrap(),
            Some("server-cursor-2".to_owned())
        );

        let reopened = TodoStore::open(db_path).unwrap();
        assert_eq!(reopened.device_id().unwrap(), device_id);
        assert_eq!(
            reopened.last_sync_cursor().unwrap(),
            Some("server-cursor-2".to_owned())
        );
    }

    #[test]
    fn persists_sync_server_url() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lemontodo.db");
        let store = TodoStore::open(&db_path).unwrap();

        assert_eq!(store.sync_server_url().unwrap(), None);
        store
            .save_sync_server_url("http://localhost:8787/")
            .unwrap();
        assert_eq!(
            store.sync_server_url().unwrap(),
            Some("http://localhost:8787".to_owned())
        );

        let reopened = TodoStore::open(db_path).unwrap();
        assert_eq!(
            reopened.sync_server_url().unwrap(),
            Some("http://localhost:8787".to_owned())
        );
    }

    #[test]
    fn stores_remote_operations_for_later_apply() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: Uuid::new_v4(),
            object_revision: 1,
            object_type: ObjectType::Task,
            operation_type: OperationType::Create,
            payload: json!({ "title": "Remote task" }),
            created_at: Utc::now(),
            synced_at: None,
        };

        assert_eq!(store.pending_remote_operation_count().unwrap(), 0);
        assert_eq!(
            store
                .save_remote_operations(std::slice::from_ref(&operation), "cursor-1")
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .save_remote_operations(std::slice::from_ref(&operation), "cursor-1")
                .unwrap(),
            0
        );

        let pending = store.pending_remote_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation.id, operation.id);
        assert_eq!(pending[0].server_cursor, "cursor-1");
        assert_eq!(pending[0].apply_status, "pending");
        assert_eq!(pending[0].apply_reason, None);
        assert_eq!(store.pending_remote_operation_count().unwrap(), 1);
    }

    #[test]
    fn applies_remote_create_operations_when_safe() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();
        let project = List {
            id: Uuid::new_v4(),
            name: "Remote Project".to_owned(),
            revision: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let task = Task {
            id: Uuid::new_v4(),
            list_id: project.id,
            revision: 1,
            title: "Remote Task".to_owned(),
            note_markdown: String::new(),
            status: TaskStatus::Open,
            tags: Vec::new(),
            due_date: None,
            sort_key: "0001".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let operations = vec![
            Operation {
                id: Uuid::new_v4(),
                device_id: Uuid::new_v4(),
                object_id: project.id,
                object_revision: 1,
                object_type: ObjectType::List,
                operation_type: OperationType::Create,
                payload: json!({ "list": project }),
                created_at: Utc::now(),
                synced_at: None,
            },
            Operation {
                id: Uuid::new_v4(),
                device_id: Uuid::new_v4(),
                object_id: task.id,
                object_revision: 1,
                object_type: ObjectType::Task,
                operation_type: OperationType::Create,
                payload: json!({ "task": task }),
                created_at: Utc::now(),
                synced_at: None,
            },
        ];

        store
            .save_remote_operations(&operations, "cursor-2")
            .unwrap();
        let summary = store.apply_pending_remote_creates().unwrap();
        assert_eq!(summary.applied, 2);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.conflicts, 0);
        assert_eq!(store.pending_remote_operation_count().unwrap(), 0);
        assert_eq!(
            store
                .list_tasks_for_project(true, Some("Remote Project"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn leaves_remote_task_create_pending_when_list_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();
        let task = Task {
            id: Uuid::new_v4(),
            list_id: Uuid::new_v4(),
            revision: 1,
            title: "Missing Project Task".to_owned(),
            note_markdown: String::new(),
            status: TaskStatus::Open,
            tags: Vec::new(),
            due_date: None,
            sort_key: "0001".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: task.id,
            object_revision: 1,
            object_type: ObjectType::Task,
            operation_type: OperationType::Create,
            payload: json!({ "task": task }),
            created_at: Utc::now(),
            synced_at: None,
        };

        store
            .save_remote_operations(std::slice::from_ref(&operation), "cursor-1")
            .unwrap();
        let summary = store.apply_pending_remote_creates().unwrap();
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.conflicts, 0);
        let pending = store.pending_remote_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].apply_status, "skipped");
        assert_eq!(
            pending[0].apply_reason,
            Some("task list does not exist locally yet".to_owned())
        );
    }

    #[test]
    fn marks_unsupported_remote_operations_as_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: Uuid::new_v4(),
            object_revision: 2,
            object_type: ObjectType::Task,
            operation_type: OperationType::Update,
            payload: json!({ "task": { "title": "unsupported update" } }),
            created_at: Utc::now(),
            synced_at: None,
        };

        store
            .save_remote_operations(std::slice::from_ref(&operation), "cursor-1")
            .unwrap();
        let summary = store.apply_pending_remote_creates().unwrap();
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.conflicts, 0);

        let pending = store.pending_remote_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].apply_status, "skipped");
        assert_eq!(
            pending[0].apply_reason,
            Some("only create operations can be applied automatically".to_owned())
        );
    }

    #[test]
    fn marks_remote_list_create_name_collision_as_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("lemontodo.db")).unwrap();
        store.create_project("Existing").unwrap();
        let remote_list = List {
            id: Uuid::new_v4(),
            name: "Existing".to_owned(),
            revision: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: remote_list.id,
            object_revision: 1,
            object_type: ObjectType::List,
            operation_type: OperationType::Create,
            payload: json!({ "list": remote_list }),
            created_at: Utc::now(),
            synced_at: None,
        };

        store
            .save_remote_operations(std::slice::from_ref(&operation), "cursor-1")
            .unwrap();
        let summary = store.apply_pending_remote_creates().unwrap();
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.conflicts, 1);

        let pending = store.pending_remote_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].apply_status, "conflict");
        assert_eq!(
            pending[0].apply_reason,
            Some("list id or name already exists locally".to_owned())
        );
    }
}
