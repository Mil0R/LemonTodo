use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use lemontodo_core::{List, NewTask, Task, TaskStatus};
use lemontodo_storage::TodoStore;

pub struct App {
    store: TodoStore,
    tasks: Vec<Task>,
    projects: Vec<List>,
    project_names: HashMap<uuid::Uuid, String>,
    current_project: ProjectSelection,
    selected: usize,
    input: String,
    input_cursor: usize,
    mode: Mode,
    view_mode: ViewMode,
    help_visible: bool,
    sync_status_visible: bool,
    sync_status: Option<SyncStatus>,
    message: String,
    search_query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectSelection {
    All,
    Project(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Add,
    EditTitle,
    EditNote,
    EditDue,
    EditTags,
    MoveProject,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Compact,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    pub server: Option<String>,
    pub account: Option<String>,
    pub access_token_configured: bool,
    pub vault_metadata_configured: bool,
    pub pending_local_operations: usize,
    pub pending_remote_operations: usize,
}

impl App {
    pub fn new(store: TodoStore) -> Result<Self> {
        let mut app = Self {
            store,
            tasks: Vec::new(),
            projects: Vec::new(),
            project_names: HashMap::new(),
            current_project: ProjectSelection::All,
            selected: 0,
            input: String::new(),
            input_cursor: 0,
            mode: Mode::Browse,
            view_mode: ViewMode::Compact,
            help_visible: false,
            sync_status_visible: false,
            sync_status: None,
            message: String::new(),
            search_query: String::new(),
        };
        app.refresh()?;
        app.restore_current_project()?;
        Ok(app)
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn project_name_for_task(&self, task: &Task) -> &str {
        self.project_names
            .get(&task.list_id)
            .map(String::as_str)
            .unwrap_or("Unknown")
    }

    pub fn current_project_name(&self) -> &str {
        match self.current_project {
            ProjectSelection::All => "All",
            ProjectSelection::Project(index) => self
                .projects
                .get(index)
                .map(|project| project.name.as_str())
                .unwrap_or("All"),
        }
    }

    pub fn shows_all_projects(&self) -> bool {
        self.current_project == ProjectSelection::All
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn input_cursor(&self) -> usize {
        self.input_cursor
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    pub fn view_mode_name(&self) -> &str {
        match self.view_mode {
            ViewMode::Compact => "compact",
            ViewMode::Detail => "detail",
        }
    }

    pub fn help_visible(&self) -> bool {
        self.help_visible
    }

    pub fn sync_status_visible(&self) -> bool {
        self.sync_status_visible
    }

    pub fn sync_status(&self) -> Option<&SyncStatus> {
        self.sync_status.as_ref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn store(&self) -> &TodoStore {
        &self.store
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.projects = self.store.projects()?;
        self.project_names = self
            .projects
            .iter()
            .map(|project| (project.id, project.name.clone()))
            .collect();
        self.clamp_project();

        let current_project = self.current_project_name().to_owned();
        let project_filter = match self.current_project {
            ProjectSelection::All => None,
            ProjectSelection::Project(_) => Some(current_project.as_str()),
        };
        self.tasks = if self.search_query.is_empty() {
            self.store.list_tasks_for_project(true, project_filter)?
        } else {
            self.store.search_tasks(&self.search_query)?
        };
        self.clamp_selection();
        Ok(())
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.tasks.get(self.selected)
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.tasks.len() {
            self.selected += 1;
        }
    }

    pub fn previous_project(&mut self) -> Result<()> {
        self.current_project = match self.current_project {
            ProjectSelection::All => {
                if self.projects.is_empty() {
                    ProjectSelection::All
                } else {
                    ProjectSelection::Project(self.projects.len() - 1)
                }
            }
            ProjectSelection::Project(0) => ProjectSelection::All,
            ProjectSelection::Project(index) => ProjectSelection::Project(index - 1),
        };
        self.message = format!("Project: {}", self.current_project_name());
        self.save_current_project()?;
        self.refresh()
    }

    pub fn next_project(&mut self) -> Result<()> {
        self.current_project = match self.current_project {
            ProjectSelection::All => {
                if self.projects.is_empty() {
                    ProjectSelection::All
                } else {
                    ProjectSelection::Project(0)
                }
            }
            ProjectSelection::Project(index) if index + 1 < self.projects.len() => {
                ProjectSelection::Project(index + 1)
            }
            ProjectSelection::Project(_) => ProjectSelection::All,
        };
        self.message = format!("Project: {}", self.current_project_name());
        self.save_current_project()?;
        self.refresh()
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Compact => ViewMode::Detail,
            ViewMode::Detail => ViewMode::Compact,
        };
        self.message = format!("View: {}", self.view_mode_name());
    }

    pub fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
        if self.help_visible {
            self.sync_status_visible = false;
        }
    }

    pub fn hide_help(&mut self) {
        self.help_visible = false;
    }

    pub fn toggle_sync_status(&mut self) -> Result<()> {
        if self.sync_status_visible {
            self.sync_status_visible = false;
            return Ok(());
        }
        self.help_visible = false;
        self.sync_status = Some(self.load_sync_status()?);
        self.sync_status_visible = true;
        self.message = "Sync status".to_owned();
        Ok(())
    }

    pub fn hide_sync_status(&mut self) {
        self.sync_status_visible = false;
    }

    pub fn start_add(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        self.mode = Mode::Add;
        self.clear_input();
        self.message = "Add task".to_owned();
    }

    pub fn start_edit_title(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.set_input(task.title.clone());
        self.mode = Mode::EditTitle;
        self.message = "Edit title".to_owned();
    }

    pub fn start_edit_note(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.set_input(task.note_markdown.clone());
        self.mode = Mode::EditNote;
        self.message = "Edit note".to_owned();
    }

    pub fn start_edit_due(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.set_input(
            task.due_date
                .map(|date| date.to_string())
                .unwrap_or_default(),
        );
        self.mode = Mode::EditDue;
        self.message = "Edit due date as YYYY-MM-DD, empty clears".to_owned();
    }

    pub fn start_edit_tags(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.set_input(task.tags.join(" "));
        self.mode = Mode::EditTags;
        self.message = "Edit tags separated by spaces or commas".to_owned();
    }

    pub fn start_move_project(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.set_input(self.project_name_for_task(task).to_owned());
        self.mode = Mode::MoveProject;
        self.message = "Move task to project".to_owned();
    }

    pub fn start_search(&mut self) {
        self.hide_help();
        self.hide_sync_status();
        self.set_input(self.search_query.clone());
        self.mode = Mode::Search;
        self.message = "Search tasks".to_owned();
    }

    pub fn cancel_input(&mut self) {
        self.mode = Mode::Browse;
        self.clear_input();
        self.message = "Cancelled".to_owned();
    }

    pub fn clear_search(&mut self) -> Result<()> {
        self.search_query.clear();
        self.message = "Search cleared".to_owned();
        self.refresh()
    }

    pub fn push_input(&mut self, value: char) {
        self.input.insert(self.input_cursor, value);
        self.input_cursor += value.len_utf8();
    }

    pub fn pop_input(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let previous = previous_char_boundary(&self.input, self.input_cursor);
        self.input.drain(previous..self.input_cursor);
        self.input_cursor = previous;
    }

    pub fn delete_input(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        let next = next_char_boundary(&self.input, self.input_cursor);
        self.input.drain(self.input_cursor..next);
    }

    pub fn move_input_left(&mut self) {
        self.input_cursor = previous_char_boundary(&self.input, self.input_cursor);
    }

    pub fn move_input_right(&mut self) {
        self.input_cursor = next_char_boundary(&self.input, self.input_cursor);
    }

    pub fn move_input_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn move_input_end(&mut self) {
        self.input_cursor = self.input.len();
    }

    pub fn submit_input(&mut self) -> Result<()> {
        match self.mode {
            Mode::Browse => {}
            Mode::Add => self.submit_add()?,
            Mode::EditTitle => self.submit_edit_title()?,
            Mode::EditNote => self.submit_edit_note()?,
            Mode::EditDue => self.submit_edit_due()?,
            Mode::EditTags => self.submit_edit_tags()?,
            Mode::MoveProject => self.submit_move_project()?,
            Mode::Search => self.submit_search()?,
        }
        Ok(())
    }

    pub fn toggle_selected(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return Ok(());
        };

        let updated = self.store.toggle_done(task.id)?;
        self.message = match updated.status {
            TaskStatus::Done => format!("Completed {}", updated.title),
            TaskStatus::Open => format!("Reopened {}", updated.title),
            TaskStatus::Archived => format!("Updated {}", updated.title),
        };
        self.refresh()
    }

    pub fn archive_selected(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return Ok(());
        };

        let archived = self.store.archive_task_by_id(task.id)?;
        self.message = format!("Archived {}", archived.title);
        self.refresh()
    }

    fn submit_add(&mut self) -> Result<()> {
        let title = self.input.trim();
        if title.is_empty() {
            self.message = "Task title cannot be empty".to_owned();
            return Ok(());
        }

        let project_name = match self.current_project {
            ProjectSelection::All => None,
            ProjectSelection::Project(_) => Some(self.current_project_name().to_owned()),
        };
        let task = self
            .store
            .add_task_to_project(NewTask::new(title), project_name.as_deref())?;
        self.message = format!("Added {}", task.title);
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(task.id);
        Ok(())
    }

    fn submit_edit_title(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.mode = Mode::Browse;
            self.clear_input();
            return Ok(());
        };

        let updated = self.store.update_task_title_by_id(task.id, &self.input)?;
        self.message = format!("Updated {}", updated.title);
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(updated.id);
        Ok(())
    }

    fn submit_edit_note(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.mode = Mode::Browse;
            self.clear_input();
            return Ok(());
        };

        let updated = self.store.update_task_note_by_id(task.id, &self.input)?;
        self.message = format!("Updated note for {}", updated.title);
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(updated.id);
        Ok(())
    }

