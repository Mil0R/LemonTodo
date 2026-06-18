export type TaskStatus = "open" | "done" | "archived";

export interface Project {
  id: string;
  name: string;
}

export interface Task {
  id: string;
  projectId: string;
  title: string;
  note: string;
  tags: string[];
  dueDate: string;
  status: TaskStatus;
  createdAt: string;
  updatedAt: string;
}

export interface DraftTask {
  title: string;
  note: string;
  tags: string;
  dueDate: string;
  projectId: string;
  status: TaskStatus;
}

export interface SyncState {
  enabled: boolean;
  status: "idle" | "dirty" | "syncing" | "saved" | "offline" | "locked";
  message: string;
  lastSyncedAt: string | null;
}

export interface WorkspaceState {
  revision: number;
  savedRevision: number;
  currentProjectId: string;
  selectedTaskId: string | null;
  search: string;
  projectDraft: string;
  draft: DraftTask;
  projects: Project[];
  tasks: Task[];
  sync: SyncState;
}

type WorkspaceSnapshot = Omit<WorkspaceState, "sync"> & {
  sync: SyncState;
};

interface NormalizedDraft {
  title: string;
  note: string;
  tags: string[];
  dueDate: string;
  projectId: string;
  status: TaskStatus;
}

type WorkspaceAction =
  | { type: "select_project"; projectId: string }
  | { type: "select_task"; taskId: string | null }
  | { type: "set_search"; value: string }
  | { type: "set_project_draft"; value: string }
  | { type: "set_auto_sync"; enabled: boolean }
  | { type: "set_sync_status"; status: SyncState["status"]; message: string }
  | { type: "set_sync_message"; message: string }
  | { type: "mark_saved"; timestamp: string }
  | { type: "hydrate"; snapshot: WorkspaceSnapshot }
  | { type: "new_project" }
  | { type: "rename_project" }
  | { type: "remove_project" }
  | { type: "set_draft_field"; field: keyof DraftTask; value: string }
  | { type: "new_task" }
  | { type: "save_task" }
  | { type: "toggle_task"; taskId: string }
  | { type: "archive_task"; taskId: string }
  | { type: "delete_task"; taskId: string }
  | { type: "move_task"; taskId: string; projectId: string }
  | { type: "load_task_into_draft"; taskId: string | null };

const STORAGE_KEY = "lemontodo.web.workspace.v1";

export function workspaceStorageKey(email?: string | null): string {
  const account = email?.trim().toLowerCase();
  return account ? `${STORAGE_KEY}:${account}` : STORAGE_KEY;
}

export function createSeedWorkspace(): WorkspaceState {
  const inbox = createProject("Inbox");
  const projectAlpha = createProject("Alpha");
  const projectBeta = createProject("Beta");
  const tasks = [
    createTask(inbox.id, "Review sync status", "Confirm that session sync is green.", ["sync", "mvp"], "2026-06-18", "open"),
    createTask(projectAlpha.id, "Refine project rail", "Keep the list compact on narrow screens.", ["ui"], "", "open"),
    createTask(projectAlpha.id, "Check mobile spacing", "Tune inspector and list density for iPad.", ["responsive"], "2026-06-21", "done"),
    createTask(projectBeta.id, "Task editor polish", "Keep actions reachable with one hand.", ["ux", "web"], "", "open"),
  ];

  return {
    revision: 1,
    savedRevision: 1,
    currentProjectId: inbox.id,
    selectedTaskId: tasks[0]?.id ?? null,
    search: "",
    projectDraft: inbox.name,
    draft: taskToDraft(tasks[0], inbox.id),
    projects: [inbox, projectAlpha, projectBeta],
    tasks,
    sync: {
      enabled: true,
      status: "saved",
      message: "Ready",
      lastSyncedAt: null,
    },
  };
}

