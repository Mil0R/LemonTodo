use std::{io, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use lemontodo_core::{NewTask, Task, TaskStatus};
use lemontodo_storage::TodoStore;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
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

struct App {
    store: TodoStore,
    tasks: Vec<Task>,
    selected: usize,
    input: String,
    mode: Mode,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Add,
}

impl App {
    fn new(store: TodoStore) -> Result<Self> {
        let mut app = Self {
            store,
            tasks: Vec::new(),
            selected: 0,
            input: String::new(),
            mode: Mode::Browse,
            message: String::new(),
        };
        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        self.tasks = self.store.list_tasks(true)?;
        if self.tasks.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.tasks.len() {
            self.selected = self.tasks.len() - 1;
        }
        Ok(())
    }

    fn selected_task(&self) -> Option<&Task> {
        self.tasks.get(self.selected)
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.tasks.len() {
            self.selected += 1;
        }
    }

    fn toggle_selected(&mut self) -> Result<()> {
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

    fn submit_input(&mut self) -> Result<()> {
        let title = self.input.trim();
        if title.is_empty() {
            self.message = "Task title cannot be empty".to_owned();
        } else {
            let task = self.store.add_task(NewTask::new(title))?;
            self.message = format!("Added {}", task.title);
            self.input.clear();
            self.mode = Mode::Browse;
            self.refresh()?;
            self.selected = self
                .tasks
                .iter()
                .position(|existing| existing.id == task.id)
                .unwrap_or(self.selected);
        }
        Ok(())
    }
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
            note,
            tag,
            due,
        }) => {
            let mut task = NewTask::new(title);
            task.note_markdown = note.unwrap_or_default();
            task.tags = tag;
            task.due_date = due;

            let task = store.add_task(task)?;
            println!("Added {} {}", short_id(&task.id.to_string()), task.title);
        }
        Some(Command::List { all }) => {
            print_tasks(store.list_tasks(all)?)?;
        }
        Some(Command::Done { id }) => {
            let task = store.mark_done(&id)?;
            println!(
                "Completed {} {}",
                short_id(&task.id.to_string()),
                task.title
            );
        }
        None => run_tui(store)?,
    }

    Ok(())
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
    match app.mode {
        Mode::Browse => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => app.move_down(),
            KeyCode::Char('k') | KeyCode::Up => app.move_up(),
            KeyCode::Char(' ') => app.toggle_selected()?,
            KeyCode::Char('a') => {
                app.mode = Mode::Add;
                app.input.clear();
                app.message = "Add task".to_owned();
            }
            KeyCode::Char('r') => {
                app.refresh()?;
                app.message = "Refreshed".to_owned();
            }
            _ => {}
        },
        Mode::Add => match key.code {
            KeyCode::Esc => {
                app.mode = Mode::Browse;
                app.input.clear();
                app.message = "Cancelled".to_owned();
            }
            KeyCode::Enter => app.submit_input()?,
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(value) => app.input.push(value),
            _ => {}
        },
    }

    Ok(false)
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "LemonTodo",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "a add  space toggle  j/k move  r refresh  q quit",
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let items = if app.tasks.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No tasks. Press 'a' to add one.",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.tasks
            .iter()
            .map(|task| {
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

                let style = match task.status {
                    TaskStatus::Open => Style::default().fg(Color::White),
                    TaskStatus::Done => Style::default().fg(Color::DarkGray),
                    TaskStatus::Archived => Style::default().fg(Color::DarkGray),
                };

                ListItem::new(Line::from(vec![
                    Span::styled(marker, style),
                    Span::raw(" "),
                    Span::styled(
                        short_id(&task.id.to_string()).to_owned(),
                        style.fg(Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::styled(task.title.clone(), style),
                    Span::styled(due, Style::default().fg(Color::Magenta)),
                    Span::styled(tags, Style::default().fg(Color::Green)),
                ]))
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.tasks.is_empty() {
        state.select(Some(app.selected));
    }
    let list = List::new(items)
        .block(Block::default().title("Tasks").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut state);

    let footer = match app.mode {
        Mode::Browse => Paragraph::new(app.message.as_str())
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        Mode::Add => Paragraph::new(app.input.as_str())
            .block(Block::default().title("New task").borders(Borders::ALL))
            .style(Style::default().fg(Color::Yellow)),
    };
    frame.render_widget(footer, chunks[2]);

    if app.mode == Mode::Add {
        let input_width = app.input.chars().count() as u16;
        frame.set_cursor_position((chunks[2].x + input_width + 1, chunks[2].y + 1));
    }

    if area.width < 60 || area.height < 12 {
        let popup = centered_rect(80, 40, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new("Terminal is too small")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            popup,
        );
    }
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
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
