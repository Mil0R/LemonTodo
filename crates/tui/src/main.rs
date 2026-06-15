mod app;
mod ui;

use std::{collections::HashMap, fs, io, path::PathBuf, time::Duration};

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
use lemontodo_storage::{RemoteOperation, TodoStore};
use lemontodo_sync::{
    PROTOCOL_VERSION, PullRequest, PullResponse, PushRequest, PushResponse, pack_operations,
    unpack_operation,
};
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
    /// Show task completion statistics.
    Stats,
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
    /// Configure remote sync settings.
    Configure {
        #[arg(long)]
        server_url: String,
    },
    /// Show local sync state.
    Status,
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
    /// Upload pending encrypted operations to the configured server.
    Push {
        #[arg(long)]
        key: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this push.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Download and decrypt remote operations without applying them locally.
    Pull {
        #[arg(long)]
        key: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this pull.
        #[arg(long)]
        server_url: Option<String>,
        /// Maximum number of encrypted objects to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Print decrypted operations as pretty JSON.
        #[arg(long)]
        json: bool,
        /// Do not save pulled operations to the local pending-apply inbox.
        #[arg(long)]
        no_save: bool,
    },
    /// Inspect decrypted remote operations waiting for apply.
    Inbox {
        /// Print pending remote operations as pretty JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect pending remote operations that need manual conflict handling.
    Conflicts {
        /// Print conflicting remote operations as pretty JSON.
        #[arg(long)]
        json: bool,
    },
    /// Apply safe pending remote operations.
    Apply,
    /// Resolve a pending remote conflict explicitly.
    Resolve {
        /// Remote operation id prefix shown by `ltd sync inbox` or `ltd sync conflicts`.
        operation: String,
        /// Keep local data and ignore the conflicting remote operation.
        #[arg(long)]
        keep_local: bool,
        /// Discard pending local changes for the object and apply the remote version.
        #[arg(long)]
        keep_remote: bool,
    },
    /// Mark local pending operations as synced after a successful upload.
    Ack {
        /// Server cursor returned by a future sync endpoint.
        #[arg(long)]
        cursor: Option<String>,
        /// Mark every currently pending operation as synced.
        #[arg(long)]
        all_pending: bool,
        /// Operation id prefixes to mark as synced.
        operations: Vec<String>,
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
        Some(Command::Stats) => {
            print_stats(&store)?;
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
                        "{} {} {} rev:{} {}",
                        short_id(&operation.id.to_string()),
                        operation.object_type.as_str(),
                        operation.operation_type.as_str(),
                        operation.object_revision,
                        operation.object_id
                    );
                }
            }
        }
        Some(Command::Sync { command }) => match command {
            SyncCommand::Configure { server_url } => {
                store.save_sync_server_url(&server_url)?;
                println!(
                    "Configured sync server {}",
                    store.sync_server_url()?.unwrap()
                );
            }
            SyncCommand::Status => {
                let pending = store.pending_operations()?.len();
                let cursor = store
                    .last_sync_cursor()?
                    .unwrap_or_else(|| "<none>".to_owned());
                let server = store
                    .sync_server_url()?
                    .unwrap_or_else(|| "<not configured>".to_owned());
                println!("Device {}", store.device_id()?);
                println!("Server {server}");
                println!("Last cursor {cursor}");
                println!("Pending operations {pending}");
                println!(
                    "Pending remote operations {}",
                    store.pending_remote_operation_count()?
                );
            }
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
                let pack = pack_operations(&vault_key, store.device_id()?, &operations)?;
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
            SyncCommand::Push {
                key,
                master_password,
                server_url,
            } => {
                let pushed = push_pending_operations(
                    &store,
                    key.as_deref(),
                    master_password,
                    server_url.as_deref(),
                )?;
                println!(
                    "Pushed {} accepted, {} rejected, cursor {}",
                    pushed.accepted, pushed.rejected, pushed.cursor
                );
            }
            SyncCommand::Pull {
                key,
                master_password,
                server_url,
                limit,
                json,
                no_save,
            } => {
                let pulled = pull_remote_operations(
                    &store,
                    key.as_deref(),
                    master_password,
                    server_url.as_deref(),
                    limit,
                )?;
                let saved = if no_save {
                    0
                } else {
                    store.save_remote_operations(&pulled.operations, &pulled.cursor)?
                };
                if json {
                    println!("{}", serde_json::to_string_pretty(&pulled.operations)?);
                } else {
                    println!(
                        "Pulled {} object(s), saved {}, cursor {}, has_more {}",
                        pulled.operations.len(),
                        saved,
                        pulled.cursor,
                        pulled.has_more
                    );
                    for operation in &pulled.operations {
                        println!(
                            "{} {} {} rev:{} {}",
                            short_id(&operation.id.to_string()),
                            operation.object_type.as_str(),
                            operation.operation_type.as_str(),
                            operation.object_revision,
                            operation.object_id
                        );
                    }
                }
            }
            SyncCommand::Inbox { json } => {
                let pending = store.pending_remote_operations()?;
                if json {
                    let operations = pending
                        .iter()
                        .map(|remote| &remote.operation)
                        .collect::<Vec<_>>();
                    println!("{}", serde_json::to_string_pretty(&operations)?);
                } else if pending.is_empty() {
                    println!("No pending remote operations");
                } else {
                    print_remote_operations(&pending);
                }
            }
            SyncCommand::Conflicts { json } => {
                let conflicts = store.pending_remote_conflicts()?;
                if json {
                    let operations = conflicts
                        .iter()
                        .map(|remote| &remote.operation)
                        .collect::<Vec<_>>();
                    println!("{}", serde_json::to_string_pretty(&operations)?);
                } else if conflicts.is_empty() {
                    println!("No pending remote conflicts");
                } else {
                    print_remote_operations(&conflicts);
                }
            }
            SyncCommand::Apply => {
                let summary = store.apply_pending_remote_operations()?;
                println!(
                    "Applied {}, skipped {}, conflicts {}",
                    summary.applied, summary.skipped, summary.conflicts
                );
            }
            SyncCommand::Resolve {
                operation,
                keep_local,
                keep_remote,
            } => {
                if keep_local == keep_remote {
                    anyhow::bail!("use exactly one of --keep-local or --keep-remote");
                }
                let resolved = if keep_local {
                    store.resolve_remote_conflict_keep_local(&operation)?
                } else {
                    store.resolve_remote_conflict_keep_remote(&operation)?
                };
                println!(
                    "Resolved {} as {}",
                    short_id(&resolved.operation.id.to_string()),
                    resolved.apply_status
                );
            }
            SyncCommand::Ack {
                cursor,
                all_pending,
                operations,
            } => {
                if all_pending && !operations.is_empty() {
                    anyhow::bail!("use either --all-pending or operation ids, not both");
                }
                if !all_pending && operations.is_empty() {
                    anyhow::bail!("provide operation ids or use --all-pending");
                }

                let updated = if all_pending {
                    store.mark_pending_operations_synced(cursor.as_deref())?
                } else {
                    let operation_ids = store.operation_ids_by_prefixes(&operations)?;
                    store.mark_operations_synced(&operation_ids, cursor.as_deref())?
                };
                println!("Marked {updated} operation(s) as synced");
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

struct PushSummary {
    accepted: usize,
    rejected: usize,
    cursor: String,
}

struct PullSummary {
    operations: Vec<lemontodo_core::Operation>,
    cursor: String,
    has_more: bool,
}

fn push_pending_operations(
    store: &TodoStore,
    key: Option<&str>,
    master_password: Option<String>,
    server_url: Option<&str>,
) -> Result<PushSummary> {
    let server_url = configured_server_url(store, server_url)?;
    let vault_key = load_vault_key(store, key, master_password)?;
    let operations = store.pending_operations()?;
    if operations.is_empty() {
        let cursor = store
            .last_sync_cursor()?
            .unwrap_or_else(|| "<none>".to_owned());
        return Ok(PushSummary {
            accepted: 0,
            rejected: 0,
            cursor,
        });
    }

    let operation_ids = operations
        .iter()
        .map(|operation| operation.id)
        .collect::<Vec<_>>();
    let pack = pack_operations(&vault_key, store.device_id()?, &operations)?;
    let request = PushRequest::from_pack(store.last_sync_cursor()?, pack);
    let response = post_push_request(&server_url, &request)?;
    let accepted_ids = response
        .accepted
        .iter()
        .map(|accepted| accepted.operation_id)
        .collect::<std::collections::HashSet<_>>();
    let synced_operation_ids = operation_ids
        .into_iter()
        .filter(|operation_id| accepted_ids.contains(operation_id))
        .collect::<Vec<_>>();
    store.mark_operations_synced(&synced_operation_ids, Some(&response.cursor))?;

    Ok(PushSummary {
        accepted: response.accepted.len(),
        rejected: response.rejected.len(),
        cursor: response.cursor,
    })
}

fn pull_remote_operations(
    store: &TodoStore,
    key: Option<&str>,
    master_password: Option<String>,
    server_url: Option<&str>,
    limit: u32,
) -> Result<PullSummary> {
    let server_url = configured_server_url(store, server_url)?;
    let vault_key = load_vault_key(store, key, master_password)?;
    let request = PullRequest {
        protocol_version: PROTOCOL_VERSION,
        device_id: store.device_id()?,
        cursor: store.last_sync_cursor()?,
        limit,
    };
    let response = post_pull_request(&server_url, &request)?;
    let operations = response
        .objects
        .iter()
        .map(|object| unpack_operation(&vault_key, object))
        .collect::<Result<Vec<_>>>()?;

    Ok(PullSummary {
        operations,
        cursor: response.cursor,
        has_more: response.has_more,
    })
}

fn post_push_request(server_url: &str, request: &PushRequest) -> Result<PushResponse> {
    let endpoint = format!("{server_url}/v1/sync/push");
    let response = ureq::post(&endpoint)
        .content_type("application/json")
        .send_json(request)
        .with_context(|| format!("failed to POST {endpoint}"))?;
    response
        .into_body()
        .read_json::<PushResponse>()
        .context("failed to parse push response")
}

fn post_pull_request(server_url: &str, request: &PullRequest) -> Result<PullResponse> {
    let endpoint = format!("{server_url}/v1/sync/pull");
    let response = ureq::post(&endpoint)
        .content_type("application/json")
        .send_json(request)
        .with_context(|| format!("failed to POST {endpoint}"))?;
    response
        .into_body()
        .read_json::<PullResponse>()
        .context("failed to parse pull response")
}

fn configured_server_url(store: &TodoStore, override_url: Option<&str>) -> Result<String> {
    override_url
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .or(store.sync_server_url()?)
        .context("sync server is not configured; run ltd sync configure --server-url <url>")
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

#[derive(Debug, Default, Clone, Copy)]
struct TaskStats {
    open: usize,
    done: usize,
}

impl TaskStats {
    fn total(self) -> usize {
        self.open + self.done
    }

    fn add_task(&mut self, task: &Task) {
        match task.status {
            TaskStatus::Open => self.open += 1,
            TaskStatus::Done => self.done += 1,
            TaskStatus::Archived => {}
        }
    }

    fn completion_percent(self) -> usize {
        self.done
            .saturating_mul(100)
            .checked_div(self.total())
            .unwrap_or(0)
    }
}

fn print_remote_operations(operations: &[RemoteOperation]) {
    for remote in operations {
        let operation = &remote.operation;
        println!(
            "{} {} {} rev:{} {} cursor:{} status:{}{}",
            short_id(&operation.id.to_string()),
            operation.object_type.as_str(),
            operation.operation_type.as_str(),
            operation.object_revision,
            operation.object_id,
            remote.server_cursor,
            remote.apply_status,
            remote
                .apply_reason
                .as_deref()
                .map(|reason| format!(" reason:{reason}"))
                .unwrap_or_default()
        );
    }
}

fn print_stats(store: &TodoStore) -> Result<()> {
    let snapshot = store.export_snapshot()?;
    let mut project_stats = snapshot
        .lists
        .iter()
        .map(|project| (project.id, TaskStats::default()))
        .collect::<HashMap<_, _>>();
    let mut total = TaskStats::default();

    for task in &snapshot.tasks {
        if task.status == TaskStatus::Archived {
            continue;
        }
        total.add_task(task);
        project_stats
            .entry(task.list_id)
            .or_default()
            .add_task(task);
    }

    let name_width = snapshot
        .lists
        .iter()
        .map(|project| project.name.len())
        .chain(std::iter::once("Total".len()))
        .max()
        .unwrap_or("Total".len());

    print_stat_line("Total", total, name_width);
    for project in snapshot.lists {
        let stats = project_stats.remove(&project.id).unwrap_or_default();
        print_stat_line(&project.name, stats, name_width);
    }

    Ok(())
}

fn print_stat_line(name: &str, stats: TaskStats, name_width: usize) {
    println!(
        "{name:<name_width$}  {} {:>3}%  {}/{} done",
        progress_bar(stats, 24),
        stats.completion_percent(),
        stats.done,
        stats.total(),
    );
}

fn progress_bar(stats: TaskStats, width: usize) -> String {
    let filled = stats
        .done
        .saturating_mul(width)
        .checked_div(stats.total())
        .unwrap_or(0);
    format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(width.saturating_sub(filled))
    )
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