export function loadWorkspace(storageKey = STORAGE_KEY): WorkspaceState {
  if (typeof window === "undefined") {
    return createSeedWorkspace();
  }

  const raw = window.localStorage.getItem(storageKey);
  if (!raw) {
    return createSeedWorkspace();
  }

  try {
    const snapshot = JSON.parse(raw) as WorkspaceSnapshot;
    return sanitizeSnapshot(snapshot);
  } catch {
    return createSeedWorkspace();
  }
}

export function persistWorkspace(state: WorkspaceState, storageKey = STORAGE_KEY): void {
  if (typeof window === "undefined") {
    return;
  }
  const snapshot: WorkspaceSnapshot = {
    ...state,
    sync: {
      ...state.sync,
    },
  };
  window.localStorage.setItem(storageKey, JSON.stringify(snapshot));
}

export function workspaceReducer(
  state: WorkspaceState,
  action: WorkspaceAction,
): WorkspaceState {
  switch (action.type) {
    case "select_project": {
      const project = state.projects.find((entry) => entry.id === action.projectId);
      if (!project) {
        return state;
      }
      const task = state.tasks.find((entry) => entry.projectId === project.id) ?? null;
      return uiUpdate(state, {
        currentProjectId: project.id,
        selectedTaskId: task?.id ?? null,
        projectDraft: project.name,
        draft: task ? taskToDraft(task, project.id) : blankDraft(project.id),
      });
    }
    case "select_task": {
      const task = action.taskId
        ? state.tasks.find((entry) => entry.id === action.taskId) ?? null
        : null;
      if (!task) {
        return uiUpdate(state, {
          selectedTaskId: null,
          draft: blankDraft(state.currentProjectId),
        });
      }
      return uiUpdate(state, {
        selectedTaskId: task.id,
        currentProjectId: task.projectId,
        projectDraft:
          state.projects.find((project) => project.id === task.projectId)?.name ??
          state.projectDraft,
        draft: taskToDraft(task, task.projectId),
      });
    }
    case "set_search":
      return uiUpdate(state, { search: action.value });
    case "set_project_draft":
      return uiUpdate(state, { projectDraft: action.value });
    case "set_auto_sync":
      return uiUpdate(state, {
        sync: { ...state.sync, enabled: action.enabled },
      });
    case "set_sync_status":
      return uiUpdate(state, {
        sync: {
          ...state.sync,
          status: action.status,
          message: action.message,
        },
      });
    case "set_sync_message":
      return uiUpdate(state, {
        sync: {
          ...state.sync,
          message: action.message,
        },
      });
    case "mark_saved":
      return uiUpdate(state, {
        savedRevision: state.revision,
        sync: {
          ...state.sync,
          status: "saved",
          message: "Synced",
          lastSyncedAt: action.timestamp,
        },
      });
    case "hydrate":
      return sanitizeSnapshot(action.snapshot);
    case "new_project": {
      const name = actionProjectName(state.projectDraft, state.projects);
      if (!name) {
        return uiUpdate(state, {
          sync: { ...state.sync, message: "Project name cannot be empty" },
        });
      }
      const project = createProject(name);
      return touch(state, {
        projects: [...state.projects, project],
        currentProjectId: project.id,
        selectedTaskId: null,
        projectDraft: project.name,
        draft: blankDraft(project.id),
      });
    }
    case "rename_project": {
      const project = state.projects.find((entry) => entry.id === state.currentProjectId);
      if (!project) {
        return state;
      }
      const name = actionProjectName(state.projectDraft, state.projects, project.id);
      if (!name) {
        return uiUpdate(state, {
          sync: { ...state.sync, message: "Project name cannot be empty" },
        });
      }
      return touch(state, {
        projects: state.projects.map((entry) =>
          entry.id === project.id ? { ...entry, name } : entry,
        ),
        projectDraft: name,
      });
    }
    case "remove_project": {
      const removable = state.projects.find((entry) => entry.id === state.currentProjectId);
      if (!removable || state.projects.length === 1) {
        return state;
      }
      const fallback = state.projects.find((entry) => entry.id !== removable.id) ?? null;
      if (!fallback) {
        return state;
      }
      const nextTasks = state.tasks.map((task) =>
        task.projectId === removable.id ? { ...task, projectId: fallback.id } : task,
      );
      return touch(state, {
        projects: state.projects.filter((entry) => entry.id !== removable.id),
        tasks: nextTasks,
        currentProjectId: fallback.id,
        selectedTaskId: nextTasks.find((task) => task.projectId === fallback.id)?.id ?? null,
        projectDraft: fallback.name,
        draft: blankDraft(fallback.id),
      });
    }
    case "set_draft_field":
      return uiUpdate(state, {
        draft: {
          ...state.draft,
          [action.field]: action.value,
        },
      });
    case "new_task":
      return uiUpdate(state, {
        selectedTaskId: null,
        draft: blankDraft(state.currentProjectId),
      });
    case "save_task": {
      const normalized = normalizeDraft(state.draft);
      if (!normalized.title) {
        return uiUpdate(state, {
          sync: { ...state.sync, message: "Task title cannot be empty" },
        });
      }
      const now = new Date().toISOString();
      if (state.selectedTaskId) {
        const task = state.tasks.find((entry) => entry.id === state.selectedTaskId);
        if (!task) {
          return state;
        }
        const nextTask = {
          ...task,
          projectId: normalized.projectId,
          title: normalized.title,
          note: normalized.note,
          tags: normalized.tags,
          dueDate: normalized.dueDate,
          status: normalized.status,
          updatedAt: now,
        };
        return touch(state, {
          tasks: state.tasks.map((entry) => (entry.id === task.id ? nextTask : entry)),
          selectedTaskId: nextTask.id,
          currentProjectId: nextTask.projectId,
          projectDraft:
            state.projects.find((project) => project.id === nextTask.projectId)?.name ??
            state.projectDraft,
          draft: taskToDraft(nextTask, nextTask.projectId),
        });
      }
      const task = createTask(
        normalized.projectId,
        normalized.title,
        normalized.note,
        normalized.tags,
        normalized.dueDate,
        normalized.status,
      );
      return touch(state, {
        tasks: [task, ...state.tasks],
        selectedTaskId: task.id,
        currentProjectId: task.projectId,
        draft: taskToDraft(task, task.projectId),
      });
    }
    case "toggle_task": {
      const task = state.tasks.find((entry) => entry.id === action.taskId);
      if (!task) {
        return state;
      }
      const nextStatus: TaskStatus = task.status === "done" ? "open" : "done";
      return touch(state, {
        tasks: state.tasks.map((entry) =>
          entry.id === task.id
            ? { ...entry, status: nextStatus, updatedAt: new Date().toISOString() }
            : entry,
        ),
      });
    }
    case "archive_task": {
      const task = state.tasks.find((entry) => entry.id === action.taskId);
      if (!task) {
        return state;
      }
      return touch(state, {
        tasks: state.tasks.map((entry) =>
          entry.id === task.id
            ? { ...entry, status: "archived", updatedAt: new Date().toISOString() }
            : entry,
        ),
      });
    }
    case "delete_task":
      return touch(state, {
        tasks: state.tasks.filter((entry) => entry.id !== action.taskId),
        selectedTaskId:
          state.selectedTaskId === action.taskId ? null : state.selectedTaskId,
      });
    case "move_task": {
      const task = state.tasks.find((entry) => entry.id === action.taskId);
      if (!task) {
        return state;
      }
      return touch(state, {
        tasks: state.tasks.map((entry) =>
          entry.id === task.id
            ? { ...entry, projectId: action.projectId, updatedAt: new Date().toISOString() }
            : entry,
        ),
      });
    }
    case "load_task_into_draft": {
      if (!action.taskId) {
        return uiUpdate(state, {
          selectedTaskId: null,
          draft: blankDraft(state.currentProjectId),
        });
      }
      const task = state.tasks.find((entry) => entry.id === action.taskId);
      if (!task) {
        return state;
      }
      return uiUpdate(state, {
        selectedTaskId: task.id,
        currentProjectId: task.projectId,
        projectDraft:
          state.projects.find((project) => project.id === task.projectId)?.name ??
          state.projectDraft,
        draft: taskToDraft(task, task.projectId),
      });
    }
    default:
      return state;
  }
}

