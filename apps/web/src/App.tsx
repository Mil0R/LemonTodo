import { useEffect, useReducer, useRef, useState } from "react";
import {
  cacheUnlockedVaultKey,
  clearAccountClientState,
  clearWebSession,
  clearCachedVaultKey,
  consumeRegisteredAccountReset,
  fetchAccount,
  loginWithMasterPassword,
  logoutFromServer,
  pullWorkspaceSnapshot,
  pushWorkspaceSnapshot,
  readCachedVaultKey,
  readAccessTokenFromLocation,
  readMasterPasswordHandoff,
  unlockVaultKey,
  type AccountStatus,
} from "./serverSync";
import {
  createBlankWorkspace,
  createSeedWorkspace,
  loadWorkspace,
  persistWorkspace,
  resetLegacyWebStorage,
  workspaceStorageKey,
  workspaceReducer,
  type DraftTask,
  type Project,
  type Task,
  type WorkspaceState,
} from "./workspace";

function App() {
  resetLegacyWebStorage();
  const [state, dispatch] = useReducer(workspaceReducer, undefined, loadWorkspace);
  const [accessToken, setAccessToken] = useState<string | null>(null);
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [vaultKeyHex, setVaultKeyHex] = useState<string | null>(null);
  const [isBootstrapping, setIsBootstrapping] = useState(true);
  const [isSubmittingLogin, setIsSubmittingLogin] = useState(false);
  const [authMessage, setAuthMessage] = useState<string | null>(null);
  const [projectRemovePending, setProjectRemovePending] = useState(false);
  const pullInFlight = useRef(false);
  const latestState = useRef<WorkspaceState>(state);
  const latestAccount = useRef<AccountStatus | null>(null);
  const latestAccessToken = useRef<string | null>(null);
  const latestVaultKeyHex = useRef<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const projectInputRef = useRef<HTMLInputElement | null>(null);
  latestState.current = state;
  latestAccount.current = account;
  latestAccessToken.current = accessToken;
  latestVaultKeyHex.current = vaultKeyHex;

  useEffect(() => {
    if (state.sync.enabled && state.revision !== state.savedRevision) {
      const timeout = window.setTimeout(() => {
        syncState("auto");
      }, 550);
      return () => window.clearTimeout(timeout);
    }
    return undefined;
  }, [state.revision, state.savedRevision, state.sync.enabled, account?.email, accessToken]);

  useEffect(() => {
    void bootstrapSession();
  }, []);

  useEffect(() => {
    setProjectRemovePending(false);
  }, [state.currentProjectId]);

  useEffect(() => {
    const flushOnVisibility = () => {
      if (document.visibilityState === "hidden") {
        syncState("background");
        return;
      }
      pullRemoteUpdates("resume");
    };

    const flushOnBeforeUnload = () => {
      syncState("unload");
    };

    const hydrateFromStorage = (event: StorageEvent) => {
      if (event.key !== workspaceStorageKey(latestAccount.current?.email) || !event.newValue) {
        return;
      }
      try {
        const incoming = JSON.parse(event.newValue) as WorkspaceState;
        if (workspaceContentChanged(latestState.current, incoming)) {
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

  useEffect(() => {
    if (!account || !accessToken || !vaultKeyHex) {
      return;
    }
    const interval = window.setInterval(() => {
      pullRemoteUpdates("poll");
    }, 5000);
    return () => window.clearInterval(interval);
  }, [account?.email, accessToken, vaultKeyHex]);

  useEffect(() => {
    const handleTaskShortcut = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      if (event.key === "Escape") {
        const searchFocused = document.activeElement === searchInputRef.current;
        if (searchFocused) {
          event.preventDefault();
          searchInputRef.current?.blur();
          return;
        }
        if (latestState.current.selectedTaskId) {
          event.preventDefault();
          dispatch({ type: "select_task", taskId: null });
        }
        return;
      }
      if (isTypingTarget(event.target)) {
        return;
      }
      if (event.key === "/") {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }
      if (event.key === "a" || event.key === "A") {
        event.preventDefault();
        dispatch({ type: "new_task" });
        return;
      }

      const currentTasks = filterTasks(
        latestState.current.tasks,
        latestState.current.search,
        latestState.current.currentProjectId,
      );
      if (!currentTasks.length) {
        return;
      }
      const selectedId = latestState.current.selectedTaskId;
      const selectedIndex = currentTasks.findIndex((task) => task.id === selectedId);
      const activeIndex = selectedIndex >= 0 ? selectedIndex : 0;
      if (event.key === "j" || event.key === "J") {
        event.preventDefault();
        const nextTask = currentTasks[Math.min(currentTasks.length - 1, activeIndex + 1)];
        dispatch({ type: "select_task", taskId: nextTask.id });
        return;
      }
      if (event.key === "k" || event.key === "K") {
        event.preventDefault();
        const nextTask = currentTasks[Math.max(0, activeIndex - 1)];
        dispatch({ type: "select_task", taskId: nextTask.id });
        return;
      }
      if (event.key === " ") {
        event.preventDefault();
        dispatch({ type: "toggle_task", taskId: currentTasks[activeIndex].id });
      }
    };

    window.addEventListener("keydown", handleTaskShortcut);
    return () => window.removeEventListener("keydown", handleTaskShortcut);
  }, []);

  async function syncState(reason: "auto" | "manual" | "background" | "unload") {
    const current = latestState.current;
    const currentAccount = latestAccount.current;
    const token = latestAccessToken.current;
    const currentVaultKey = latestVaultKeyHex.current;
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
      const storageKey = workspaceStorageKey(currentAccount?.email);
      const saved = createSavedSnapshot(current, timestamp);
      if (token && currentAccount && currentVaultKey) {
        if (current.revision !== current.savedRevision || reason === "manual") {
          await pushWorkspaceSnapshot(token, currentAccount.email, currentVaultKey, saved);
        }
        const remoteWorkspace = await pullWorkspaceSnapshot(
          token,
          currentAccount.email,
          currentVaultKey,
          saved,
        );
        if (remoteWorkspace && workspaceContentChanged(current, remoteWorkspace)) {
          const hydrated = createSavedSnapshot(remoteWorkspace, timestamp);
          persistWorkspace(hydrated, storageKey);
          dispatch({ type: "hydrate", snapshot: hydrated });
          dispatch({
            type: "set_sync_status",
            status: "saved",
            message: "Remote updates applied",
          });
          return;
        }
      }
      persistWorkspace(saved, storageKey);
      dispatch({ type: "mark_saved", timestamp });
      dispatch({
        type: "set_sync_status",
        status: "saved",
        message:
          token && currentAccount && currentVaultKey
            ? reason === "manual"
              ? "Server sync complete"
              : "Synced to server"
            : reason === "manual"
              ? "Workspace saved locally"
              : "Workspace saved locally",
      });
    } catch (error) {
      dispatch({
        type: "set_sync_status",
        status: "offline",
        message: error instanceof Error ? error.message : "Local sync failed",
      });
    }
  }

  async function pullRemoteUpdates(reason: "poll" | "resume") {
    const current = latestState.current;
    const currentAccount = latestAccount.current;
    const token = latestAccessToken.current;
    const currentVaultKey = latestVaultKeyHex.current;
    if (!current.sync.enabled || current.revision !== current.savedRevision) {
      return;
    }
    if (!token || !currentAccount || !currentVaultKey || pullInFlight.current) {
      return;
    }

    pullInFlight.current = true;
    try {
      const timestamp = new Date().toISOString();
      const storageKey = workspaceStorageKey(currentAccount.email);
      const remoteWorkspace = await pullWorkspaceSnapshot(
        token,
        currentAccount.email,
        currentVaultKey,
        current,
      );
      if (!remoteWorkspace || !workspaceContentChanged(current, remoteWorkspace)) {
        return;
      }
      const hydrated = createSavedSnapshot(remoteWorkspace, timestamp);
      persistWorkspace(hydrated, storageKey);
      dispatch({ type: "hydrate", snapshot: hydrated });
      dispatch({
        type: "set_sync_status",
        status: "saved",
        message: reason === "resume" ? "Remote updates loaded" : "Remote updates applied",
      });
    } catch (error) {
      dispatch({
        type: "set_sync_status",
        status: "offline",
        message: error instanceof Error ? error.message : "Remote sync failed",
      });
    } finally {
      pullInFlight.current = false;
    }
  }

  const projects = state.projects;
  const selectedProject =
    projects.find((project) => project.id === state.currentProjectId) ?? projects[0] ?? null;
  const isPremium = account?.plan === "premium";
  const visibleTasks = filterTasks(state.tasks, state.search, selectedProject?.id ?? null);
  const selectedTask =
    state.tasks.find((task) => task.id === state.selectedTaskId) ?? null;
  const activeTask = selectedTask ?? visibleTasks[0] ?? null;

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
    if (!isPremium) {
      dispatch({
        type: "set_sync_message",
        message: "Free accounts can only use Inbox",
      });
      return;
    }
    setProjectRemovePending(false);
    dispatch({ type: "set_project_draft", value: "" });
    window.requestAnimationFrame(() => {
      projectInputRef.current?.focus();
      projectInputRef.current?.select();
    });
  }

  function handleProjectCreate() {
    if (!isPremium) {
      dispatch({
        type: "set_sync_message",
        message: "Upgrade to Premium to create projects",
      });
      return;
    }
    setProjectRemovePending(false);
    dispatch({ type: "new_project" });
  }

  function handleProjectRename() {
    setProjectRemovePending(false);
    dispatch({ type: "rename_project" });
  }

  function handleProjectRemove() {
    if (projects.length <= 1) {
      return;
    }
    if (!projectRemovePending) {
      setProjectRemovePending(true);
      return;
    }
    setProjectRemovePending(false);
    dispatch({ type: "remove_project" });
  }

  function handleCancelProjectRemove() {
    setProjectRemovePending(false);
  }

  async function bootstrapSession(masterPasswordHint?: string) {
    setIsBootstrapping(true);
    setAuthMessage(null);
    const token = readAccessTokenFromLocation();
    if (!token) {
      setAccessToken(null);
      setAccount(null);
      setVaultKeyHex(null);
      dispatch({ type: "hydrate", snapshot: loadWorkspace() });
      dispatch({
        type: "set_sync_status",
        status: "locked",
        message: "Log in to enable sync",
      });
      setIsBootstrapping(false);
      return;
    }

    try {
      setAccessToken(token);
      const nextAccount = await fetchAccount(token);
      setAccount(nextAccount);
      const shouldSeedRegisteredAccount = consumeRegisteredAccountReset(nextAccount.email);
      if (shouldSeedRegisteredAccount) {
        clearAccountClientState(nextAccount.email);
      }
      const cachedVaultKey = readCachedVaultKey(nextAccount.email);
      let unlockedVaultKey = cachedVaultKey;
      if (!unlockedVaultKey) {
        const masterPassword = masterPasswordHint ?? readMasterPasswordHandoff();
        if (!masterPassword) {
          dispatch({
            type: "set_sync_status",
            status: "locked",
            message: "Enter master password to unlock sync",
          });
          setIsBootstrapping(false);
          return;
        }
        try {
          unlockedVaultKey = await unlockVaultKey(token, masterPassword);
          cacheUnlockedVaultKey(nextAccount.email, unlockedVaultKey);
        } catch (error) {
          setVaultKeyHex(null);
          dispatch({
            type: "set_sync_status",
            status: "locked",
            message: "Master password required",
          });
          setAuthMessage(error instanceof Error ? error.message : "Vault unlock failed");
          setIsBootstrapping(false);
          return;
        }
      }
      setVaultKeyHex(unlockedVaultKey);
      const storageKey = workspaceStorageKey(nextAccount.email);
      const localWorkspace = shouldSeedRegisteredAccount
        ? createBlankWorkspace()
        : loadWorkspace(storageKey);
      dispatch({ type: "hydrate", snapshot: localWorkspace });
      dispatch({
        type: "set_sync_status",
        status: "syncing",
        message: "Checking server workspace",
      });
      const remoteWorkspace = await pullWorkspaceSnapshot(
        token,
        nextAccount.email,
        unlockedVaultKey,
        localWorkspace,
      );
      if (remoteWorkspace) {
        const hydrated = createSavedSnapshot(remoteWorkspace, new Date().toISOString());
        persistWorkspace(hydrated, storageKey);
        dispatch({ type: "hydrate", snapshot: hydrated });
        dispatch({
          type: "set_sync_status",
          status: "saved",
          message: "Server workspace loaded",
        });
      } else if (shouldSeedRegisteredAccount) {
        const seededWorkspace = createSeedWorkspace();
        persistWorkspace(seededWorkspace, storageKey);
        dispatch({ type: "hydrate", snapshot: seededWorkspace });
        await pushWorkspaceSnapshot(
          token,
          nextAccount.email,
          unlockedVaultKey,
          seededWorkspace,
        );
        const hydrated = createSavedSnapshot(seededWorkspace, new Date().toISOString());
        persistWorkspace(hydrated, storageKey);
        dispatch({ type: "hydrate", snapshot: hydrated });
        dispatch({
          type: "set_sync_status",
          status: "saved",
          message: "Starter workspace created",
        });
      } else {
        persistWorkspace(localWorkspace, storageKey);
        dispatch({
          type: "set_sync_status",
          status: "saved",
          message: "Logged in, local workspace ready",
        });
      }
    } catch (error) {
      clearWebSession();
      setAccessToken(null);
      setAccount(null);
      setVaultKeyHex(null);
      dispatch({ type: "hydrate", snapshot: loadWorkspace() });
      dispatch({
        type: "set_sync_status",
        status: "locked",
        message: error instanceof Error ? error.message : "Login session failed",
      });
      setAuthMessage(error instanceof Error ? error.message : "Login session failed");
    } finally {
      setIsBootstrapping(false);
    }
  }

  async function handleLogin(email: string, masterPassword: string) {
    setIsSubmittingLogin(true);
    setAuthMessage(null);
    try {
      await loginWithMasterPassword(email, masterPassword);
      await bootstrapSession(masterPassword);
      window.history.replaceState({}, "", window.location.pathname);
    } catch (error) {
      setAuthMessage(error instanceof Error ? error.message : "Login failed");
    } finally {
      setIsSubmittingLogin(false);
    }
  }

  async function handleUnlock(masterPassword: string) {
    setIsSubmittingLogin(true);
    setAuthMessage(null);
    try {
      await bootstrapSession(masterPassword);
      window.history.replaceState({}, "", window.location.pathname);
    } catch (error) {
      setAuthMessage(error instanceof Error ? error.message : "Unlock failed");
    } finally {
      setIsSubmittingLogin(false);
    }
  }

  async function handleLogout() {
    const token = latestAccessToken.current;
    const email = latestAccount.current?.email ?? null;
    setIsBootstrapping(true);
    setAuthMessage(null);
    try {
      if (token) {
        await logoutFromServer(token);
      } else {
        clearWebSession();
      }
    } catch (error) {
      setAuthMessage(error instanceof Error ? error.message : "Logout failed");
    } finally {
      if (email) {
        clearCachedVaultKey(email);
      }
      setAccessToken(null);
      setAccount(null);
      setVaultKeyHex(null);
      dispatch({ type: "hydrate", snapshot: loadWorkspace() });
      dispatch({
        type: "set_sync_status",
        status: "locked",
        message: "Logged out",
      });
      window.history.replaceState({}, "", window.location.pathname);
      setIsBootstrapping(false);
    }
  }

  const hasSession = Boolean(account && accessToken);
  const isAuthenticated = Boolean(account && accessToken && vaultKeyHex);

  if (hasSession && !vaultKeyHex) {
    return (
      <UnlockScreen
        busy={isBootstrapping || isSubmittingLogin}
        email={account?.email ?? ""}
        message={authMessage}
        onUnlock={handleUnlock}
        onLogout={handleLogout}
      />
    );
  }

  if (!isAuthenticated) {
    return (
      <AuthScreen
        busy={isBootstrapping || isSubmittingLogin}
        message={authMessage}
        onLogin={handleLogin}
      />
    );
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
          <span className="mono-meta">{account ? account.email : "not logged in"}</span>
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
          <button className="button button-ghost" onClick={handleLogout}>
            Logout
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
              ref={projectInputRef}
              className="terminal-input"
              value={state.projectDraft}
              onChange={(event) =>
                dispatch({ type: "set_project_draft", value: event.target.value })
              }
              placeholder="Project name"
            />
            <div className="project-actions">
              <button
                className="button button-ghost"
                onClick={handleProjectCreate}
                disabled={!isPremium}
                title={isPremium ? "Create project" : "Premium required"}
              >
                Create
              </button>
              <button className="button button-ghost" onClick={handleProjectRename}>
                Rename
              </button>
              <button
                className="button button-ghost"
                onClick={handleProjectRemove}
                disabled={projects.length <= 1}
              >
                {projectRemovePending ? "Confirm remove" : "Remove"}
              </button>
            </div>

            {projectRemovePending ? (
              <div className="delete-confirmation" role="status" aria-live="polite">
                <div className="delete-confirmation-copy">
                  <strong>Delete this project?</strong>
                  <span>
                    Tasks move to{" "}
                    {projects.find((project) => project.id !== selectedProject?.id)?.name ??
                      "the fallback project"}
                    .
                  </span>
                </div>
                <div className="delete-confirmation-actions">
                  <button className="button button-danger" onClick={handleProjectRemove}>
                    Confirm
                  </button>
                  <button className="button button-ghost" onClick={handleCancelProjectRemove}>
                    Cancel
                  </button>
                </div>
              </div>
            ) : null}
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
            <div className="task-head-actions">
              <div className="shortcut-strip" aria-label="Task shortcuts">
                <kbd>j/k</kbd>
                <span>select</span>
                <kbd>space</kbd>
                <span>toggle</span>
                <kbd>/</kbd>
                <span>search</span>
                <kbd>a</kbd>
                <span>add</span>
              </div>
              <span className="panel-count">{visibleTasks.length}</span>
            </div>
          </div>

          <div className="task-toolbar">
            <input
              ref={searchInputRef}
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
                const active = task.id === activeTask?.id;
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

function AuthScreen(props: {
  busy: boolean;
  message: string | null;
  onLogin: (email: string, masterPassword: string) => Promise<void>;
}) {
  const [email, setEmail] = useState("");
  const [masterPassword, setMasterPassword] = useState("");

  return (
    <div className="auth-shell">
      <main className="auth-panel">
        <div className="auth-copy">
          <div className="brand-kicker">LemonTodo</div>
          <h1>Login</h1>
          <p>Sign in here, then enter the encrypted workspace directly.</p>
        </div>

        <form
          className="auth-form"
          onSubmit={(event) => {
            event.preventDefault();
            void props.onLogin(email.trim(), masterPassword);
          }}
        >
          <label className="field">
            <span>Email</span>
            <input
              className="terminal-input"
              type="email"
              autoComplete="username"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="you@example.com"
              required
              autoFocus
            />
          </label>

          <label className="field">
            <span>Master password</span>
            <input
              className="terminal-input"
              type="password"
              autoComplete="current-password"
              value={masterPassword}
              onChange={(event) => setMasterPassword(event.target.value)}
              placeholder="Vault unlock secret"
              required
            />
          </label>

          <div className="auth-actions">
            <button className="button button-primary" type="submit" disabled={props.busy}>
              {props.busy ? "Checking..." : "Log in"}
            </button>
            <a className="button button-ghost" href="/register">
              Register
            </a>
          </div>
        </form>

        <div className="auth-status" aria-live="polite">
          {props.message ?? "Server identity stays local to this host."}
        </div>
      </main>
    </div>
  );
}

function UnlockScreen(props: {
  busy: boolean;
  email: string;
  message: string | null;
  onUnlock: (masterPassword: string) => Promise<void>;
  onLogout: () => Promise<void>;
}) {
  const [masterPassword, setMasterPassword] = useState("");

  return (
    <div className="auth-shell">
      <main className="auth-panel">
        <div className="auth-copy">
          <div className="brand-kicker">LemonTodo</div>
          <h1>Unlock Sync</h1>
          <p>Enter the master password for {props.email} to unlock the encrypted workspace.</p>
        </div>

        <form
          className="auth-form"
          onSubmit={(event) => {
            event.preventDefault();
            void props.onUnlock(masterPassword);
          }}
        >
          <label className="field">
            <span>Account</span>
            <input className="terminal-input" value={props.email} readOnly />
          </label>

          <label className="field">
            <span>Master password</span>
            <input
              className="terminal-input"
              type="password"
              autoComplete="current-password"
              value={masterPassword}
              onChange={(event) => setMasterPassword(event.target.value)}
              placeholder="Vault unlock secret"
              required
              autoFocus
            />
          </label>

          <div className="auth-actions">
            <button className="button button-primary" type="submit" disabled={props.busy}>
              {props.busy ? "Unlocking..." : "Unlock"}
            </button>
            <button
              className="button button-ghost"
              type="button"
              onClick={() => void props.onLogout()}
              disabled={props.busy}
            >
              Logout
            </button>
          </div>
        </form>

        <div className="auth-status" aria-live="polite">
          {props.message ?? "Master password stays in this browser session only."}
        </div>
      </main>
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
    .sort((left, right) =>
      left.createdAt !== right.createdAt
        ? left.createdAt.localeCompare(right.createdAt)
        : left.id.localeCompare(right.id),
    );
}

function isTypingTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tagName = target.tagName.toLowerCase();
  return (
    target.isContentEditable ||
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select"
  );
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

function workspaceContentChanged(left: WorkspaceState, right: WorkspaceState) {
  return JSON.stringify(contentFingerprint(left)) !== JSON.stringify(contentFingerprint(right));
}

function contentFingerprint(state: WorkspaceState) {
  return {
    projects: [...state.projects].sort((left, right) => left.id.localeCompare(right.id)),
    tasks: [...state.tasks].sort((left, right) => left.id.localeCompare(right.id)),
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