    fn submit_edit_due(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.mode = Mode::Browse;
            self.clear_input();
            return Ok(());
        };

        let due_date = parse_due_input(&self.input)?;
        let updated = self.store.update_task_due_date_by_id(task.id, due_date)?;
        self.message = match updated.due_date {
            Some(date) => format!("Updated due date for {} to {date}", updated.title),
            None => format!("Cleared due date for {}", updated.title),
        };
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(updated.id);
        Ok(())
    }

    fn submit_edit_tags(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.mode = Mode::Browse;
            self.clear_input();
            return Ok(());
        };

        let tags = parse_tags_input(&self.input);
        let updated = self.store.update_task_tags_by_id(task.id, tags)?;
        self.message = if updated.tags.is_empty() {
            format!("Cleared tags for {}", updated.title)
        } else {
            format!("Updated tags for {}", updated.title)
        };
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(updated.id);
        Ok(())
    }

    fn submit_move_project(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.clear_input();
            self.mode = Mode::Browse;
            return Ok(());
        };
        let task_id = task.id;
        let task_title = task.title.clone();

        let project_name = self.input.trim();
        if project_name.is_empty() {
            self.message = "Project name cannot be empty".to_owned();
            return Ok(());
        }

        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.name == project_name)
            .cloned()
        else {
            self.message = format!("Project not found: {project_name}");
            return Ok(());
        };

        let moved = self.store.move_task_to_project_by_id(task_id, project.id)?;
        self.message = format!("Moved {task_title} to {}", project.name);
        self.clear_input();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(moved.id);
        Ok(())
    }

    fn submit_search(&mut self) -> Result<()> {
        self.search_query = self.input.trim().to_owned();
        self.clear_input();
        self.mode = Mode::Browse;
        self.message = if self.search_query.is_empty() {
            "Search cleared".to_owned()
        } else {
            format!("Search: {}", self.search_query)
        };
        self.refresh()
    }

    fn select_task(&mut self, id: uuid::Uuid) {
        self.selected = self
            .tasks
            .iter()
            .position(|task| task.id == id)
            .unwrap_or(self.selected);
    }

    fn clamp_selection(&mut self) {
        if self.tasks.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.tasks.len() {
            self.selected = self.tasks.len() - 1;
        }
    }

    fn clamp_project(&mut self) {
        if let ProjectSelection::Project(index) = self.current_project
            && index >= self.projects.len()
        {
            self.current_project = ProjectSelection::All;
        }
    }

    fn restore_current_project(&mut self) -> Result<()> {
        let Some(project_name) = self.store.tui_current_project()? else {
            return Ok(());
        };
        self.current_project = if project_name == "All" {
            ProjectSelection::All
        } else {
            self.projects
                .iter()
                .position(|project| project.name == project_name)
                .map(ProjectSelection::Project)
                .unwrap_or(ProjectSelection::All)
        };
        self.refresh()
    }

    fn save_current_project(&self) -> Result<()> {
        self.store
            .save_tui_current_project(self.current_project_name())
    }

    fn load_sync_status(&self) -> Result<SyncStatus> {
        Ok(SyncStatus {
            server: self.store.sync_server_url()?,
            account: self.store.sync_account_email()?,
            access_token_configured: self.store.sync_access_token()?.is_some(),
            vault_metadata_configured: self.store.encrypted_vault_key()?.is_some(),
            pending_local_operations: self.store.pending_operations()?.len(),
            pending_remote_operations: self.store.pending_remote_operation_count()?,
        })
    }

    fn set_input(&mut self, input: String) {
        self.input = input;
        self.input_cursor = self.input.len();
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
    }
}

