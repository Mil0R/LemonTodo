use chrono::{DateTime, Utc};
use lemontodo_core::{Task, TaskStatus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode, ViewMode};

pub fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_tasks(frame, chunks[1], app);
    draw_footer(frame, chunks[2], app);

    if app.mode() != Mode::Browse {
        let input_width = UnicodeWidthStr::width(app.input()) as u16;
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

fn draw_header(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let help = if area.width < 100 {
        "[] project  v view  a add e edit m move / find x archive q quit"
    } else {
        "[] project  v view  a add  e edit  n note  d due  t tags  m move  / search  c clear  x archive  space toggle  j/k select  q quit"
    };

    let search = if app.search_query().is_empty() {
        String::new()
    } else {
        format!("  filter: {}", app.search_query())
    };

    let title = Paragraph::new(Line::from(vec![
        Span::styled("LemonTodo", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            format!("project: {}", app.current_project_name()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("view: {}", app.view_mode_name()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(help, Style::default().add_modifier(Modifier::DIM)),
        Span::styled(search, Style::default().add_modifier(Modifier::ITALIC)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn draw_tasks(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let items = if app.tasks().is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No tasks. Press 'a' to add one.",
            Style::default().add_modifier(Modifier::DIM),
        )))]
    } else {
        app.tasks()
            .iter()
            .map(|task| task_item(task, app.project_name_for_task(task), app.view_mode()))
            .collect()
    };

    let mut state = ListState::default();
    if !app.tasks().is_empty() {
        state.select(Some(app.selected()));
    }

    let list = List::new(items)
        .block(Block::default().title("Tasks").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let footer = match app.mode() {
        Mode::Browse => Paragraph::new(app.message())
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        Mode::Add => input_footer("New task", app.input()),
        Mode::EditTitle => input_footer("Edit title", app.input()),
        Mode::EditNote => input_footer("Edit note", app.input()),
        Mode::EditDue => input_footer("Edit due YYYY-MM-DD, empty clears", app.input()),
        Mode::EditTags => input_footer("Edit tags", app.input()),
        Mode::MoveProject => input_footer("Move to project", app.input()),
        Mode::Search => input_footer("Search", app.input()),
    };
    frame.render_widget(footer, area);
}

fn input_footer<'a>(title: &'a str, input: &'a str) -> Paragraph<'a> {
    Paragraph::new(input).block(Block::default().title(title).borders(Borders::ALL))
}

fn task_item<'a>(task: &'a Task, project_name: &'a str, view_mode: ViewMode) -> ListItem<'a> {
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

    let task_style = match task.status {
        TaskStatus::Open => Style::default(),
        TaskStatus::Done | TaskStatus::Archived => Style::default().add_modifier(Modifier::DIM),
    };
    let meta_style = Style::default().add_modifier(Modifier::DIM);

    let title = Line::from(vec![
        Span::styled(marker, task_style),
        Span::raw(" "),
        Span::styled(task.title.clone(), task_style),
        Span::styled(format!(" [{}]", project_name), meta_style),
        Span::styled(due, meta_style),
        Span::styled(tags, meta_style),
    ]);

    match view_mode {
        ViewMode::Compact => ListItem::new(title),
        ViewMode::Detail => ListItem::new(vec![
            title,
            Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("id: {}", task.id), meta_style),
                Span::raw("  "),
                Span::styled(format!("project: {project_name}"), meta_style),
                Span::raw("  "),
                Span::styled(
                    format!("created: {}", format_time(task.created_at)),
                    meta_style,
                ),
                Span::raw("  "),
                Span::styled(
                    format!("updated: {}", format_time(task.updated_at)),
                    meta_style,
                ),
            ]),
            Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!(
                        "due: {}",
                        task.due_date
                            .map(|date| date.to_string())
                            .unwrap_or_else(|| "-".to_owned())
                    ),
                    meta_style,
                ),
                Span::raw("  "),
                Span::styled(
                    format!(
                        "tags: {}",
                        if task.tags.is_empty() {
                            "-".to_owned()
                        } else {
                            task.tags.join(",")
                        }
                    ),
                    meta_style,
                ),
            ]),
        ]),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

fn format_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%MZ").to_string()
}
