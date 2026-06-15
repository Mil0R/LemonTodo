use lemontodo_core::{Task, TaskStatus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::{App, Mode};

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
        let input_width = app.input().chars().count() as u16;
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
    let search = if app.search_query().is_empty() {
        Span::raw("")
    } else {
        Span::styled(
            format!("  filter: {}", app.search_query()),
            Style::default().fg(Color::Green),
        )
    };

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "LemonTodo",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "a add  e edit  / search  c clear  x archive  space toggle  j/k move  q quit",
            Style::default().fg(Color::DarkGray),
        ),
        search,
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn draw_tasks(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let items = if app.tasks().is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No tasks. Press 'a' to add one.",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.tasks().iter().map(task_item).collect()
    };

    let mut state = ListState::default();
    if !app.tasks().is_empty() {
        state.select(Some(app.selected()));
    }

    let list = List::new(items)
        .block(Block::default().title("Tasks").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
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
        Mode::Edit => input_footer("Edit title", app.input()),
        Mode::Search => input_footer("Search", app.input()),
    };
    frame.render_widget(footer, area);
}

fn input_footer<'a>(title: &'a str, input: &'a str) -> Paragraph<'a> {
    Paragraph::new(input)
        .block(Block::default().title(title).borders(Borders::ALL))
        .style(Style::default().fg(Color::Yellow))
}

fn task_item(task: &Task) -> ListItem<'_> {
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

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