fn previous_char_boundary(input: &str, cursor: usize) -> usize {
    input[..cursor]
        .char_indices()
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(input: &str, cursor: usize) -> usize {
    input[cursor..]
        .char_indices()
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(input.len())
}

pub fn parse_due_input(input: &str) -> Result<Option<NaiveDate>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }

    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .map(Some)
        .with_context(|| format!("invalid due date '{input}', expected YYYY-MM-DD"))
}

pub fn parse_tags_input(input: &str) -> Vec<String> {
    input
        .split(|value: char| value == ',' || value.is_whitespace())
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::open(dir.path().join("tui.db")).unwrap();
        App::new(store).unwrap()
    }

    #[test]
    fn edits_input_at_cursor_with_unicode_boundaries() {
        let mut app = test_app();
        app.start_add();
        app.push_input('A');
        app.push_input('中');
        app.push_input('B');

        app.move_input_left();
        app.push_input('x');

        assert_eq!(app.input(), "A中xB");
        assert_eq!(app.input_cursor(), "A中x".len());

        app.move_input_left();
        app.pop_input();

        assert_eq!(app.input(), "AxB");
        assert_eq!(app.input_cursor(), "A".len());

        app.delete_input();

        assert_eq!(app.input(), "AB");
        assert_eq!(app.input_cursor(), "A".len());
    }
}
