use anyhow::Result;
use lemontodo_core::{NewTask, Task, TaskStatus};
use lemontodo_storage::TodoStore;

pub struct App {
    store: TodoStore,
    tasks: Vec<Task>,
    selected: usize,
    input: String,
    mode: Mode,
    message: String,
    search_query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Add,
    Edit,
    Search,
}

impl App {
    pub fn new(store: TodoStore) -> Result<Self> {
        let mut app = Self {
            store,
            tasks: Vec::new(),
            selected: 0,
            input: String::new(),
            mode: Mode::Browse,
            message: String::new(),
            search_query: String::new(),
        };
        app.refresh()?;
        Ok(app)
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.tasks = if self.search_query.is_empty() {
            self.store.list_tasks(true)?
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

    pub fn start_add(&mut self) {
        self.mode = Mode::Add;
        self.input.clear();
        self.message = "Add task".to_owned();
    }

    pub fn start_edit(&mut self) {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            return;
        };
        self.input = task.title.clone();
        self.mode = Mode::Edit;
        self.message = "Edit task".to_owned();
    }

    pub fn start_search(&mut self) {
        self.input = self.search_query.clone();
        self.mode = Mode::Search;
        self.message = "Search tasks".to_owned();
    }

    pub fn cancel_input(&mut self) {
        self.mode = Mode::Browse;
        self.input.clear();
        self.message = "Cancelled".to_owned();
    }

    pub fn clear_search(&mut self) -> Result<()> {
        self.search_query.clear();
        self.message = "Search cleared".to_owned();
        self.refresh()
    }

    pub fn push_input(&mut self, value: char) {
        self.input.push(value);
    }

    pub fn pop_input(&mut self) {
        self.input.pop();
    }

    pub fn submit_input(&mut self) -> Result<()> {
        match self.mode {
            Mode::Browse => {}
            Mode::Add => self.submit_add()?,
            Mode::Edit => self.submit_edit()?,
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

        let task = self.store.add_task(NewTask::new(title))?;
        self.message = format!("Added {}", task.title);
        self.input.clear();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(task.id);
        Ok(())
    }

    fn submit_edit(&mut self) -> Result<()> {
        let Some(task) = self.selected_task() else {
            self.message = "No task selected".to_owned();
            self.mode = Mode::Browse;
            self.input.clear();
            return Ok(());
        };

        let updated = self.store.update_task_title_by_id(task.id, &self.input)?;
        self.message = format!("Updated {}", updated.title);
        self.input.clear();
        self.mode = Mode::Browse;
        self.refresh()?;
        self.select_task(updated.id);
        Ok(())
    }

    fn submit_search(&mut self) -> Result<()> {
        self.search_query = self.input.trim().to_owned();
        self.input.clear();
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
}
