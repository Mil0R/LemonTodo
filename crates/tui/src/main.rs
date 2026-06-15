mod app;
mod ui;

use std::{fs, io, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use lemontodo_core::{NewTask, Task, TaskStatus};
use lemontodo_crypto::{KdfParams, VaultKey, unwrap_vault_key, wrap_vault_key};
use lemontodo_storage::TodoStore;
use lemontodo_sync::pack_operations;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::{App, Mode, parse_due_input},
    ui::draw,
};

#[derive(Debug, Parser)]
#[command(name = "ltd")]
#[command(about = "LemonTodo developer-first Todo CLI/TUI")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize the local LemonTodo database.
    Init,
    /// Add a task to Inbox.
    Add {
        title: String,
        #[arg(long, short = 'p')]
        project: Option<String>,
        #[arg(long, short = 'n')]
        note: Option<String>,
        #[arg(long, short = 't')]
        tag: Vec<String>,
        #[arg(long)]
        due: Option<NaiveDate>,
    },
    /// List tasks.
    List {
        #[arg(long)]
        all: bool,
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
    /// Manage projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Search tasks locally.
    Search { query: String },
    /// Mark a task done by id prefix.
    Done { id: String },
    /// Edit a task title by id prefix.
    Edit { id: String, title: String },
    /// Edit a task note by id prefix.
    Note { id: String, note: String },
    /// Set or clear a task due date by id prefix.
    Due {
        id: String,
        #[arg(value_name = "YYYY-MM-DD")]
        date: Option<String>,
    },
    /// Replace task tags by id prefix.
    Tags { id: String, tags: Vec<String> },
    /// Move a task to a project by id prefix.
    Move { id: String, project: String },
    /// Export local data as JSON snapshot.
    Export {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Import a JSON snapshot.
    Import {
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Inspect pending local operations for future sync.
    Ops,
    /// Local encrypted sync dry-run commands.
    Sync {
        #[command(subcommand)]
        command: SyncCommand,
    },
    /// Manage the local encrypted vault metadata.
    Vault {
        #[command(subcommand)]
        command: VaultCommand,
    },
    /// Archive a task by id prefix.
    Archive { id: String },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Create a project.
    Add { name: String },
    /// List projects.
    List,
}

#[derive(Debug, Subcommand)]
enum SyncCommand {
    /// Generate a random local vault key as hex.
    Keygen,
    /// Pack pending operations into encrypted sync objects.
    Pack {
        #[arg(long)]
        key: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum VaultCommand {
    /// Initialize local vault metadata with a master password.
    Init {
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
    },
    /// Show whether local vault metadata exists.
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(default_db_path);
    let store = TodoStore::open(&db_path)?;

    match cli.command {
        Some(Command::Init) => {
            println!("Initialized LemonTodo database at {}", db_path.display());
        }
        Some(Command::Add {
            title,
            project,
            note,
            tag,
            due,
        }) => {
            let mut task = NewTask::new(title);
            task.note_markdown = note.unwrap_or_default();
            task.tags = tag;
            task.due_date = due;

            let task = store.add_task_to_project(task, project.as_deref())?;
            println!("Added {} {}", short_id(&task.id.to_string()), task.title);
        }
        Some(Command::List { all, project }) => {
            print_tasks(store.list_tasks_for_project(all, project.as_deref())?)?;
        }
        Some(Command::Project { command }) => match command {
            ProjectCommand::Add { name } => {
                let project = store.create_project(&name)?;
                println!(
                    "Project {} {}",
                    short_id(&project.id.to_string()),
                    project.name
                );
            }
            ProjectCommand::List => {
                for project in store.projects()? {
                    println!("{} {}", short_id(&project.id.to_string()), project.name);
                }
            }
        },
        Some(Command::Search { query }) => {
            print_tasks(store.search_tasks(&query)?)?;
        }
        Some(Command::Done { id }) => {
            let task = store.mark_done(&id)?;
            println!(
                "Completed {} {}",
                short_id(&task.id.to_string()),
                task.title
            );
        }
        Some(Command::Edit { id, title }) => {
            let task = store.update_task_title(&id, &title)?;
            println!("Updated {} {}", short_id(&task.id.to_string()), task.title);
        }
        Some(Command::Note { id, note }) => {
            let task = store.update_task_note(&id, &note)?;
            println!(
                "Updated note for {} {}",
                short_id(&task.id.to_string()),
                task.title
            );
        }
        Some(Command::Due { id, date }) => {
            let due_date = parse_due_input(date.as_deref().unwrap_or_default())?;
            let task = store.update_task_due_date(&id, due_date)?;
            match task.due_date {
                Some(date) => println!(
                    "Updated due date for {} {} to {date}",
                    short_id(&task.id.to_string()),
                    task.title
                ),
                None => println!(
                    "Cleared due date for {} {}",
                    short_id(&task.id.to_string()),
                    task.title
                ),
            }
        }
        Some(Command::Tags { id, tags }) => {
            let task = store.update_task_tags(&id, tags)?;
            println!(
                "Updated tags for {} {}",
                short_id(&task.id.to_string()),
                task.title
            );
        }
        Some(Command::Move { id, project }) => {
            let task = store.move_task_to_project(&id, &project)?;
            println!(
                "Moved {} {} to {}",
                short_id(&task.id.to_string()),
                task.title,
                project
            );
        }
        Some(Command::Export { path }) => {
            let snapshot = store.export_snapshot()?;
            let json = serde_json::to_string_pretty(&snapshot)?;
            if let Some(path) = path {
                fs::write(&path, json)
                    .with_context(|| format!("failed to write export {}", path.display()))?;
                println!("Exported LemonTodo snapshot to {}", path.display());
            } else {
                println!("{json}");
            }
        }
        Some(Command::Import { path }) => {
            let json = fs::read_to_string(&path)
                .with_context(|| format!("failed to read import {}", path.display()))?;
            let snapshot = serde_json::from_str(&json)
                .with_context(|| format!("failed to parse import {}", path.display()))?;
            let mut store = store;
            store.import_snapshot(snapshot)?;
            println!("Imported LemonTodo snapshot from {}", path.display());
        }
        Some(Command::Ops) => {
            let operations = store.pending_operations()?;
            if operations.is_empty() {
                println!("No pending operations");
            } else {
                for operation in operations {
                    println!(
                        "{} {} {} {}",
                        short_id(&operation.id.to_string()),
                        operation.object_type.as_str(),
                        operation.operation_type.as_str(),
                        operation.object_id
                    );
                }
            }
        }
        Some(Command::Sync { command }) => match command {
            SyncCommand::Keygen => {
                println!("{}", VaultKey::generate().to_hex());
            }
            SyncCommand::Pack {
                key,
                master_password,
                out,
            } => {
                let vault_key = load_vault_key(&store, key.as_deref(), master_password)?;
                let operations = store.pending_operations()?;
                let pack = pack_operations(&vault_key, &operations)?;
                let json = serde_json::to_string_pretty(&pack)?;
                if let Some(path) = out {
                    fs::write(&path, json)
                        .with_context(|| format!("failed to write sync pack {}", path.display()))?;
                    println!(
                        "Packed {} encrypted sync objects to {}",
                        pack.objects.len(),
                        path.display()
                    );
                } else {
                    println!("{json}");
                }
            }
        },
        Some(Command::Vault { command }) => match command {
            VaultCommand::Init { master_password } => {
                if store.encrypted_vault_key()?.is_some() {
                    anyhow::bail!("local vault metadata already exists");
                }
                let master_password = master_password
                    .map(Ok)
                    .unwrap_or_else(prompt_new_master_password)?;
                let vault_key = VaultKey::generate();
                let encrypted_vault_key = wrap_vault_key(
                    &vault_key,
                    &master_password,
                    KdfParams::generate_interactive(),
                )?;
                store.save_encrypted_vault_key(&encrypted_vault_key)?;
                println!("Initialized local encrypted vault metadata");
            }
            VaultCommand::Status => {
                if store.encrypted_vault_key()?.is_some() {
                    println!("Local encrypted vault metadata exists");
                } else {
                    println!("Local encrypted vault metadata is not initialized");
                }
            }
        },
        Some(Command::Archive { id }) => {
            let task = store.archive_task(&id)?;
            println!("Archived {} {}", short_id(&task.id.to_string()), task.title);
        }
        None => run_tui(store)?,
    }

    Ok(())
}

fn load_vault_key(
    store: &TodoStore,
    key: Option<&str>,
    master_password: Option<String>,
) -> Result<VaultKey> {
    match (key, master_password) {
        (Some(key), None) => VaultKey::from_hex(key),
        (None, Some(master_password)) => {
            let encrypted = store
                .encrypted_vault_key()?
                .context("local vault metadata is not initialized; run ltd vault init first")?;
            unwrap_vault_key(&encrypted, &master_password)
        }
        (None, None) => {
            let encrypted = store
                .encrypted_vault_key()?
                .context("local vault metadata is not initialized; run ltd vault init first")?;
            let master_password = prompt_master_password("Master password: ")?;
            unwrap_vault_key(&encrypted, &master_password)
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("use either --key or --master-password, not both")
        }
    }
}

fn prompt_new_master_password() -> Result<String> {
    let password = prompt_master_password("New master password: ")?;
    let confirmation = prompt_master_password("Confirm master password: ")?;
    if password != confirmation {
        anyhow::bail!("master passwords do not match");
    }
    if password.is_empty() {
        anyhow::bail!("master password cannot be empty");
    }
    Ok(password)
}

fn prompt_master_password(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).context("failed to read master password")
}

fn run_tui(store: TodoStore) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, App::new(store)?);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if handle_key(key, &mut app)? {
                return Ok(());
            }
        }
    }
}

