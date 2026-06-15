use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use lemontodo_core::{NewTask, TaskStatus};
use lemontodo_storage::TodoStore;

#[derive(Debug, Parser)]
#[command(name = "ltd")]
#[command(about = "LemonTodo developer-first Todo CLI/TUI")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize the local LemonTodo database.
    Init,
    /// Add a task to Inbox.
    Add {
        title: String,
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
    },
    /// Mark a task done by id prefix.
    Done { id: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(default_db_path);
    let store = TodoStore::open(&db_path)?;

    match cli.command {
        Command::Init => {
            println!("Initialized LemonTodo database at {}", db_path.display());
        }
        Command::Add {
            title,
            note,
            tag,
            due,
        } => {
            let mut task = NewTask::new(title);
            task.note_markdown = note.unwrap_or_default();
            task.tags = tag;
            task.due_date = due;

            let task = store.add_task(task)?;
            println!("Added {} {}", short_id(&task.id.to_string()), task.title);
        }
        Command::List { all } => {
            let tasks = store.list_tasks(all)?;
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
        }
        Command::Done { id } => {
            let task = store.mark_done(&id)?;
            println!(
                "Completed {} {}",
                short_id(&task.id.to_string()),
                task.title
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