function touch(
  state: WorkspaceState,
  patch: Partial<WorkspaceState>,
): WorkspaceState {
  const revision = patch.revision ?? state.revision + 1;
  const sync = patch.sync ?? {
    ...state.sync,
    status: state.sync.enabled ? "dirty" : "idle",
    message: state.sync.enabled ? "Pending sync" : "Autosync paused",
  };
  return {
    ...state,
    ...patch,
    revision,
    sync,
  };
}

function uiUpdate(
  state: WorkspaceState,
  patch: Partial<WorkspaceState>,
): WorkspaceState {
  return {
    ...state,
    ...patch,
  };
}

function actionProjectName(
  input: string,
  projects: Project[],
  ignoreProjectId?: string,
): string {
  const name = input.trim();
  if (!name) {
    return "";
  }
  const collision = projects.find(
    (project) =>
      project.name.toLowerCase() === name.toLowerCase() &&
      project.id !== ignoreProjectId,
  );
  return collision ? "" : name;
}

function normalizeDraft(draft: DraftTask): NormalizedDraft {
  return {
    title: draft.title.trim(),
    note: draft.note.trim(),
    tags: parseTags(draft.tags),
    dueDate: draft.dueDate.trim(),
    projectId: draft.projectId,
    status: draft.status,
  };
}