fn handle_key(key: KeyEvent, app: &mut App) -> Result<bool> {
    match app.mode() {
        Mode::Browse => match key.code {
            KeyCode::Esc if app.help_visible() => app.hide_help(),
            KeyCode::Char('?') => app.toggle_help(),
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => app.move_down(),
            KeyCode::Char('k') | KeyCode::Up => app.move_up(),
            KeyCode::Char('[') => app.previous_project()?,
            KeyCode::Char(']') => app.next_project()?,
            KeyCode::Char('v') => app.toggle_view_mode(),
            KeyCode::Char(' ') => app.toggle_selected()?,
            KeyCode::Char('a') => app.start_add(),
            KeyCode::Char('e') => app.start_edit_title(),
            KeyCode::Char('n') => app.start_edit_note(),
            KeyCode::Char('d') => app.start_edit_due(),
            KeyCode::Char('t') => app.start_edit_tags(),
            KeyCode::Char('m') => app.start_move_project(),
            KeyCode::Char('/') => app.start_search(),
            KeyCode::Char('c') => app.clear_search()?,
            KeyCode::Char('x') => app.archive_selected()?,
            KeyCode::Char('r') => {
                app.refresh()?;
            }
            _ => {}
        },
        Mode::Add
        | Mode::EditTitle
        | Mode::EditNote
        | Mode::EditDue
        | Mode::EditTags
        | Mode::MoveProject
        | Mode::Search => match key.code {
            KeyCode::Esc => app.cancel_input(),
            KeyCode::Enter => app.submit_input()?,
            KeyCode::Backspace => app.pop_input(),
            KeyCode::Char(value) => app.push_input(value),
            _ => {}
        },
    }

    Ok(false)
}

fn print_tasks(tasks: Vec<Task>) -> Result<()> {
    if tasks.is_empty() {
        println!("No tasks");
    } else {
        for task in tasks {
            let marker = match task.status {
                TaskStatus::Open => "[ ]",
                TaskStatus::Done => "[x]",
                TaskStatus::Archived => "[-]",
            };
            let due = task
                .due_date
                .map(|date| format!(" due:{date}"))
                .unwrap_or_default();
            let tags = if task.tags.is_empty() {
                String::new()
            } else {
                format!(
                    " {}",
                    task.tags
                        .iter()
                        .map(|tag| format!("#{tag}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            };
            println!(
                "{} {} {}{}{}",
                marker,
                short_id(&task.id.to_string()),
                task.title,
                due,
                tags
            );
        }
    }

    Ok(())
}

fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .context("failed to locate user data directory")
        .map(|path| path.join("lemontodo").join("lemontodo.db"))
        .unwrap_or_else(|_| PathBuf::from(".lemontodo.db"))
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
