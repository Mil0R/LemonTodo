mod app;
mod ui;

use std::{
    collections::HashMap,
    fs, io,
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use lemontodo_core::{NewTask, Task, TaskStatus};
use lemontodo_crypto::{KdfParams, VaultKey, derive_auth_hash, unwrap_vault_key, wrap_vault_key};
use lemontodo_storage::{RemoteOperation, TodoStore};
use lemontodo_sync::{
    AccountStatusResponse, LoginRequest, LoginResponse, LogoutRequest, LogoutResponse,
    PROTOCOL_VERSION, PullRequest, PullResponse, PushRequest, PushResponse,
    PutVaultMetadataRequest, RegisterRequest, RegisterResponse, RevokeSessionRequest,
    RevokeSessionResponse, SessionsResponse, VaultMetadataResponse, pack_operations,
    unpack_operation,
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::{App, Mode, parse_due_input},
    ui::draw,
};

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8787";
const TUI_AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(60);
const TUI_AUTO_SYNC_DEBOUNCE: Duration = Duration::from_secs(5);
const TUI_AUTO_SYNC_LIMIT: u32 = 100;

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
    #[command(hide = true)]
    Done { id: String },
    /// Edit a task title by id prefix.
    #[command(hide = true)]
    Edit { id: String, title: String },
    /// Edit a task note by id prefix.
    #[command(hide = true)]
    Note { id: String, note: String },
    /// Set or clear a task due date by id prefix.
    #[command(hide = true)]
    Due {
        id: String,
        #[arg(value_name = "YYYY-MM-DD")]
        date: Option<String>,
    },
    /// Replace task tags by id prefix.
    #[command(hide = true)]
    Tags { id: String, tags: Vec<String> },
    /// Move a task to a project by id prefix.
    #[command(hide = true)]
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
    /// Log into a LemonTodo server account.
    Login {
        #[arg(long)]
        email: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the default server URL for this login.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Log out from the current LemonTodo server account.
    Logout {
        /// Only clear the local token without calling the server.
        #[arg(long)]
        local_only: bool,
        /// Revoke every active server session for the current account.
        #[arg(long)]
        all: bool,
        /// Override the configured server URL for this logout.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Sync local and remote task data.
    Sync {
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Maximum number of encrypted objects to fetch before pushing.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Leave safe pulled remote operations in the inbox instead of applying them.
        #[arg(long)]
        no_apply_safe: bool,
        #[command(subcommand)]
        command: Option<SyncCommand>,
    },
    /// Manage the local encrypted vault metadata.
    #[command(hide = true)]
    Vault {
        #[command(subcommand)]
        command: VaultCommand,
    },
    /// Archive a task by id prefix.
    #[command(hide = true)]
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
    #[command(hide = true)]
    Configure {
        #[arg(long)]
        server_url: String,
        #[arg(long)]
        email: Option<String>,
    },
    /// Register a server account and save the account email locally.
    #[command(hide = true)]
    Register {
        #[arg(long)]
        email: String,
        /// Development/script compatibility. Prefer server-side registration.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this registration.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Log into a server account and save the access token locally.
    #[command(hide = true)]
    Login {
        #[arg(long)]
        email: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this login.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Connect a new device by logging in, downloading vault metadata, and verifying the master password.
    #[command(hide = true)]
    Connect {
        #[arg(long)]
        email: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this device connection.
        #[arg(long)]
        server_url: Option<String>,
        /// Overwrite existing local vault metadata.
        #[arg(long)]
        force: bool,
        /// Pull encrypted remote operations immediately after a successful connection.
        #[arg(long)]
        pull: bool,
        /// Maximum number of encrypted objects to fetch when using --pull.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Apply safe pulled remote operations immediately after saving them to the local inbox.
        #[arg(long)]
        apply_safe: bool,
    },
    /// Revoke the current server session and clear the local access token.
    #[command(hide = true)]
    Logout {
        /// Only clear the local token without calling the server.
        #[arg(long)]
        local_only: bool,
        /// Revoke every active server session for the current account.
        #[arg(long)]
        all: bool,
        /// Override the configured server URL for this logout.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Show local sync state.
    Status,
    /// Show the current authenticated server account for the stored access token.
    #[command(hide = true)]
    Whoami {
        /// Print the server response as pretty JSON.
        #[arg(long)]
        json: bool,
        /// Override the configured server URL for this request.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// List active server sessions for the current account.
    #[command(hide = true)]
    Sessions {
        /// Print the server response as pretty JSON.
        #[arg(long)]
        json: bool,
        /// Override the configured server URL for this request.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Revoke an active server session by id prefix from `ltd sync sessions`.
    #[command(hide = true)]
    Revoke {
        session: String,
        /// Override the configured server URL for this request.
        #[arg(long)]
        server_url: Option<String>,
    },
    /// Generate a random local vault key as hex.
    #[command(hide = true)]
    Keygen,
    /// Pack pending operations into encrypted sync objects.
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    /// Pull remote changes, optionally apply safe ones, then push local changes.
    #[command(hide = true)]
    Now {
        #[arg(long)]
        key: Option<String>,
        /// Development/script compatibility. Prefer hidden prompt.
        #[arg(long)]
        master_password: Option<String>,
        /// Override the configured server URL for this sync run.
        #[arg(long)]
        server_url: Option<String>,
        /// Maximum number of encrypted objects to fetch before pushing.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Apply safe pulled remote operations before pushing local changes.
        #[arg(long)]
        apply_safe: bool,
    },
    /// Upload local encrypted vault metadata to the server account.
    #[command(hide = true)]
    VaultPush,
    /// Download encrypted vault metadata from the server account.
    #[command(hide = true)]
    VaultPull {
        #[arg(long)]
        force: bool,
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
    #[command(hide = true)]
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
    let db_path = cli
        .db
        .unwrap_or_else(|| default_db_path_for_command(&cli.command));
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
        Some(Command::Login {
            email,
            master_password,
            server_url,
        }) => {
            run_login_command(&store, email, master_password, server_url)?;
        }
        Some(Command::Logout {
            local_only,
            all,
            server_url,
        }) => {
            run_logout_command(&store, local_only, all, server_url)?;
        }
        Some(Command::Sync {
            master_password,
            limit,
            no_apply_safe,
            command,
        }) => match command {
            None => {
                let summary = sync_now(&store, None, master_password, None, limit, !no_apply_safe)?;
                print_sync_now_summary(summary);
            }
            Some(command) => match command {
                SyncCommand::Configure { server_url, email } => {
                    store.save_sync_server_url(&server_url)?;
                    if let Some(email) = email {
                        store.save_sync_account_email(&email)?;
                    }
                    println!(
                        "Configured sync server {}{}",
                        store.sync_server_url()?.unwrap(),
                        store
                            .sync_account_email()?
                            .map(|email| format!(" account {email}"))
                            .unwrap_or_default()
                    );
                }
                SyncCommand::Register {
                    email,
                    master_password,
                    server_url,
                } => {
                    let master_password = master_password
                        .map(Ok)
                        .unwrap_or_else(prompt_new_master_password)?;
                    let response =
                        register_account(&store, server_url.as_deref(), &email, &master_password)?;
                    store.save_sync_account_email(&response.email)?;
                    println!("Registered account {}", response.email);
                }
                SyncCommand::Login {
                    email,
                    master_password,
                    server_url,
                } => {
                    run_login_command(&store, email, master_password, server_url)?;
                }
                SyncCommand::Connect {
                    email,
                    master_password,
                    server_url,
                    force,
                    pull,
                    limit,
                    apply_safe,
                } => {
                    if apply_safe && !pull {
                        anyhow::bail!("--apply-safe requires --pull");
                    }
                    let email = email
                        .or(store.sync_account_email()?)
                        .context("sync account email is not configured; pass --email")?;
                    let master_password = master_password
                        .map(Ok)
                        .unwrap_or_else(|| prompt_master_password("Master password: "))?;
                    let connected = connect_device(
                        &store,
                        DeviceConnectOptions {
                            server_url: server_url.as_deref(),
                            email: &email,
                            master_password: &master_password,
                            force,
                            pull,
                            limit,
                            apply_safe,
                        },
                    )?;
                    println!("Connected device for {}", connected.email);
                    if let Some(pulled) = connected.pulled {
                        println!(
                            "Pulled {} object(s), saved {}, cursor {}, has_more {}",
                            pulled.fetched, pulled.saved, pulled.cursor, pulled.has_more
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
                        if let Some(applied) = pulled.applied {
                            println!(
                                "Applied {}, skipped {}, conflicts {}",
                                applied.applied, applied.skipped, applied.conflicts
                            );
                        }
                    }
                }
                SyncCommand::Logout {
                    local_only,
                    all,
                    server_url,
                } => {
                    run_logout_command(&store, local_only, all, server_url)?;
                }
                SyncCommand::Status => {
                    let pending = store.pending_operations()?.len();
                    let cursor = store
                        .last_sync_cursor()?
                        .unwrap_or_else(|| "<none>".to_owned());
                    let server = store
                        .sync_server_url()?
                        .unwrap_or_else(|| "<not configured>".to_owned());
                    let email = store
                        .sync_account_email()?
                        .unwrap_or_else(|| "<not configured>".to_owned());
                    let token = if store.sync_access_token()?.is_some() {
                        "configured"
                    } else {
                        "<not configured>"
                    };
                    let vault_metadata = if store.encrypted_vault_key()?.is_some() {
                        "configured"
                    } else {
                        "<not initialized>"
                    };
                    println!("Device {}", store.device_id()?);
                    println!("Server {server}");
                    println!("Account {email}");
                    println!("Access token {token}");
                    println!("Vault metadata {vault_metadata}");
                    println!("Last cursor {cursor}");
                    println!("Pending operations {pending}");
                    println!(
                        "Pending remote operations {}",
                        store.pending_remote_operation_count()?
                    );
                    if store.sync_server_url()?.is_some() && store.sync_access_token()?.is_some() {
                        match fetch_account_status(&store, None) {
                            Ok(status) => print_remote_account_status(&status),
                            Err(error) => println!("Remote status error {error}"),
                        }
                    } else {
                        println!("Remote account <unavailable>");
                    }
                }
                SyncCommand::Whoami { json, server_url } => {
                    let status = fetch_account_status(&store, server_url.as_deref())?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&status)?);
                    } else {
                        print_remote_account_status(&status);
                    }
                }
                SyncCommand::Sessions { json, server_url } => {
                    let sessions = fetch_sessions(&store, server_url.as_deref())?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&sessions)?);
                    } else {
                        print_sessions(&sessions);
                    }
                }
                SyncCommand::Revoke {
                    session,
                    server_url,
                } => {
                    let response =
                        revoke_session_by_prefix(&store, server_url.as_deref(), &session)?;
                    if response.current {
                        store.clear_sync_access_token()?;
                    }
                    println!(
                        "Revoked session {}{}",
                        response.session_id,
                        if response.current {
                            " and cleared local token"
                        } else {
                            ""
                        }
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
                        fs::write(&path, json).with_context(|| {
                            format!("failed to write sync pack {}", path.display())
                        })?;
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
                SyncCommand::Now {
                    key,
                    master_password,
                    server_url,
                    limit,
                    apply_safe,
                } => {
                    let summary = sync_now(
                        &store,
                        key.as_deref(),
                        master_password,
                        server_url.as_deref(),
                        limit,
                        apply_safe,
                    )?;
                    print_sync_now_summary(summary);
                }
                SyncCommand::VaultPush => {
                    push_vault_metadata(&store)?;
                    println!("Uploaded encrypted vault metadata");
                }
                SyncCommand::VaultPull { force } => {
                    let downloaded = pull_vault_metadata(&store, force)?;
                    if downloaded {
                        println!("Downloaded encrypted vault metadata");
                    } else {
                        println!("Server account has no encrypted vault metadata");
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
                        print_remote_operations(&store, &pending)?;
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
                        print_remote_operations(&store, &conflicts)?;
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

struct SyncNowSummary {
    pulled: PullSummary,
    saved: usize,
    applied: Option<lemontodo_storage::RemoteApplySummary>,
    pushed: PushSummary,
}

struct DeviceConnectSummary {
    email: String,
    pulled: Option<ConnectPullSummary>,
}

struct DeviceConnectOptions<'a> {
    server_url: Option<&'a str>,
    email: &'a str,
    master_password: &'a str,
    force: bool,
    pull: bool,
    limit: u32,
    apply_safe: bool,
}

struct ConnectPullSummary {
    operations: Vec<lemontodo_core::Operation>,
    fetched: usize,
    saved: usize,
    cursor: String,
    has_more: bool,
    applied: Option<lemontodo_storage::RemoteApplySummary>,
}

fn push_pending_operations(
    store: &TodoStore,
    key: Option<&str>,
    master_password: Option<String>,
    server_url: Option<&str>,
) -> Result<PushSummary> {
    let server_url = configured_server_url(store, server_url)?;
    let vault_key = load_vault_key(store, key, master_password)?;
    push_pending_operations_with_vault_key(store, &vault_key, &server_url)
}

fn push_pending_operations_with_vault_key(
    store: &TodoStore,
    vault_key: &VaultKey,
    server_url: &str,
) -> Result<PushSummary> {
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
    let pack = pack_operations(vault_key, store.device_id()?, &operations)?;
    let request = PushRequest::from_pack(
        configured_access_token(store)?,
        store.last_sync_cursor()?,
        pack,
    );
    let response = post_push_request(server_url, &request)?;
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
    pull_remote_operations_with_vault_key(store, &vault_key, &server_url, limit)
}

fn pull_remote_operations_with_vault_key(
    store: &TodoStore,
    vault_key: &VaultKey,
    server_url: &str,
    limit: u32,
) -> Result<PullSummary> {
    let request = PullRequest {
        protocol_version: PROTOCOL_VERSION,
        device_id: store.device_id()?,
        access_token: configured_access_token(store)?,
        cursor: store.last_sync_cursor()?,
        limit,
    };
    let response = post_pull_request(server_url, &request)?;
    let operations = response
        .objects
        .iter()
        .map(|object| unpack_operation(vault_key, object))
        .collect::<Result<Vec<_>>>()?;

    Ok(PullSummary {
        operations,
        cursor: response.cursor,
        has_more: response.has_more,
    })
}

fn sync_now(
    store: &TodoStore,
    key: Option<&str>,
    master_password: Option<String>,
    server_url: Option<&str>,
    limit: u32,
    apply_safe: bool,
) -> Result<SyncNowSummary> {
    let server_url = configured_server_url(store, server_url)?;
    let vault_key = load_vault_key(store, key, master_password)?;
    sync_now_with_vault_key(store, &vault_key, &server_url, limit, apply_safe)
}

fn sync_now_with_vault_key(
    store: &TodoStore,
    vault_key: &VaultKey,
    server_url: &str,
    limit: u32,
    apply_safe: bool,
) -> Result<SyncNowSummary> {
    let pulled = pull_remote_operations_with_vault_key(store, vault_key, server_url, limit)?;
    let saved = store.save_remote_operations(&pulled.operations, &pulled.cursor)?;
    let applied = if apply_safe {
        Some(store.apply_pending_remote_operations()?)
    } else {
        None
    };
    let pushed = push_pending_operations_with_vault_key(store, vault_key, server_url)?;

    Ok(SyncNowSummary {
        pulled,
        saved,
        applied,
        pushed,
    })
}

fn run_login_command(
    store: &TodoStore,
    email: Option<String>,
    master_password: Option<String>,
    server_url: Option<String>,
) -> Result<()> {
    let server_url = match server_url {
        Some(server_url) => Some(server_url),
        None => {
            let default = store
                .sync_server_url()?
                .unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned());
            if io::stdin().is_terminal() {
                let input = prompt_text(&format!("Server [{default}]: "))?;
                if input.is_empty() {
                    Some(default)
                } else {
                    Some(input)
                }
            } else {
                Some(default)
            }
        }
    };
    let email = match email.or(store.sync_account_email()?) {
        Some(email) => email,
        None => prompt_text("Account email: ")?,
    };
    if email.trim().is_empty() {
        anyhow::bail!("account email cannot be empty");
    }
    let master_password = master_password
        .map(Ok)
        .unwrap_or_else(|| prompt_master_password("Master password: "))?;
    let response = login_account(store, server_url.as_deref(), &email, &master_password)?;
    let vault_metadata =
        fetch_vault_metadata_for_token(store, server_url.as_deref(), &response.access_token)?;
    verify_or_save_vault_metadata(store, &vault_metadata, &master_password)?;
    store.save_sync_server_url(server_url.as_deref().unwrap_or(DEFAULT_SERVER_URL))?;
    store.save_sync_account_email(&response.email)?;
    store.save_sync_access_token(&response.access_token)?;
    println!("Logged in as {}", response.email);
    Ok(())
}

fn run_logout_command(
    store: &TodoStore,
    local_only: bool,
    all: bool,
    server_url: Option<String>,
) -> Result<()> {
    if local_only && all {
        bail!("use either --local-only or --all, not both");
    }
    let token = store
        .sync_access_token()?
        .context("sync access token is not configured; run ltd login")?;
    if all {
        let revoked = logout_all_sessions(store, server_url.as_deref())?;
        store.clear_sync_access_token()?;
        println!("Logged out and revoked {revoked} remote session(s)");
    } else {
        let revoked = if local_only {
            false
        } else {
            logout_account(store, server_url.as_deref(), &token)?.revoked
        };
        store.clear_sync_access_token()?;
        println!(
            "Logged out{}",
            if revoked {
                " and revoked remote session"
            } else {
                ""
            }
        );
    }
    Ok(())
}

fn print_sync_now_summary(summary: SyncNowSummary) {
    println!(
        "Pulled {} object(s), saved {}, cursor {}, has_more {}",
        summary.pulled.operations.len(),
        summary.saved,
        summary.pulled.cursor,
        summary.pulled.has_more
    );
    if let Some(applied) = summary.applied {
        println!(
            "Applied {}, skipped {}, conflicts {}",
            applied.applied, applied.skipped, applied.conflicts
        );
    }
    println!(
        "Pushed {} accepted, {} rejected, cursor {}",
        summary.pushed.accepted, summary.pushed.rejected, summary.pushed.cursor
    );
}

fn push_vault_metadata(store: &TodoStore) -> Result<()> {
    let server_url = configured_server_url(store, None)?;
    let access_token = configured_access_token(store)?;
    let encrypted_vault_key = store
        .encrypted_vault_key()?
        .context("local vault metadata is not initialized; run ltd vault init first")?;
    put_vault_metadata_request(
        &server_url,
        &PutVaultMetadataRequest {
            access_token,
            encrypted_vault_key,
        },
    )?;
    Ok(())
}

fn pull_vault_metadata(store: &TodoStore, force: bool) -> Result<bool> {
    ensure_vault_metadata_write_allowed(store, force)?;
    let server_url = configured_server_url(store, None)?;
    let access_token = configured_access_token(store)?;
    let response = get_vault_metadata_request(&server_url, &access_token)?;
    let Some(encrypted_vault_key) = response.encrypted_vault_key else {
        return Ok(false);
    };
    store.save_encrypted_vault_key(&encrypted_vault_key)?;
    Ok(true)
}

fn fetch_vault_metadata_for_token(
    store: &TodoStore,
    server_url: Option<&str>,
    access_token: &str,
) -> Result<lemontodo_crypto::EncryptedVaultKey> {
    let server_url = configured_server_url(store, server_url)?;
    let response = get_vault_metadata_request(&server_url, access_token)?;
    response.encrypted_vault_key.context(
        "server account has no encrypted vault metadata; register the account on the server first",
    )
}

fn verify_or_save_vault_metadata(
    store: &TodoStore,
    remote: &lemontodo_crypto::EncryptedVaultKey,
    master_password: &str,
) -> Result<()> {
    unwrap_vault_key(remote, master_password)
        .context("master password could not unlock the server vault metadata")?;
    if let Some(local) = store.encrypted_vault_key()? {
        unwrap_vault_key(&local, master_password)
            .context("master password could not unlock the existing local vault metadata")?;
    } else {
        store.save_encrypted_vault_key(remote)?;
    }
    Ok(())
}

fn connect_device(
    store: &TodoStore,
    options: DeviceConnectOptions<'_>,
) -> Result<DeviceConnectSummary> {
    let server_url = configured_server_url(store, options.server_url)?;
    let login = post_login_request(
        &server_url,
        &LoginRequest {
            email: options.email.to_owned(),
            auth_hash: derive_auth_hash(options.email, options.master_password)?,
            device_id: store.device_id()?,
            device_name: inferred_device_name(),
        },
    )?;
    let response = get_vault_metadata_request(&server_url, &login.access_token)?;
    let encrypted_vault_key = response.encrypted_vault_key.context(
        "server account has no encrypted vault metadata; register the account on the server first",
    )?;
    import_remote_vault_metadata(
        store,
        &encrypted_vault_key,
        options.force,
        options.master_password,
    )?;
    store.save_sync_server_url(&server_url)?;
    store.save_sync_account_email(&login.email)?;
    store.save_sync_access_token(&login.access_token)?;
    let pulled = if options.pull {
        let pulled = pull_remote_operations(
            store,
            None,
            Some(options.master_password.to_owned()),
            None,
            options.limit,
        )?;
        let saved = store.save_remote_operations(&pulled.operations, &pulled.cursor)?;
        let applied = if options.apply_safe {
            Some(store.apply_pending_remote_operations()?)
        } else {
            None
        };
        Some(ConnectPullSummary {
            fetched: pulled.operations.len(),
            saved,
            cursor: pulled.cursor.clone(),
            has_more: pulled.has_more,
            operations: pulled.operations,
            applied,
        })
    } else {
        None
    };
    Ok(DeviceConnectSummary {
        email: login.email,
        pulled,
    })
}

fn register_account(
    store: &TodoStore,
    server_url: Option<&str>,
    email: &str,
    master_password: &str,
) -> Result<RegisterResponse> {
    let server_url = configured_server_url(store, server_url)?;
    let vault_key = VaultKey::generate();
    let encrypted_vault_key = wrap_vault_key(
        &vault_key,
        master_password,
        KdfParams::generate_interactive(),
    )?;
    unwrap_vault_key(&encrypted_vault_key, master_password)?;
    store.save_encrypted_vault_key(&encrypted_vault_key)?;
    post_register_request(
        &server_url,
        &RegisterRequest {
            email: email.to_owned(),
            auth_hash: derive_auth_hash(email, master_password)?,
            encrypted_vault_key,
        },
    )
}

fn login_account(
    store: &TodoStore,
    server_url: Option<&str>,
    email: &str,
    master_password: &str,
) -> Result<LoginResponse> {
    let server_url = configured_server_url(store, server_url)?;
    post_login_request(
        &server_url,
        &LoginRequest {
            email: email.to_owned(),
            auth_hash: derive_auth_hash(email, master_password)?,
            device_id: store.device_id()?,
            device_name: inferred_device_name(),
        },
    )
}

fn logout_account(
    store: &TodoStore,
    server_url: Option<&str>,
    access_token: &str,
) -> Result<LogoutResponse> {
    let server_url = configured_server_url(store, server_url)?;
    post_logout_request(
        &server_url,
        &LogoutRequest {
            access_token: access_token.to_owned(),
        },
    )
}

fn logout_all_sessions(store: &TodoStore, server_url: Option<&str>) -> Result<usize> {
    let sessions = fetch_sessions(store, server_url)?;
    let mut revoked = 0;
    for session in sessions.sessions.iter().filter(|session| !session.current) {
        let response = revoke_session_by_prefix(store, server_url, &session.session_id)?;
        if response.revoked {
            revoked += 1;
        }
    }
    for session in sessions.sessions.iter().filter(|session| session.current) {
        let response = revoke_session_by_prefix(store, server_url, &session.session_id)?;
        if response.revoked {
            revoked += 1;
        }
    }
    Ok(revoked)
}

fn fetch_account_status(
    store: &TodoStore,
    server_url: Option<&str>,
) -> Result<AccountStatusResponse> {
    let server_url = configured_server_url(store, server_url)?;
    let access_token = configured_access_token(store)?;
    get_account_status_request(&server_url, &access_token)
}

fn fetch_sessions(store: &TodoStore, server_url: Option<&str>) -> Result<SessionsResponse> {
    let server_url = configured_server_url(store, server_url)?;
    let access_token = configured_access_token(store)?;
    get_sessions_request(&server_url, &access_token)
}

fn revoke_session_by_prefix(
    store: &TodoStore,
    server_url: Option<&str>,
    session_id: &str,
) -> Result<RevokeSessionResponse> {
    let server_url = configured_server_url(store, server_url)?;
    let access_token = configured_access_token(store)?;
    post_revoke_session_request(
        &server_url,
        &RevokeSessionRequest {
            access_token,
            session_id: session_id.to_owned(),
        },
    )
}

fn print_remote_account_status(status: &AccountStatusResponse) {
    println!("Remote user {}", status.email);
    println!("Remote user id {}", status.user_id);
    println!("Remote admin {}", status.is_admin);
    println!("Remote vault key {}", status.has_vault_key);
    println!(
        "Remote user created {}",
        status.user_created_at.to_rfc3339()
    );
    println!(
        "Remote session created {}",
        status.session_created_at.to_rfc3339()
    );
    println!(
        "Remote session last used {}",
        status.session_last_used_at.to_rfc3339()
    );
    println!(
        "Remote session device id {}",
        status
            .session_device_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned())
    );
    println!(
        "Remote session device name {}",
        status
            .session_device_name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_owned())
    );
}

fn print_sessions(response: &SessionsResponse) {
    if response.sessions.is_empty() {
        println!("No active sessions");
        return;
    }
    for session in &response.sessions {
        let current = if session.current { "current" } else { "active" };
        let device_id = session
            .device_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned());
        let device_name = session
            .device_name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_owned());
        println!(
            "{} {} {} {} created:{} last-used:{}",
            session.session_id,
            current,
            device_name,
            device_id,
            session.created_at.to_rfc3339(),
            session.last_used_at.to_rfc3339()
        );
    }
}

fn auth_hint_from_message(message: &str) -> Option<&'static str> {
    if message.contains("expired access token") {
        Some("server rejected the stored access token as expired; run ltd login again")
    } else if message.contains("invalid access token") {
        Some("server rejected the stored access token; run ltd login again")
    } else {
        None
    }
}

fn checked_response(
    endpoint: &str,
    response: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ureq::http::Response<ureq::Body>> {
    match response {
        Ok(response) => Ok(response),
        Err(ureq::Error::StatusCode(code)) => {
            let message = format!("request failed with HTTP {code} at {endpoint}");
            if code == 401 {
                bail!("{message}: run ltd login again");
            }
            bail!("{message}");
        }
        Err(error) => Err(anyhow!(error)).with_context(|| format!("request failed for {endpoint}")),
    }
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn inferred_device_name() -> String {
    std::env::var("LEMONTODO_DEVICE_NAME")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown-device".to_owned())
}

fn checked_json_response<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    response: ureq::http::Response<ureq::Body>,
    parse_context: &str,
) -> Result<T> {
    let status = response.status().as_u16();
    let mut body = response.into_body();
    if (200..300).contains(&status) {
        return body
            .read_json::<T>()
            .with_context(|| parse_context.to_owned());
    }
    let message = body.read_to_string().unwrap_or_default();
    if let Some(hint) = auth_hint_from_message(&message) {
        bail!("{hint}");
    }
    let detail = if message.trim().is_empty() {
        format!("request failed with HTTP {status} at {endpoint}")
    } else {
        format!(
            "request failed with HTTP {status} at {endpoint}: {}",
            message.trim()
        )
    };
    bail!("{detail}");
}

fn post_push_request(server_url: &str, request: &PushRequest) -> Result<PushResponse> {
    let endpoint = format!("{server_url}/v1/sync/push");
    let response = checked_response(
        &endpoint,
        http_agent()
            .post(&endpoint)
            .content_type("application/json")
            .send_json(request),
    )?;
    checked_json_response(&endpoint, response, "failed to parse push response")
}

fn post_pull_request(server_url: &str, request: &PullRequest) -> Result<PullResponse> {
    let endpoint = format!("{server_url}/v1/sync/pull");
    let response = checked_response(
        &endpoint,
        http_agent()
            .post(&endpoint)
            .content_type("application/json")
            .send_json(request),
    )?;
    checked_json_response(&endpoint, response, "failed to parse pull response")
}

fn post_register_request(server_url: &str, request: &RegisterRequest) -> Result<RegisterResponse> {
    let endpoint = format!("{server_url}/v1/account/register");
    let response = checked_response(&endpoint, http_agent().post(&endpoint).send_json(request))?;
    checked_json_response(&endpoint, response, "failed to parse register response")
}

fn post_login_request(server_url: &str, request: &LoginRequest) -> Result<LoginResponse> {
    let endpoint = format!("{server_url}/v1/account/login");
    let response = checked_response(&endpoint, http_agent().post(&endpoint).send_json(request))?;
    checked_json_response(&endpoint, response, "failed to parse login response")
}

fn post_logout_request(server_url: &str, request: &LogoutRequest) -> Result<LogoutResponse> {
    let endpoint = format!("{server_url}/v1/account/logout");
    let response = checked_response(&endpoint, http_agent().post(&endpoint).send_json(request))?;
    checked_json_response(&endpoint, response, "failed to parse logout response")
}

fn post_revoke_session_request(
    server_url: &str,
    request: &RevokeSessionRequest,
) -> Result<RevokeSessionResponse> {
    let endpoint = format!("{server_url}/v1/account/sessions/revoke");
    let response = checked_response(&endpoint, http_agent().post(&endpoint).send_json(request))?;
    checked_json_response(
        &endpoint,
        response,
        "failed to parse revoke session response",
    )
}

fn get_account_status_request(
    server_url: &str,
    access_token: &str,
) -> Result<AccountStatusResponse> {
    let endpoint = format!("{server_url}/v1/account/me");
    let response = checked_response(
        &endpoint,
        http_agent()
            .get(&endpoint)
            .query("access_token", access_token)
            .call(),
    )?;
    checked_json_response(
        &endpoint,
        response,
        "failed to parse account status response",
    )
}

fn get_sessions_request(server_url: &str, access_token: &str) -> Result<SessionsResponse> {
    let endpoint = format!("{server_url}/v1/account/sessions");
    let response = checked_response(
        &endpoint,
        http_agent()
            .get(&endpoint)
            .query("access_token", access_token)
            .call(),
    )?;
    checked_json_response(&endpoint, response, "failed to parse sessions response")
}

fn put_vault_metadata_request(
    server_url: &str,
    request: &PutVaultMetadataRequest,
) -> Result<VaultMetadataResponse> {
    let endpoint = format!("{server_url}/v1/account/vault-key");
    let response = checked_response(&endpoint, http_agent().put(&endpoint).send_json(request))?;
    checked_json_response(
        &endpoint,
        response,
        "failed to parse vault metadata response",
    )
}

fn get_vault_metadata_request(
    server_url: &str,
    access_token: &str,
) -> Result<VaultMetadataResponse> {
    let endpoint = format!("{server_url}/v1/account/vault-key");
    let response = checked_response(
        &endpoint,
        http_agent()
            .get(&endpoint)
            .query("access_token", access_token)
            .call(),
    )?;
    checked_json_response(
        &endpoint,
        response,
        "failed to parse vault metadata response",
    )
}

fn configured_server_url(store: &TodoStore, override_url: Option<&str>) -> Result<String> {
    override_url
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .or(store.sync_server_url()?)
        .or_else(|| Some(DEFAULT_SERVER_URL.to_owned()))
        .context("sync server is not configured")
}

fn configured_access_token(store: &TodoStore) -> Result<String> {
    store
        .sync_access_token()?
        .context("sync access token is not configured; run ltd login")
}

fn ensure_vault_metadata_write_allowed(store: &TodoStore, force: bool) -> Result<()> {
    if store.encrypted_vault_key()?.is_some() && !force {
        anyhow::bail!("local vault metadata already exists; use --force to overwrite");
    }
    Ok(())
}

fn import_remote_vault_metadata(
    store: &TodoStore,
    encrypted_vault_key: &lemontodo_crypto::EncryptedVaultKey,
    force: bool,
    master_password: &str,
) -> Result<()> {
    ensure_vault_metadata_write_allowed(store, force)?;
    unwrap_vault_key(encrypted_vault_key, master_password)
        .context("master password could not unlock downloaded vault metadata")?;
    store.save_encrypted_vault_key(encrypted_vault_key)?;
    Ok(())
}

fn unlock_tui_auto_sync(store: &TodoStore) -> Result<Option<VaultKey>> {
    if store.sync_server_url()?.is_none()
        || store.sync_access_token()?.is_none()
        || store.encrypted_vault_key()?.is_none()
        || !io::stdin().is_terminal()
    {
        return Ok(None);
    }

    let master_password =
        prompt_master_password("Master password for TUI auto-sync (empty skips): ")?;
    if master_password.is_empty() {
        println!("TUI auto-sync disabled");
        return Ok(None);
    }

    let encrypted = store
        .encrypted_vault_key()?
        .context("local vault metadata is not initialized; run ltd login first")?;
    match unwrap_vault_key(&encrypted, &master_password) {
        Ok(vault_key) => Ok(Some(vault_key)),
        Err(error) => {
            println!("TUI auto-sync disabled: {error}");
            Ok(None)
        }
    }
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

fn prompt_text(prompt: &str) -> Result<String> {
    use std::io::Write;

    print!("{prompt}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    Ok(input.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use lemontodo_crypto::{KdfParams, VaultKey, wrap_vault_key};
    use lemontodo_storage::TodoStore;
    use tempfile::tempdir;

    use crate::import_remote_vault_metadata;

    #[test]
    fn import_remote_vault_metadata_saves_when_master_password_matches() -> Result<()> {
        let tempdir = tempdir()?;
        let store = TodoStore::open(tempdir.path().join("lemontodo.db"))?;
        let encrypted = wrap_vault_key(
            &VaultKey::generate(),
            "correct horse battery staple",
            KdfParams::generate_interactive(),
        )?;

        import_remote_vault_metadata(&store, &encrypted, false, "correct horse battery staple")?;

        assert_eq!(store.encrypted_vault_key()?, Some(encrypted));
        Ok(())
    }

    #[test]
    fn import_remote_vault_metadata_rejects_wrong_master_password() -> Result<()> {
        let tempdir = tempdir()?;
        let store = TodoStore::open(tempdir.path().join("lemontodo.db"))?;
        let encrypted = wrap_vault_key(
            &VaultKey::generate(),
            "correct horse battery staple",
            KdfParams::generate_interactive(),
        )?;

        let error =
            import_remote_vault_metadata(&store, &encrypted, false, "wrong password").unwrap_err();

        assert!(
            error
                .to_string()
                .contains("master password could not unlock downloaded vault metadata")
        );
        assert!(store.encrypted_vault_key()?.is_none());
        Ok(())
    }

    #[test]
    fn import_remote_vault_metadata_requires_force_to_overwrite() -> Result<()> {
        let tempdir = tempdir()?;
        let store = TodoStore::open(tempdir.path().join("lemontodo.db"))?;
        let original = wrap_vault_key(
            &VaultKey::generate(),
            "first password",
            KdfParams::generate_interactive(),
        )?;
        store.save_encrypted_vault_key(&original)?;
        let replacement = wrap_vault_key(
            &VaultKey::generate(),
            "second password",
            KdfParams::generate_interactive(),
        )?;

        let error = import_remote_vault_metadata(&store, &replacement, false, "second password")
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("local vault metadata already exists")
        );
        assert_eq!(store.encrypted_vault_key()?, Some(original));

        import_remote_vault_metadata(&store, &replacement, true, "second password")?;
        assert_eq!(store.encrypted_vault_key()?, Some(replacement));
        Ok(())
    }
}

fn run_tui(store: TodoStore) -> Result<()> {
    let tui_vault_key = unlock_tui_auto_sync(&store)?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, App::new(store)?, tui_vault_key);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

struct AutoSync {
    vault_key: Option<VaultKey>,
    last_attempt: Option<Instant>,
    dirty_since: Option<Instant>,
}

impl AutoSync {
    fn new(vault_key: Option<VaultKey>) -> Self {
        Self {
            vault_key,
            last_attempt: None,
            dirty_since: None,
        }
    }

    fn enabled(&self) -> bool {
        self.vault_key.is_some()
    }

    fn mark_dirty(&mut self) {
        if self.enabled() && self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn should_sync(&self, now: Instant, mode: Mode) -> bool {
        if !self.enabled() || mode != Mode::Browse {
            return false;
        }
        if let Some(dirty_since) = self.dirty_since
            && now.duration_since(dirty_since) >= TUI_AUTO_SYNC_DEBOUNCE
        {
            return true;
        }
        self.last_attempt
            .map(|last| now.duration_since(last) >= TUI_AUTO_SYNC_INTERVAL)
            .unwrap_or(true)
    }

    fn vault_key(&self) -> Option<&VaultKey> {
        self.vault_key.as_ref()
    }

    fn finish_attempt(&mut self, now: Instant, success: bool) {
        self.last_attempt = Some(now);
        if success {
            self.dirty_since = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyOutcome {
    quit: bool,
    changed: bool,
    force_sync: bool,
}

impl KeyOutcome {
    fn none() -> Self {
        Self {
            quit: false,
            changed: false,
            force_sync: false,
        }
    }

    fn quit() -> Self {
        Self {
            quit: true,
            changed: false,
            force_sync: false,
        }
    }

    fn changed() -> Self {
        Self {
            quit: false,
            changed: true,
            force_sync: false,
        }
    }

    fn force_sync() -> Self {
        Self {
            quit: false,
            changed: false,
            force_sync: true,
        }
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
    tui_vault_key: Option<VaultKey>,
) -> Result<()> {
    let mut auto_sync = AutoSync::new(tui_vault_key);
    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let outcome = handle_key(key, &mut app)?;
            if outcome.changed {
                auto_sync.mark_dirty();
            }
            if outcome.force_sync {
                run_tui_sync(&mut app, &mut auto_sync, "sync")?;
            }
            if outcome.quit {
                run_tui_sync(&mut app, &mut auto_sync, "final sync")?;
                return Ok(());
            }
        }

        if auto_sync.should_sync(Instant::now(), app.mode()) {
            run_tui_sync(&mut app, &mut auto_sync, "auto-sync")?;
        }
    }
}

fn run_tui_sync(app: &mut App, auto_sync: &mut AutoSync, label: &str) -> Result<()> {
    let Some(vault_key) = auto_sync.vault_key().cloned() else {
        app.set_message("sync: locked");
        return Ok(());
    };
    app.set_message(format!("{label}: syncing..."));
    let now = Instant::now();
    let server_url = configured_server_url(app.store(), None)?;
    match sync_now_with_vault_key(
        app.store(),
        &vault_key,
        &server_url,
        TUI_AUTO_SYNC_LIMIT,
        true,
    ) {
        Ok(summary) => {
            app.refresh()?;
            app.set_message(sync_summary_message(label, &summary));
            auto_sync.finish_attempt(now, true);
        }
        Err(error) => {
            app.set_message(format!("{label}: failed: {error}"));
            auto_sync.finish_attempt(now, false);
        }
    }
    Ok(())
}

fn sync_summary_message(label: &str, summary: &SyncNowSummary) -> String {
    format!(
        "{label}: ok pulled {} saved {} pushed {} conflicts {}",
        summary.pulled.operations.len(),
        summary.saved,
        summary.pushed.accepted,
        summary
            .applied
            .as_ref()
            .map(|applied| applied.conflicts)
            .unwrap_or(0)
    )
}

fn handle_key(key: KeyEvent, app: &mut App) -> Result<KeyOutcome> {
    match app.mode() {
        Mode::Browse => match key.code {
            KeyCode::Esc if app.help_visible() => app.hide_help(),
            KeyCode::Esc if app.sync_status_visible() => app.hide_sync_status(),
            KeyCode::Char('?') => app.toggle_help(),
            KeyCode::Char('q') => return Ok(KeyOutcome::quit()),
            KeyCode::Char('j') | KeyCode::Down => app.move_down(),
            KeyCode::Char('k') | KeyCode::Up => app.move_up(),
            KeyCode::Char('[') => app.previous_project()?,
            KeyCode::Char(']') => app.next_project()?,
            KeyCode::Char('v') => app.toggle_view_mode(),
            KeyCode::Char(' ') => {
                app.toggle_selected()?;
                return Ok(KeyOutcome::changed());
            }
            KeyCode::Char('a') => app.start_add(),
            KeyCode::Char('e') => app.start_edit_title(),
            KeyCode::Char('n') => app.start_edit_note(),
            KeyCode::Char('d') => app.start_edit_due(),
            KeyCode::Char('t') => app.start_edit_tags(),
            KeyCode::Char('m') => app.start_move_project(),
            KeyCode::Char('/') => app.start_search(),
            KeyCode::Char('c') => app.clear_search()?,
            KeyCode::Char('x') => {
                app.archive_selected()?;
                return Ok(KeyOutcome::changed());
            }
            KeyCode::Char('s') => app.toggle_sync_status()?,
            KeyCode::Char('S') => return Ok(KeyOutcome::force_sync()),
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
            KeyCode::Enter => {
                let mode = app.mode();
                app.submit_input()?;
                if mode != Mode::Search {
                    return Ok(KeyOutcome::changed());
                }
            }
            KeyCode::Backspace => app.pop_input(),
            KeyCode::Char(value) => app.push_input(value),
            _ => {}
        },
    }

    Ok(KeyOutcome::none())
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

fn print_remote_operations(store: &TodoStore, operations: &[RemoteOperation]) -> Result<()> {
    for remote in operations {
        let operation = &remote.operation;
        let local_revision =
            store.local_object_revision(operation.object_type, operation.object_id)?;
        let pending_local = store.pending_local_operation_count_for_object(operation.object_id)?;
        println!(
            "{} {} {} remote-rev:{} local-rev:{} local-pending:{} {} cursor:{} status:{}{}",
            short_id(&operation.id.to_string()),
            operation.object_type.as_str(),
            operation.operation_type.as_str(),
            operation.object_revision,
            local_revision
                .map(|revision| revision.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            pending_local,
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
    Ok(())
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

fn default_db_path_for_command(command: &Option<Command>) -> PathBuf {
    match command {
        Some(Command::Login {
            email: Some(email),
            server_url,
            ..
        })
        | Some(Command::Sync {
            command:
                Some(
                    SyncCommand::Login {
                        email: Some(email),
                        server_url,
                        ..
                    }
                    | SyncCommand::Connect {
                        email: Some(email),
                        server_url,
                        ..
                    },
                ),
            ..
        }) => {
            let server_url = server_url.as_deref().unwrap_or(DEFAULT_SERVER_URL);
            account_db_path(server_url, email)
        }
        _ => default_db_path(),
    }
}

fn account_db_path(server_url: &str, email: &str) -> PathBuf {
    dirs::data_dir()
        .context("failed to locate user data directory")
        .map(|path| {
            path.join("lemontodo")
                .join("accounts")
                .join(safe_path_component(server_url))
                .join(safe_path_component(email))
                .join("lemontodo.db")
        })
        .unwrap_or_else(|_| {
            PathBuf::from(".lemontodo")
                .join("accounts")
                .join(safe_path_component(server_url))
                .join(safe_path_component(email))
                .join("lemontodo.db")
        })
}

fn safe_path_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.trim().to_ascii_lowercase().bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => output.push(byte as char),
            _ => output.push_str(&format!("_{byte:02x}")),
        }
    }
    if output.is_empty() {
        "unknown".to_owned()
    } else {
        output
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
