import { useEffect, useReducer, useRef } from "react";
import {
  STORAGE_KEY,
  loadWorkspace,
  persistWorkspace,
  workspaceReducer,
  type DraftTask,
  type Project,
  type Task,
  type WorkspaceState,
} from "./workspace";

function App() {
  const [state, dispatch] = useReducer(workspaceReducer, undefined, loadWorkspace);
  const latestState = useRef<WorkspaceState>(state);
  latestState.current = state;

  useEffect(() => {
    if (state.sync.enabled && state.revision !== state.savedRevision) {
      const timeout = window.setTimeout(() => {
        syncState("auto");
      }, 550);
      return () => window.clearTimeout(timeout);
    }
    return undefined;
  }, [state.revision, state.savedRevision, state.sync.enabled]);

  useEffect(() => {
    const flushOnVisibility = () => {
      if (document.visibilityState === "hidden") {
        syncState("background");
      }
    };

    const flushOnBeforeUnload = () => {
      syncState("unload");
    };

    const hydrateFromStorage = (event: StorageEvent) => {
      if (event.key !== STORAGE_KEY || !event.newValue) {
        return;
      }
      try {
        const incoming = JSON.parse(event.newValue) as WorkspaceState;
        if (incoming.revision > latestState.current.revision) {
          dispatch({ type: "hydrate", snapshot: incoming });
        }
      } catch {
        dispatch({
          type: "set_sync_status",
          status: "offline",
          message: "Shared state changed, but reload failed",
        });
      }
    };

    window.addEventListener("beforeunload", flushOnBeforeUnload);
    document.addEventListener("visibilitychange", flushOnVisibility);
    window.addEventListener("storage", hydrateFromStorage);

    return () => {
      window.removeEventListener("beforeunload", flushOnBeforeUnload);
      document.removeEventListener("visibilitychange", flushOnVisibility);
      window.removeEventListener("storage", hydrateFromStorage);
    };
  }, []);

  function syncState(reason: "auto" | "manual" | "background" | "unload") {
    const current = latestState.current;
    if (reason === "auto" && !current.sync.enabled) {
      return;
    }
    if (current.revision === current.savedRevision && reason !== "manual") {
      return;
    }

    dispatch({
      type: "set_sync_status",
      status: "syncing",
      message: reason === "manual" ? "Syncing workspace" : "Syncing in the background",
    });

    try {
      const timestamp = new Date().toISOString();
      persistWorkspace(createSavedSnapshot(current, timestamp));
      dispatch({ type: "mark_saved", timestamp });
      dispatch({
        type: "set_sync_status",
        status: "saved",
        message: reason === "manual" ? "Workspace saved" : "Workspace synced",
      });
    } catch {
      dispatch({
        type: "set_sync_status",
        status: "offline",
        message: "Local sync failed",
      });
    }
  }

  const projects = state.projects;
  const selectedProject =
    projects.find((project) => project.id === state.currentProjectId) ?? projects[0] ?? null;
  const visibleTasks = filterTasks(state.tasks, state.search, selectedProject?.id ?? null);
  const selectedTask =
    state.tasks.find((task) => task.id === state.selectedTaskId) ?? visibleTasks[0] ?? null;

  const projectCounts = projects.map((project) => {
    const tasks = state.tasks.filter((task) => task.projectId === project.id);
    const open = tasks.filter((task) => task.status === "open").length;
    return { project, total: tasks.length, open };
  });

  function handleSelectTask(task: Task) {
    dispatch({ type: "select_task", taskId: task.id });
  }

  function handleSaveTask() {
    dispatch({ type: "save_task" });
  }

  function handleNewTask() {
    dispatch({ type: "new_task" });
  }

  function handleProjectNew() {
    dispatch({ type: "new_project" });
  }

  function handleProjectRename() {
    dispatch({ type: "rename_project" });
  }

  function handleProjectRemove() {
    dispatch({ type: "remove_project" });
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-block">
          <div className="brand-kicker">LemonTodo</div>
          <div className="brand-title">Terminal Web</div>
        </div>

        <div className="status-strip" aria-live="polite">
          <StatusChip tone={state.sync.status}>{state.sync.message}</StatusChip>
          <span className="mono-meta">
            rev {state.revision} / saved {state.savedRevision}
          </span>
          <label className="switch">
            <input
              type="checkbox"
              checked={state.sync.enabled}
              onChange={(event) =>
                dispatch({ type: "set_auto_sync", enabled: event.target.checked })
              }
            />
            <span>Auto sync</span>
          </label>
        </div>

        <div className="topbar-actions">
          <button className="button button-ghost" onClick={handleProjectNew}>
            + Project
          </button>
          <button className="button button-ghost" onClick={handleNewTask}>
            + Task
          </button>
          <button className="button button-primary" onClick={() => syncState("manual")}>
            ↻ Sync
          </button>
        </div>
      </header>

      <main className="workspace">
        <section className="panel rail-panel">
          <div className="panel-head">
            <div>
              <div className="panel-label">Projects</div>
              <h2>Groups</h2>
            </div>
            <span className="panel-count">{projects.length}</span>
          </div>

          <div className="project-editor">
            <input
              className="terminal-input"
              value={state.projectDraft}
              onChange={(event) =>
                dispatch({ type: "set_project_draft", value: event.target.value })
              }
              placeholder="Project name"
            />
            <div className="project-actions">
              <button className="button button-ghost" onClick={handleProjectRename}>
                Rename
              </button>
              <button className="button button-ghost" onClick={handleProjectRemove}>
                Remove
              </button>
            </div>
          </div>

          <div className="project-list" role="list" aria-label="Projects">
            {projectCounts.map(({ project, open, total }) => {
              const active = project.id === selectedProject?.id;
              return (
                <button
                  key={project.id}
                  className={`project-item ${active ? "is-active" : ""}`}
                  onClick={() => dispatch({ type: "select_project", projectId: project.id })}
                >
                  <span className="project-name">{project.name}</span>
                  <span className="project-meta">
                    <span>{open} open</span>
                    <span>{total} total</span>
                  </span>
                </button>
              );
            })}
          </div>
        </section>

        <section className="panel tasks-panel">
          <div className="panel-head">
            <div>
              <div className="panel-label">Tasks</div>
              <h2>{selectedProject ? selectedProject.name : "Inbox"}</h2>
            </div>
            <span className="panel-count">{visibleTasks.length}</span>
          </div>

          <div className="task-toolbar">
            <input
              className="terminal-input"
              value={state.search}
              onChange={(event) => dispatch({ type: "set_search", value: event.target.value })}
              placeholder="Search title, note, tag"
            />
            <div className="task-toolbar-actions">
              <button className="button button-ghost" onClick={handleNewTask}>
                New
              </button>
              <button className="button button-ghost" onClick={() => dispatch({ type: "select_task", taskId: null })}>
                Clear
              </button>
            </div>
          </div>

          <div className="task-list" role="list" aria-label="Tasks">
            {visibleTasks.length ? (
              visibleTasks.map((task) => {
                const active = task.id === selectedTask?.id;
                return (
                  <button
                    key={task.id}
                    className={`task-item ${active ? "is-active" : ""} status-${task.status}`}
                    onClick={() => handleSelectTask(task)}
                  >
                    <div className="task-line">
                      <span className="task-marker">{taskMarker(task.status)}</span>
                      <span className="task-title">{task.title}</span>
                    </div>
                    <div className="task-meta">
                      <span>{task.dueDate || "no due date"}</span>
                      <span>{task.tags.length ? task.tags.map((tag) => `#${tag}`).join(" ") : "untagged"}</span>
                    </div>
                  </button>
                );
              })
            ) : (
              <div className="empty-state">
                <div className="empty-title">No tasks match the current filter.</div>
                <div className="empty-copy">Create a new task or clear the search field.</div>
              </div>
            )}
          </div>
        </section>

        <section className="panel inspector-panel">
          <div className="panel-head">
            <div>
              <div className="panel-label">Inspector</div>
              <h2>{selectedTask ? selectedTask.title : "Draft"}</h2>
            </div>
            <span className="panel-count">
              {selectedTask ? taskMarker(selectedTask.status) : "draft"}
            </span>
          </div>

          {selectedTask ? (
            <TaskInspector
              task={selectedTask}
              projects={projects}
              draft={state.draft}
              onFieldChange={(field, value) =>
                dispatch({ type: "set_draft_field", field, value })
              }
              onSave={handleSaveTask}
              onToggle={() => dispatch({ type: "toggle_task", taskId: selectedTask.id })}
              onArchive={() => dispatch({ type: "archive_task", taskId: selectedTask.id })}
              onDelete={() => dispatch({ type: "delete_task", taskId: selectedTask.id })}
              onMove={(projectId) =>
                dispatch({ type: "move_task", taskId: selectedTask.id, projectId })
              }
            />
          ) : (
            <TaskInspector
              task={null}
              projects={projects}
              draft={state.draft}
              onFieldChange={(field, value) =>
                dispatch({ type: "set_draft_field", field, value })
              }
              onSave={handleSaveTask}
              onToggle={() => undefined}
              onArchive={() => undefined}
              onDelete={() => undefined}
              onMove={(projectId) =>
                dispatch({ type: "set_draft_field", field: "projectId", value: projectId })
              }
            />
          )}
        </section>
      </main>

      <footer className="statusbar">
        <div className="statusbar-left">
          <span>{selectedProject?.name ?? "Inbox"}</span>
          <span>{selectedTask ? selectedTask.title : "No task selected"}</span>
        </div>
        <div className="statusbar-right">
          <span>{state.sync.lastSyncedAt ? formatClock(state.sync.lastSyncedAt) : "not synced"}</span>
          <span>{state.sync.enabled ? "autosync active" : "autosync paused"}</span>
        </div>
      </footer>
    </div>
  );
}