function parseTags(input: string): string[] {
  const seen = new Set<string>();
  return input
    .split(/[,\s]+/)
    .map((tag) => tag.trim().replace(/^#/, "").toLowerCase())
    .filter((tag) => {
      if (!tag || seen.has(tag)) {
        return false;
      }
      seen.add(tag);
      return true;
    });
}

function blankDraft(projectId: string): DraftTask {
  return {
    title: "",
    note: "",
    tags: "",
    dueDate: "",
    projectId,
    status: "open",
  };
}

function taskToDraft(task: Task, projectId: string): DraftTask {
  return {
    title: task.title,
    note: task.note,
    tags: task.tags.join(" "),
    dueDate: task.dueDate,
    projectId,
    status: task.status,
  };
}

function createProject(name: string): Project {
  return {
    id: crypto.randomUUID(),
    name,
  };
}

function createTask(
  projectId: string,
  title: string,
  note: string,
  tags: string[],
  dueDate: string,
  status: TaskStatus,
): Task {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    projectId,
    title,
    note,
    tags,
    dueDate,
    status,
    createdAt: now,
    updatedAt: now,
  };
}

function sanitizeSnapshot(snapshot: WorkspaceSnapshot): WorkspaceState {
  if (
    !Array.isArray(snapshot.projects) ||
    !snapshot.projects.length ||
    !Array.isArray(snapshot.tasks) ||
    !snapshot.draft ||
    typeof snapshot.currentProjectId !== "string"
  ) {
    return createSeedWorkspace();
  }
  const projectExists = snapshot.projects.some(
    (project) => project.id === snapshot.currentProjectId,
  );
  const currentProjectId = projectExists
    ? snapshot.currentProjectId
    : snapshot.projects[0]?.id ?? crypto.randomUUID();
  const selectedTask = snapshot.tasks.find((task) => task.id === snapshot.selectedTaskId) ?? null;
  return {
    ...snapshot,
    currentProjectId,
    selectedTaskId: selectedTask?.id ?? null,
    draft: selectedTask
      ? taskToDraft(selectedTask, selectedTask.projectId)
      : blankDraft(currentProjectId),
    sync: {
      ...snapshot.sync,
      status: snapshot.sync.enabled ? snapshot.sync.status : "locked",
      message: snapshot.sync.message || "Ready",
    },
  };
}

export type { WorkspaceAction };
export { STORAGE_KEY };
