use chrono::{DateTime, Utc};
use lemontodo_core::{Task, TaskStatus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Mode, SyncStatus, ViewMode};

const READY_MESSAGE: &str = "Ready";
const COLOR_BG: Color = Color::Rgb(13, 17, 23);
const COLOR_PANEL: Color = Color::Rgb(22, 27, 34);
const COLOR_LINE: Color = Color::Rgb(48, 54, 61);
const COLOR_TEXT: Color = Color::Rgb(230, 237, 243);
const COLOR_MUTED: Color = Color::Rgb(139, 148, 158);
const COLOR_GREEN: Color = Color::Rgb(63, 185, 80);

fn text_style() -> Style {
    Style::default().fg(COLOR_TEXT)
}

fn muted_style() -> Style {
    Style::default().fg(COLOR_MUTED)
}

fn accent_style() -> Style {
    Style::default().fg(COLOR_GREEN)
}

fn border_style() -> Style {
    Style::default().fg(COLOR_LINE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Viewport {
    Phone,
    Tablet,
    Desktop,
}

pub fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 32 || area.height < 10 {
        draw_too_small(frame, area);
        return;
    }

    let viewport = viewport_for(area.width);
    let header_lines = header_lines(app, viewport);
    let footer_lines = footer_lines(app, viewport);
    let header_height = block_height(&header_lines);
    let footer_height = block_height(&footer_lines);

    if area.height <= header_height + footer_height {
        draw_too_small(frame, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(3),
            Constraint::Length(footer_height),
        ])
        .split(area);

    draw_header(frame, chunks[0], &header_lines);
    draw_tasks(frame, chunks[1], app, viewport);
    draw_footer(frame, chunks[2], app, &footer_lines);

    if app.help_visible() {
        draw_help(frame, area, viewport);
    }

    if app.sync_status_visible() {
        draw_sync_status(frame, area, viewport, app.sync_status());
    }

    if app.mode() != Mode::Browse {
        let cursor_x = chunks[2].x + 1 + footer_input_offset(app.input());
        let cursor_y = chunks[2].y + footer_input_row(viewport);
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn draw_header(frame: &mut ratatui::Frame<'_>, area: Rect, lines: &[Line<'_>]) {
    let header = Paragraph::new(lines.to_vec())
        .style(text_style().bg(COLOR_PANEL))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(header, area);
}

fn draw_tasks(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App, viewport: Viewport) {
    let items = if app.tasks().is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No tasks. Press 'a' to add one.",
            Style::default().add_modifier(Modifier::DIM),
        )))]
    } else {
        app.tasks()
            .iter()
            .map(|task| {
                task_item(
                    task,
                    app.project_name_for_task(task),
                    app.view_mode(),
                    app.shows_all_projects(),
                    viewport,
                )
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.tasks().is_empty() {
        state.select(Some(app.selected()));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled("Tasks", accent_style().add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .highlight_style(
            Style::default()
                .bg(COLOR_GREEN)
                .fg(COLOR_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(match viewport {
            Viewport::Phone => ">",
            Viewport::Tablet | Viewport::Desktop => "> ",
        });
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App, lines: &[Line<'_>]) {
    let title = footer_title(app.mode(), app.message(), app.search_query(), app.input());
    let footer = Paragraph::new(lines.to_vec())
        .style(text_style().bg(COLOR_PANEL))
        .block(
            Block::default()
                .title(Span::styled(title, accent_style().add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, area);
}

fn draw_help(frame: &mut ratatui::Frame<'_>, area: Rect, viewport: Viewport) {
    let popup = popup_rect(area, viewport);
    frame.render_widget(Clear, popup);

    let lines = match viewport {
        Viewport::Phone => vec![
            Line::from(vec![Span::styled(
                "Move",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("j/k or arrows  select task"),
            Line::from("[ ]           switch project"),
            Line::from("v             view"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Task",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("space         toggle done/open"),
            Line::from("a e n d t     add/edit"),
            Line::from("m x           move/archive"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Search",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("/  search"),
            Line::from("c  clear"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "System",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("s sync panel"),
            Line::from("S sync now"),
            Line::from("? help"),
            Line::from("Esc close"),
            Line::from("q quit"),
        ],
        Viewport::Tablet => vec![
            Line::from(vec![Span::styled(
                "Navigation",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("j/k or Up/Down  select task    [ / ]  switch project    v  compact/detail"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Task",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(
                "space toggle   a add   e title   n note   d due   t tags   m move   x archive",
            ),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Search + System",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(
                "/ search   c clear   r refresh   s status   S sync now   ? help   Esc close   q quit",
            ),
        ],
        Viewport::Desktop => vec![
            Line::from(vec![Span::styled(
                "Navigation",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("  j/k, Up/Down   select task"),
            Line::from("  [, ]           switch project"),
            Line::from("  v              compact/detail view"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Task",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("  space          toggle done/open"),
            Line::from("  a              add task"),
            Line::from("  e              edit title"),
            Line::from("  n              edit note"),
            Line::from("  d              edit due date"),
            Line::from("  t              edit tags"),
            Line::from("  m              move to project"),
            Line::from("  x              archive"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Search",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("  /              search"),
            Line::from("  c              clear search"),
            Line::from(""),
            Line::from(vec![Span::styled(
                "System",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from("  r              refresh"),
            Line::from("  s              sync status"),
            Line::from("  S              sync now"),
            Line::from("  ?              toggle this help"),
            Line::from("  Esc            close help"),
            Line::from("  q              quit"),
        ],
    };

    let help = Paragraph::new(lines)
        .style(text_style().bg(COLOR_PANEL))
        .block(
            Block::default()
                .title(Span::styled("Help", accent_style().add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(help, popup);
}

fn draw_sync_status(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    viewport: Viewport,
    status: Option<&SyncStatus>,
) {
    let popup = popup_rect(area, viewport);
    frame.render_widget(Clear, popup);

    let lines = if let Some(status) = status {
        match viewport {
            Viewport::Phone => vec![
                sync_section("Local"),
                compact_status_line(
                    "server",
                    status.server.as_deref().unwrap_or("<not configured>"),
                ),
                compact_status_line(
                    "account",
                    status.account.as_deref().unwrap_or("<not configured>"),
                ),
                compact_status_line(
                    "token",
                    if status.access_token_configured {
                        "configured"
                    } else {
                        "<not configured>"
                    },
                ),
                compact_status_line(
                    "vault",
                    if status.vault_metadata_configured {
                        "configured"
                    } else {
                        "<not initialized>"
                    },
                ),
                Line::from(""),
                sync_section("Queue"),
                compact_status_line("local ops", status.pending_local_operations.to_string()),
                compact_status_line("remote ops", status.pending_remote_operations.to_string()),
                Line::from(""),
                Line::from(Span::styled(
                    "Esc closes this panel",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            ],
            Viewport::Tablet | Viewport::Desktop => vec![
                sync_section("Local"),
                status_line(
                    "server",
                    status.server.as_deref().unwrap_or("<not configured>"),
                ),
                status_line(
                    "account",
                    status.account.as_deref().unwrap_or("<not configured>"),
                ),
                status_line(
                    "access token",
                    if status.access_token_configured {
                        "configured"
                    } else {
                        "<not configured>"
                    },
                ),
                status_line(
                    "vault metadata",
                    if status.vault_metadata_configured {
                        "configured"
                    } else {
                        "<not initialized>"
                    },
                ),
                Line::from(""),
                sync_section("Queue"),
                status_line(
                    "pending local ops",
                    status.pending_local_operations.to_string(),
                ),
                status_line(
                    "pending remote ops",
                    status.pending_remote_operations.to_string(),
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Esc closes this panel",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            ],
        }
    } else {
        vec![Line::from("Sync status is not loaded")]
    };

    let status = Paragraph::new(lines)
        .style(text_style().bg(COLOR_PANEL))
        .block(
            Block::default()
                .title(Span::styled(
                    "Sync Status",
                    accent_style().add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(status, popup);
}

fn sync_section<'a>(title: &'a str) -> Line<'a> {
    Line::from(vec![Span::styled(
        title,
        accent_style().add_modifier(Modifier::BOLD),
    )])
}

fn compact_status_line<'a>(label: &'a str, value: impl Into<String>) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label}: "), muted_style()),
        Span::styled(value.into(), text_style()),
    ])
}

fn status_line<'a>(label: &'a str, value: impl Into<String>) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {label:<18}"), muted_style()),
        Span::styled(value.into(), text_style()),
    ])
}

fn footer_title<'a>(
    mode: Mode,
    _message: &'a str,
    search_query: &'a str,
    _input: &'a str,
) -> &'a str {
    match mode {
        Mode::Browse => {
            if !search_query.is_empty() {
                "Search"
            } else {
                "Status"
            }
        }
        Mode::Add => "New task",
        Mode::EditTitle => "Edit title",
        Mode::EditNote => "Edit note",
        Mode::EditDue => "Edit due YYYY-MM-DD, empty clears",
        Mode::EditTags => "Edit tags",
        Mode::MoveProject => "Move to project",
        Mode::Search => "Search",
    }
}

fn header_lines(app: &App, viewport: Viewport) -> Vec<Line<'static>> {
    let title = Span::styled("LemonTodo", accent_style().add_modifier(Modifier::BOLD));
    let project = app.current_project_name().to_owned();
    let view = app.view_mode_name().to_owned();
    let filter = app.search_query().to_owned();

    match viewport {
        Viewport::Phone => {
            let mut lines = vec![
                Line::from(vec![
                    title,
                    Span::raw("  "),
                    Span::styled(
                        truncate_to_width(&format!("project: {project}"), 18),
                        text_style().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(format!("view: {view}"), muted_style()),
                    Span::raw("  "),
                    Span::styled(
                        "a add  e edit  S sync  ? help",
                        muted_style(),
                    ),
                ]),
            ];
            if !filter.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("filter: ", muted_style()),
                    Span::styled(
                        truncate_to_width(&filter, 24),
                        accent_style().add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            lines
        }
        Viewport::Tablet => {
            let mut lines = vec![
                Line::from(vec![
                    title,
                    Span::raw("  "),
                    Span::styled(
                        truncate_to_width(&format!("project: {project}"), 30),
                        text_style().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("view: {view}"),
                        accent_style().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    "j/k move  [ ] project  space toggle  a add  e edit  S sync  ? help",
                    muted_style(),
                )]),
            ];
            if !filter.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("filter: ", muted_style()),
                    Span::styled(
                        truncate_to_width(&filter, 48),
                        accent_style().add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            lines
        }
        Viewport::Desktop => {
            let mut lines = vec![Line::from(vec![
                title,
                Span::raw("  "),
                Span::styled(
                    fixed_cell(&format!("project: {project}"), 28),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    fixed_cell(&format!("view: {view}"), 13),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  |  "),
                Span::styled(
                    "j/k select  space toggle  a add  e edit  S sync  s status  ? help  q quit",
                    muted_style(),
                ),
            ])];
            if !filter.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("filter: ", muted_style()),
                    Span::styled(
                        truncate_to_width(&filter, 72),
                        accent_style().add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            lines
        }
    }
}

fn footer_lines(app: &App, viewport: Viewport) -> Vec<Line<'static>> {
    match app.mode() {
        Mode::Browse => {
            let text = if app.message().is_empty() {
                READY_MESSAGE
            } else {
                app.message()
            };
            wrap_text_lines(
                text,
                match viewport {
                    Viewport::Phone => 30,
                    Viewport::Tablet => 60,
                    Viewport::Desktop => 96,
                },
            )
        }
        mode => input_lines(mode, app.input(), viewport),
    }
}

fn input_lines(mode: Mode, input: &str, viewport: Viewport) -> Vec<Line<'static>> {
    let hint = match mode {
        Mode::Add => "Enter saves. Esc cancels.",
        Mode::EditTitle => "Enter saves. Esc cancels.",
        Mode::EditNote => "Enter saves. Esc cancels.",
        Mode::EditDue => "YYYY-MM-DD. Empty clears.",
        Mode::EditTags => "Use spaces or commas.",
        Mode::MoveProject => "Type exact project name.",
        Mode::Search => "Enter applies search. Esc cancels.",
        Mode::Browse => "",
    };
    match viewport {
        Viewport::Phone => vec![
            Line::from(Span::styled(
                hint,
                Style::default().add_modifier(Modifier::DIM),
            )),
            Line::from(input.to_owned()),
        ],
        Viewport::Tablet | Viewport::Desktop => {
            let mut lines = vec![Line::from(input.to_owned())];
            if !hint.is_empty() {
                lines.push(Line::from(Span::styled(
                    hint,
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            lines
        }
    }
}

fn footer_input_row(viewport: Viewport) -> u16 {
    match viewport {
        Viewport::Phone => 2,
        Viewport::Tablet | Viewport::Desktop => 1,
    }
}

fn footer_input_offset(input: &str) -> u16 {
    let width = UnicodeWidthStr::width(input);
    width.min(u16::MAX as usize) as u16
}

fn task_item<'a>(
    task: &'a Task,
    project_name: &'a str,
    view_mode: ViewMode,
    show_project: bool,
    viewport: Viewport,
) -> ListItem<'a> {
    match viewport {
        Viewport::Phone => task_item_phone(task, project_name, view_mode, show_project),
        Viewport::Tablet => task_item_tablet(task, project_name, view_mode, show_project),
        Viewport::Desktop => task_item_desktop(task, project_name, view_mode, show_project),
    }
}

fn task_item_phone<'a>(
    task: &'a Task,
    project_name: &'a str,
    view_mode: ViewMode,
    show_project: bool,
) -> ListItem<'a> {
    let marker = task_marker(task.status);
    let title_style = task_style(task.status);
    let meta_style = muted_style();
    let mut lines = vec![Line::from(vec![
        Span::styled(marker, title_style),
        Span::raw(" "),
        Span::styled(task.title.clone(), title_style),
    ])];

    let mut meta = Vec::new();
    if show_project {
        meta.push(format!("@{project_name}"));
    }
    if let Some(due) = task.due_date {
        meta.push(format!("due {due}"));
    }
    if !task.tags.is_empty() {
        meta.push(
            task.tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    if !meta.is_empty() {
        lines.push(Line::from(Span::styled(meta.join("  "), meta_style)));
    }
    if view_mode == ViewMode::Detail {
        lines.push(Line::from(Span::styled(
            format!(
                "id {}  updated {}",
                short_uuid(&task.id.to_string()),
                format_short_time(task.updated_at)
            ),
            meta_style,
        )));
    }
    ListItem::new(lines)
}

fn task_item_tablet<'a>(
    task: &'a Task,
    project_name: &'a str,
    view_mode: ViewMode,
    show_project: bool,
) -> ListItem<'a> {
    let marker = task_marker(task.status);
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
    let task_style = task_style(task.status);
    let meta_style = muted_style();
    let project = if show_project {
        format!(" @{project_name}")
    } else {
        String::new()
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(marker, task_style),
        Span::raw(" "),
        Span::styled(task.title.clone(), task_style),
        Span::styled(project, meta_style),
        Span::styled(due, meta_style),
        Span::styled(tags, meta_style),
    ])];
    if view_mode == ViewMode::Detail {
        lines.push(Line::from(Span::styled(
            format!(
                "id {}  created {}  updated {}",
                short_uuid(&task.id.to_string()),
                format_short_time(task.created_at),
                format_short_time(task.updated_at)
            ),
            meta_style,
        )));
    }
    ListItem::new(lines)
}

fn task_item_desktop<'a>(
    task: &'a Task,
    project_name: &'a str,
    view_mode: ViewMode,
    show_project: bool,
) -> ListItem<'a> {
    let marker = task_marker(task.status);
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

    let task_style = task_style(task.status);
    let meta_style = muted_style();

    let project = if show_project {
        format!(" [{project_name}]")
    } else {
        String::new()
    };
    let title = Line::from(vec![
        Span::styled(marker, task_style),
        Span::raw(" "),
        Span::styled(task.title.clone(), task_style),
        Span::styled(project, meta_style),
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
                project_detail(project_name, show_project, meta_style),
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

fn task_marker(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Open => "[ ]",
        TaskStatus::Done => "[x]",
        TaskStatus::Archived => "[-]",
    }
}

fn task_style(status: TaskStatus) -> Style {
    match status {
        TaskStatus::Open => accent_style(),
        TaskStatus::Done | TaskStatus::Archived => muted_style(),
    }
}

fn project_detail<'a>(project_name: &'a str, show_project: bool, meta_style: Style) -> Span<'a> {
    if show_project {
        Span::styled(format!("  project: {project_name}  "), meta_style)
    } else {
        Span::raw("  ")
    }
}

fn popup_rect(area: Rect, viewport: Viewport) -> Rect {
    match viewport {
        Viewport::Phone => centered_rect(94, 88, area),
        Viewport::Tablet => centered_rect(88, 76, area),
        Viewport::Desktop => centered_rect(74, 76, area),
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

fn draw_too_small(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let popup = centered_rect(92, 56, area);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Terminal is too small"),
            Line::from("Use at least 32x10"),
        ])
        .alignment(Alignment::Center)
        .style(text_style().bg(COLOR_PANEL))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style()),
        ),
        popup,
    );
}

fn block_height(lines: &[Line<'_>]) -> u16 {
    (lines.len() as u16).saturating_add(2).max(3)
}

fn viewport_for(width: u16) -> Viewport {
    if width < 64 {
        Viewport::Phone
    } else if width < 108 {
        Viewport::Tablet
    } else {
        Viewport::Desktop
    }
}

fn wrap_text_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        let separator = if current.is_empty() { 0 } else { 1 };
        if current_width + separator + word_width > width && !current.is_empty() {
            lines.push(Line::from(std::mem::take(&mut current)));
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }

    if current.is_empty() {
        vec![Line::from(text.to_owned())]
    } else {
        lines.push(Line::from(current));
        lines
    }
}

fn fixed_cell(value: &str, width: usize) -> String {
    let fitted = truncate_to_width(value, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(fitted.as_str()));
    format!("{fitted}{}", " ".repeat(padding))
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }

    let ellipsis = "...";
    let ellipsis_width = UnicodeWidthStr::width(ellipsis);
    let target_width = width.saturating_sub(ellipsis_width);
    let mut output = String::new();
    let mut used_width = 0;

    for value in value.chars() {
        let char_width = UnicodeWidthChar::width(value).unwrap_or(0);
        if used_width + char_width > target_width {
            break;
        }
        output.push(value);
        used_width += char_width;
    }

    output.push_str(ellipsis);
    output
}

fn format_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%MZ").to_string()
}

fn format_short_time(value: DateTime<Utc>) -> String {
    value.format("%m-%d %H:%MZ").to_string()
}

fn short_uuid(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_viewport_breakpoints() {
        assert_eq!(viewport_for(40), Viewport::Phone);
        assert_eq!(viewport_for(80), Viewport::Tablet);
        assert_eq!(viewport_for(120), Viewport::Desktop);
    }

    #[test]
    fn truncates_with_ellipsis_for_unicode_width() {
        let truncated = truncate_to_width("project: 长名字长名字长名字", 12);
        assert!(UnicodeWidthStr::width(truncated.as_str()) <= 12);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn wraps_long_messages_into_multiple_lines() {
        let lines = wrap_text_lines("sync completed with remote updates waiting to apply", 18);
        assert!(lines.len() >= 2);
    }
}