function TaskInspector(props: {
  task: Task | null;
  projects: Project[];
  draft: DraftTask;
  onFieldChange: (field: keyof DraftTask, value: string) => void;
  onSave: () => void;
  onToggle: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onMove: (projectId: string) => void;
}) {
  return (
    <div className="inspector">
      <label className="field">
        <span>Title</span>
        <input
          className="terminal-input"
          value={props.draft.title}
          onChange={(event) => props.onFieldChange("title", event.target.value)}
          placeholder="Task title"
        />
      </label>

      <label className="field">
        <span>Note</span>
        <textarea
          className="terminal-textarea"
          value={props.draft.note}
          onChange={(event) => props.onFieldChange("note", event.target.value)}
          placeholder="Markdown note"
        />
      </label>

      <div className="field-grid">
        <label className="field">
          <span>Tags</span>
          <input
            className="terminal-input"
            value={props.draft.tags}
            onChange={(event) => props.onFieldChange("tags", event.target.value)}
            placeholder="sync mvp"
          />
        </label>
        <label className="field">
          <span>Due</span>
          <input
            className="terminal-input"
            value={props.draft.dueDate}
            onChange={(event) => props.onFieldChange("dueDate", event.target.value)}
            placeholder="YYYY-MM-DD"
          />
        </label>
      </div>

      <div className="field-grid">
        <label className="field">
          <span>Status</span>
          <select
            className="terminal-input"
            value={props.draft.status}
            onChange={(event) => props.onFieldChange("status", event.target.value)}
          >
            <option value="open">Open</option>
            <option value="done">Done</option>
            <option value="archived">Archived</option>
          </select>
        </label>

        <label className="field">
          <span>Project</span>
          <select
            className="terminal-input"
            value={props.draft.projectId}
            onChange={(event) => props.onMove(event.target.value)}
          >
            {props.projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="inspector-actions">
        <button className="button button-primary" onClick={props.onSave}>
          Save
        </button>
        <button className="button button-ghost" onClick={props.onToggle} disabled={!props.task}>
          Toggle
        </button>
        <button className="button button-ghost" onClick={props.onArchive} disabled={!props.task}>
          Archive
        </button>
        <button className="button button-danger" onClick={props.onDelete} disabled={!props.task}>
          Delete
        </button>
      </div>
    </div>
  );
}

function StatusChip({
  tone,
  children,
}: {
  tone: WorkspaceState["sync"]["status"];
  children: string;
}) {
  return <span className={`status-chip tone-${tone}`}>{children}</span>;
}

function taskMarker(status: Task["status"]) {
  switch (status) {
    case "done":
      return "[x]";
    case "archived":
      return "[-]";
    default:
      return "[ ]";
  }
}

function filterTasks(tasks: Task[], query: string, projectId: string | null) {
  const needle = query.trim().toLowerCase();
  return [...tasks]
    .filter((task) => !projectId || task.projectId === projectId)
    .filter((task) => {
      if (!needle) {
        return true;
      }
      return (
        task.title.toLowerCase().includes(needle) ||
        task.note.toLowerCase().includes(needle) ||
        task.tags.some((tag) => tag.toLowerCase().includes(needle))
      );
    })
    .sort((left, right) => {
      if (left.status !== right.status) {
        const order = { open: 0, done: 1, archived: 2 } as const;
        return order[left.status] - order[right.status];
      }
      return right.updatedAt.localeCompare(left.updatedAt);
    });
}

function createSavedSnapshot(state: WorkspaceState, timestamp: string): WorkspaceState {
  return {
    ...state,
    savedRevision: state.revision,
    sync: {
      ...state.sync,
      status: "saved",
      message: "Synced",
      lastSyncedAt: timestamp,
    },
  };
}

function formatClock(value: string) {
  return new Intl.DateTimeFormat("en", {
    hour: "2-digit",
    minute: "2-digit",
    month: "short",
    day: "2-digit",
  }).format(new Date(value));
}

export default App;
