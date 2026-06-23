import { StatusBar } from "expo-status-bar";
import * as Crypto from "expo-crypto";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Alert,
  Animated,
  AppState,
  type AppStateStatus,
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import {
  fetchAccount,
  fetchServerInfo,
  fetchVaultMetadata,
  login,
  logout,
  normalizeServerUrl,
  register,
  type AccountStatus,
  type Plan,
  type ServerInfo,
} from "./src/client";
import { unwrapVaultKeyHex } from "./src/crypto";
import { debugEmail, debugError, debugLog, debugServerUrl } from "./src/debug";
import {
  getOrCreateDeviceId,
  clearSession,
  loadSession,
  loadWorkspace,
  saveSession,
  saveWorkspace,
} from "./src/storage";
import { pullWorkspace, pushWorkspace, type Project, type Task, type Workspace } from "./src/sync";

type Screen = "server" | "login" | "register" | "unlock" | "projects" | "tasks" | "taskDetail";

const now = new Date().toISOString();
const inboxProjectId = "00000000-0000-4000-8000-000000000001";
const seedProjects: Project[] = [{ id: inboxProjectId, name: "Inbox", archived: false, createdAt: now }];
const seedTasks: Task[] = [
  {
    id: "00000000-0000-4000-8000-000000000002",
    projectId: inboxProjectId,
    title: "Welcome to LemonTodo mobile",
    note: "Real login and vault unlock are wired. Sync pull/push is the next slice.",
    dueDate: "",
    tags: ["mobile"],
    status: "open",
    createdAt: now,
    updatedAt: now,
  },
];

export default function App() {
  const [screen, setScreen] = useState<Screen>("server");
  const [serverUrl, setServerUrl] = useState("http://127.0.0.1:8787");
  const [email, setEmail] = useState("");
  const [masterPassword, setMasterPassword] = useState("");
  const [confirmMasterPassword, setConfirmMasterPassword] = useState("");
  const [plan, setPlan] = useState<Plan>("premium");
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [vaultKeyHex, setVaultKeyHex] = useState<string | null>(null);
  const [accessToken, setAccessToken] = useState<string | null>(null);
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [status, setStatus] = useState("Ready");
  const [statusTone, setStatusTone] = useState<"muted" | "error" | "success">("muted");
  const [busy, setBusy] = useState(false);
  const [projects, setProjects] = useState(seedProjects);
  const [tasks, setTasks] = useState(seedTasks);
  const [selectedProjectId, setSelectedProjectId] = useState(inboxProjectId);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [projectDraft, setProjectDraft] = useState("");
  const [taskDraft, setTaskDraft] = useState("");
  const syncingRef = useRef(false);
  const authOperationRef = useRef(0);

  const selectedProject = projects.find((project) => project.id === selectedProjectId) ?? projects[0];
  const selectedTask = tasks.find((task) => task.id === selectedTaskId) ?? null;
  const activeProjects = useMemo(
    () =>
      projects
        .filter((project) => !project.archived)
        .sort((left, right) => right.createdAt.localeCompare(left.createdAt)),
    [projects],
  );
  const archivedProjects = useMemo(
    () =>
      projects
        .filter((project) => project.archived)
        .sort((left, right) => right.createdAt.localeCompare(left.createdAt)),
    [projects],
  );
  const currentTasks = useMemo(
    () =>
      tasks
        .filter((task) => task.projectId === selectedProject?.id && task.status !== "archived")
        .sort((left, right) => left.createdAt.localeCompare(right.createdAt)),
    [selectedProject?.id, tasks],
  );
  const archivedTasks = useMemo(
    () =>
      tasks
        .filter((task) => task.projectId === selectedProject?.id && task.status === "archived")
        .sort((left, right) => left.createdAt.localeCompare(right.createdAt)),
    [selectedProject?.id, tasks],
  );

  useEffect(() => {
    debugLog("STATE", "authentication UI changed", {
      screen,
      busy,
      status,
      signedIn: Boolean(accessToken),
    });
  }, [accessToken, busy, screen, status]);

  useEffect(() => {
    if (screen !== "projects" && screen !== "tasks" && screen !== "taskDetail") {
      return;
    }
    debugLog("NAV", "workspace navigation changed", {
      screen,
      selectedProjectId,
      selectedProjectFound: Boolean(selectedProject),
      selectedProjectName: selectedProject?.name,
      visibleTaskCount: currentTasks.length,
      projectCount: activeProjects.length,
    });
  }, [activeProjects.length, currentTasks.length, screen, selectedProject, selectedProjectId]);

  useEffect(() => {
    let cancelled = false;
    async function restoreSession() {
      debugLog("RESTORE", "session restore started");
      try {
        const [storedSession, nextDeviceId] = await Promise.all([loadSession(), getOrCreateDeviceId()]);
        debugLog("RESTORE", "local session loaded", { hasSession: Boolean(storedSession) });
        if (cancelled) {
          return;
        }
        setDeviceId(nextDeviceId);
        if (!storedSession) {
          return;
        }
        setServerUrl(storedSession.serverUrl);
        setEmail(storedSession.email);
        setAccessToken(storedSession.accessToken);
        const localWorkspace = await loadWorkspace(storedSession.email);
        if (!cancelled && localWorkspace) {
          applyWorkspace(localWorkspace);
        }
        const [nextInfo, nextAccount] = await Promise.all([
          fetchServerInfo(storedSession.serverUrl),
          fetchAccount(storedSession.serverUrl, storedSession.accessToken),
        ]);
        debugLog("RESTORE", "stored session verified", { plan: nextAccount.plan });
        if (cancelled) {
          return;
        }
        setServerInfo(nextInfo);
        setAccount(nextAccount);
        setPlan(nextAccount.plan);
        setStatus("Session restored, unlock vault");
        setStatusTone("muted");
        setScreen("unlock");
      } catch (error) {
        debugError("RESTORE", "session restore failed", error);
        if (!cancelled) {
          setStatus(errorMessage(error));
          setStatusTone("error");
        }
      }
    }
    void restoreSession();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!account) {
      return;
    }
    void saveWorkspace(account.email, currentWorkspace());
  }, [account, projects, tasks, selectedProjectId, selectedTaskId]);

  useEffect(() => {
    if (!account || !accessToken || !vaultKeyHex || !deviceId) {
      return;
    }
    const timeout = setTimeout(() => {
      void syncWithRemote(false);
    }, 2_500);
    return () => clearTimeout(timeout);
  }, [
    account,
    accessToken,
    vaultKeyHex,
    deviceId,
    serverUrl,
    projects,
    tasks,
    selectedProjectId,
    selectedTaskId,
  ]);

  useEffect(() => {
    if (!account || !accessToken || !vaultKeyHex || !deviceId) {
      return;
    }
    const interval = setInterval(() => {
      void syncWithRemote(false);
    }, 30_000);
    const subscription = AppState.addEventListener("change", (nextState: AppStateStatus) => {
      if (nextState === "active") {
        void syncWithRemote(false);
      }
    });
    return () => {
      clearInterval(interval);
      subscription.remove();
    };
  }, [
    account,
    accessToken,
    vaultKeyHex,
    deviceId,
    serverUrl,
    projects,
    tasks,
    selectedProjectId,
    selectedTaskId,
  ]);

  async function handleServerContinue() {
    debugLog("UI", "Continue pressed", { screen: "server", server: debugServerUrl(serverUrl) });
    if (!serverUrl.trim()) {
      debugLog("UI", "server validation failed", { reason: "missing URL" });
      setStatus("Server URL is required.");
      setStatusTone("error");
      return;
    }
    const operationId = beginAuthOperation();
    debugLog("FLOW", "server connection started", { operationId });
    setStatus("Checking server");
    setStatusTone("muted");
    try {
      const normalized = normalizeServerUrl(serverUrl);
      debugLog("FLOW", "server URL normalized", { operationId, server: debugServerUrl(normalized) });
      const info = await fetchServerInfo(normalized);
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setServerUrl(normalized);
      setServerInfo(info);
      setStatus(`Server ${info.protocol_version}`);
      setStatusTone("success");
      setScreen("login");
      debugLog("FLOW", "server connection complete", { operationId, protocolVersion: info.protocol_version });
    } catch (error) {
      debugError("FLOW", "server connection failed", error, { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setStatus(errorMessage(error));
      setStatusTone("error");
    } finally {
      finishAuthOperation(operationId);
    }
  }

  async function handleUnlock() {
    debugLog("UI", "Unlock pressed", {
      screen: "unlock",
      email: debugEmail(email),
      passwordProvided: masterPassword.length > 0,
    });
    if (!email.trim()) {
      debugLog("UI", "unlock validation failed", { reason: "missing email" });
      setStatus("Email is required.");
      setStatusTone("error");
      setScreen("login");
      return;
    }
    if (!masterPassword) {
      debugLog("UI", "unlock validation failed", { reason: "missing master password" });
      setStatus("Master password is required.");
      setStatusTone("error");
      return;
    }
    const operationId = beginAuthOperation();
    debugLog("FLOW", "unlock started", { operationId, email: debugEmail(email) });
    setStatus("Signing in");
    setStatusTone("muted");
    try {
      const normalized = normalizeServerUrl(serverUrl);
      debugLog("FLOW", "device ID loading", { operationId });
      const deviceId = await getOrCreateDeviceId();
      debugLog("FLOW", "device ID ready", { operationId });
      debugLog("FLOW", "login request starting", { operationId });
      const session = await login(normalized, email, masterPassword, deviceId);
      debugLog("FLOW", "login request complete", { operationId, email: debugEmail(session.email) });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      await saveSession({
        serverUrl: normalized,
        email: session.email,
        accessToken: session.access_token,
      });
      debugLog("FLOW", "session saved", { operationId });
      debugLog("FLOW", "account metadata loading", { operationId });
      const nextAccount = await fetchAccount(normalized, session.access_token);
      const metadata = await fetchVaultMetadata(normalized, session.access_token);
      debugLog("FLOW", "account metadata ready", {
        operationId,
        plan: nextAccount.plan,
        hasVaultKey: Boolean(metadata.encrypted_vault_key),
      });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      if (!metadata.encrypted_vault_key) {
        throw new Error("Server account has no vault metadata.");
      }
      const nextVaultKeyHex = await unwrapVaultKeyHex(metadata.encrypted_vault_key, masterPassword);
      debugLog("FLOW", "vault key unwrapped", { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setAccount(nextAccount);
      setPlan(nextAccount.plan);
      setVaultKeyHex(nextVaultKeyHex);
      setAccessToken(session.access_token);
      setDeviceId(deviceId);
      const remote = await pullWorkspace({
        serverUrl: normalized,
        accessToken: session.access_token,
        accountEmail: session.email,
        vaultKeyHex: nextVaultKeyHex,
        deviceId,
        current: currentWorkspace(),
      });
      debugLog("FLOW", "initial pull complete", { operationId, remoteWorkspace: Boolean(remote) });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      if (remote) {
        applyWorkspace(remote);
        setStatus("Vault unlocked, remote loaded");
      } else {
        setStatus("Vault unlocked");
      }
      setStatusTone("success");
      setScreen("projects");
      debugLog("FLOW", "unlock complete", { operationId });
    } catch (error) {
      debugError("FLOW", "unlock failed", error, { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setStatus(errorMessage(error));
      setStatusTone("error");
    } finally {
      finishAuthOperation(operationId);
    }
  }

  async function handleRegister() {
    debugLog("UI", "Register pressed", {
      screen: "register",
      email: debugEmail(email),
      passwordProvided: masterPassword.length > 0,
      confirmationProvided: confirmMasterPassword.length > 0,
    });
    if (!email.trim()) {
      debugLog("UI", "registration validation failed", { reason: "missing email" });
      setStatus("Email is required.");
      setStatusTone("error");
      return;
    }
    if (!masterPassword) {
      debugLog("UI", "registration validation failed", { reason: "missing master password" });
      setStatus("Master password is required.");
      setStatusTone("error");
      return;
    }
    if (masterPassword !== confirmMasterPassword) {
      debugLog("UI", "registration validation failed", { reason: "password mismatch" });
      setStatus("Master passwords do not match.");
      setStatusTone("error");
      return;
    }
    const operationId = beginAuthOperation();
    debugLog("FLOW", "registration started", { operationId, email: debugEmail(email) });
    setStatus("Registering");
    setStatusTone("muted");
    try {
      const normalized = normalizeServerUrl(serverUrl);
      await register(normalized, email, masterPassword);
      debugLog("FLOW", "registration request complete", { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setStatus("Registered, signing in");
      const deviceId = await getOrCreateDeviceId();
      debugLog("FLOW", "post-registration login starting", { operationId });
      const session = await login(normalized, email, masterPassword, deviceId);
      debugLog("FLOW", "post-registration login complete", { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      await saveSession({
        serverUrl: normalized,
        email: session.email,
        accessToken: session.access_token,
      });
      debugLog("FLOW", "session saved", { operationId });
      const nextAccount = await fetchAccount(normalized, session.access_token);
      const metadata = await fetchVaultMetadata(normalized, session.access_token);
      debugLog("FLOW", "account metadata ready", {
        operationId,
        plan: nextAccount.plan,
        hasVaultKey: Boolean(metadata.encrypted_vault_key),
      });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      if (!metadata.encrypted_vault_key) {
        throw new Error("Server account has no vault metadata.");
      }
      const nextVaultKeyHex = await unwrapVaultKeyHex(metadata.encrypted_vault_key, masterPassword);
      debugLog("FLOW", "vault key unwrapped", { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setAccount(nextAccount);
      setPlan(nextAccount.plan);
      setVaultKeyHex(nextVaultKeyHex);
      setAccessToken(session.access_token);
      setDeviceId(deviceId);
      const remote = await pullWorkspace({
        serverUrl: normalized,
        accessToken: session.access_token,
        accountEmail: session.email,
        vaultKeyHex: nextVaultKeyHex,
        deviceId,
        current: currentWorkspace(),
      });
      debugLog("FLOW", "initial pull complete", { operationId, remoteWorkspace: Boolean(remote) });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      if (remote) {
        applyWorkspace(remote);
      }
      setStatus("Registered and unlocked");
      setStatusTone("success");
      setScreen("projects");
      debugLog("FLOW", "registration flow complete", { operationId });
    } catch (error) {
      debugError("FLOW", "registration failed", error, { operationId });
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      setStatus(errorMessage(error));
      setStatusTone("error");
    } finally {
      finishAuthOperation(operationId);
    }
  }

  async function handleSync() {
    await syncWithRemote(true);
  }

  async function syncWithRemote(showStatus: boolean) {
    if (!account || !accessToken || !vaultKeyHex || !deviceId) {
      setStatus("Unlock before sync.");
      return;
    }
    if (syncingRef.current) {
      return;
    }
    syncingRef.current = true;
    if (showStatus) {
      setBusy(true);
      setStatus("Syncing");
    }
    try {
      await pushWorkspace({
        serverUrl,
        accessToken,
        accountEmail: account.email,
        vaultKeyHex,
        deviceId,
        current: currentWorkspace(),
      });
      const remote = await pullWorkspace({
        serverUrl,
        accessToken,
        accountEmail: account.email,
        vaultKeyHex,
        deviceId,
        current: currentWorkspace(),
      });
      if (remote) {
        applyWorkspace(remote);
      }
      if (showStatus) {
        setStatus("Sync ok");
        setStatusTone("success");
      }
    } catch (error) {
      setStatus(errorMessage(error));
      setStatusTone("error");
    } finally {
      syncingRef.current = false;
      if (showStatus) {
        setBusy(false);
      }
    }
  }

  async function handleLogout() {
    const token = accessToken;
    const operationId = beginAuthOperation();
    setStatus("Logging out");
    setStatusTone("muted");
    try {
      if (token) {
        await logout(serverUrl, token);
      }
      await clearSession();
      setAccount(null);
      setVaultKeyHex(null);
      setAccessToken(null);
      setMasterPassword("");
      setConfirmMasterPassword("");
      setProjects(seedProjects);
      setTasks(seedTasks);
      setSelectedProjectId(inboxProjectId);
      setSelectedTaskId(null);
      setProjectDraft("");
      setTaskDraft("");
      setStatus("Logged out");
      setStatusTone("success");
      setScreen("login");
    } catch (error) {
      if (!isCurrentAuthOperation(operationId)) {
        return;
      }
      await clearSession();
      setAccount(null);
      setVaultKeyHex(null);
      setAccessToken(null);
      setMasterPassword("");
      setConfirmMasterPassword("");
      setStatus(errorMessage(error));
      setStatusTone("error");
      setScreen("login");
    } finally {
      finishAuthOperation(operationId);
    }
  }

  function beginAuthOperation(): number {
    authOperationRef.current += 1;
    setBusy(true);
    debugLog("STATE", "auth operation marked busy", { operationId: authOperationRef.current });
    return authOperationRef.current;
  }

  function cancelAuthOperation() {
    authOperationRef.current += 1;
    setBusy(false);
    debugLog("STATE", "auth operation cancelled", { nextOperationId: authOperationRef.current });
  }

  function finishAuthOperation(operationId: number) {
    if (isCurrentAuthOperation(operationId)) {
      setBusy(false);
      debugLog("STATE", "auth operation marked idle", { operationId });
    } else {
      debugLog("STATE", "stale auth operation ignored", { operationId, currentOperationId: authOperationRef.current });
    }
  }

  function isCurrentAuthOperation(operationId: number): boolean {
    return authOperationRef.current === operationId;
  }

  function addProject() {
    if (plan === "free" || !projectDraft.trim()) {
      return;
    }
    const project: Project = {
      id: Crypto.randomUUID(),
      name: projectDraft.trim(),
      archived: false,
      createdAt: new Date().toISOString(),
    };
    setProjects((current) => [project, ...current]);
    setSelectedProjectId(project.id);
    setProjectDraft("");
  }

  function archiveProject(projectId: string) {
    if (plan === "free" || projectId === inboxProjectId) {
      return;
    }
    setProjects((current) =>
      current.map((project) => (project.id === projectId ? { ...project, archived: true } : project)),
    );
    setSelectedProjectId(inboxProjectId);
  }

  function confirmArchiveProject(project: Project) {
    Alert.alert(
      "Archive project?",
      `\"${project.name}\" and its tasks will leave the active project list. You can restore them from Archived.`,
      [
        { text: "Cancel", style: "cancel" },
        { text: "Archive", style: "destructive", onPress: () => archiveProject(project.id) },
      ],
    );
  }

  function restoreProject(projectId: string) {
    setProjects((current) =>
      current.map((project) => (project.id === projectId ? { ...project, archived: false } : project)),
    );
    setStatus("Project restored");
    setStatusTone("success");
  }

  function addTask() {
    if (!selectedProject || !taskDraft.trim()) {
      return;
    }
    const task: Task = {
      id: Crypto.randomUUID(),
      projectId: selectedProject.id,
      title: taskDraft.trim(),
      note: "",
      dueDate: "",
      tags: [],
      status: "open",
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    };
    setTasks((current) => [...current, task]);
    setSelectedTaskId(task.id);
    setTaskDraft("");
  }

  function toggleTask(taskId: string) {
    setTasks((current) =>
      current.map((task) =>
        task.id === taskId
          ? { ...task, status: task.status === "done" ? "open" : "done", updatedAt: new Date().toISOString() }
          : task,
      ),
    );
  }

  function updateSelectedTask(patch: Partial<Task>) {
    if (!selectedTask) {
      return;
    }
    setTasks((current) =>
      current.map((task) =>
        task.id === selectedTask.id ? { ...task, ...patch, updatedAt: new Date().toISOString() } : task,
      ),
    );
  }

  function archiveSelectedTask() {
    if (!selectedTask) {
      return;
    }
    updateSelectedTask({ status: "archived" });
    setSelectedTaskId(null);
    setScreen("tasks");
  }

  function confirmArchiveSelectedTask() {
    if (!selectedTask) {
      return;
    }
    Alert.alert(
      "Archive task?",
      `\"${selectedTask.title}\" will leave the active task list. You can restore it from Archived.`,
      [
        { text: "Cancel", style: "cancel" },
        { text: "Archive", style: "destructive", onPress: archiveSelectedTask },
      ],
    );
  }

  function restoreTask(taskId: string) {
    setTasks((current) =>
      current.map((task) =>
        task.id === taskId ? { ...task, status: "open", updatedAt: new Date().toISOString() } : task,
      ),
    );
    setStatus("Task restored as open");
    setStatusTone("success");
  }

  function currentWorkspace(): Workspace {
    return {
      projects,
      tasks,
      selectedProjectId,
      selectedTaskId,
    };
  }

  function applyWorkspace(workspace: Workspace) {
    setProjects(workspace.projects.length ? workspace.projects : seedProjects);
    setTasks(workspace.tasks);
    setSelectedProjectId(workspace.selectedProjectId || workspace.projects[0]?.id || inboxProjectId);
    setSelectedTaskId(workspace.selectedTaskId);
  }

  const workspaceContent =
    screen === "projects" ? (
      <View style={styles.content}>
        <View style={styles.panelHeader}>
          <Text style={styles.panelLabel}>PROJECTS</Text>
          <Text style={styles.muted}>{activeProjects.length} active</Text>
        </View>
        <FlatList
          data={activeProjects}
          keyExtractor={(item) => item.id}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.listContent}
          ListFooterComponent={
            archivedProjects.length ? (
              <ArchiveSection label={`ARCHIVED PROJECTS  ${archivedProjects.length}`}>
                {archivedProjects.map((project) => {
                  const count = tasks.filter((task) => task.projectId === project.id).length;
                  return (
                    <View key={project.id} style={[styles.row, styles.archivedRow]}>
                      <View style={styles.rowMain}>
                        <Text style={styles.archivedTitle} numberOfLines={1}>{project.name}</Text>
                        <Text style={styles.rowMeta}>{count} tasks</Text>
                      </View>
                      <Pressable style={styles.restoreButton} onPress={() => restoreProject(project.id)}>
                        <Text style={styles.restoreButtonText}>RESTORE</Text>
                      </Pressable>
                    </View>
                  );
                })}
              </ArchiveSection>
            ) : null
          }
          renderItem={({ item }) => {
            const count = tasks.filter((task) => task.projectId === item.id && task.status !== "archived").length;
            return (
              <View style={[styles.row, item.id === selectedProjectId ? styles.rowSelected : null]}>
                <Pressable
                  style={styles.rowMain}
                  onPress={() => {
                    debugLog("UI", "project pressed", {
                      projectId: item.id,
                      projectName: item.name,
                      taskCount: count,
                    });
                    setSelectedProjectId(item.id);
                    setScreen("tasks");
                  }}
                >
                  <Text style={styles.rowTitle} numberOfLines={1}>
                    {item.name}
                  </Text>
                  <Text style={styles.rowMeta}>{count} tasks</Text>
                </Pressable>
                {plan === "premium" && item.id !== inboxProjectId ? (
                  <Pressable style={styles.smallButton} onPress={() => confirmArchiveProject(item)}>
                    <Text style={styles.smallButtonText}>ARCHIVE</Text>
                  </Pressable>
                ) : null}
              </View>
            );
          }}
        />
        <View style={styles.composer}>
          <TextInput
            editable={plan === "premium"}
            value={projectDraft}
            onChangeText={setProjectDraft}
            placeholder={plan === "free" ? "Free plan uses Inbox only" : "New project"}
            placeholderTextColor={colors.muted}
            style={[styles.input, styles.composerInput, plan === "free" ? styles.disabledInput : null]}
          />
          <Pressable
            disabled={plan === "free"}
            style={[styles.actionButton, plan === "free" ? styles.disabledButton : null]}
            onPress={addProject}
          >
            <Text style={styles.actionButtonText}>ADD</Text>
          </Pressable>
        </View>
      </View>
    ) : null;

  const taskContent =
    screen === "tasks" && selectedProject ? (
      <View style={styles.content}>
        <View style={styles.panelHeader}>
          <Pressable onPress={() => setScreen("projects")}>
            <Text style={styles.backText}>{"< PROJECTS"}</Text>
          </Pressable>
          <Text style={styles.muted}>{currentTasks.length} tasks</Text>
        </View>
        <FlatList
          data={currentTasks}
          keyExtractor={(item) => item.id}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.listContent}
          ListFooterComponent={
            archivedTasks.length ? (
              <ArchiveSection label={`ARCHIVED TASKS  ${archivedTasks.length}`}>
                {archivedTasks.map((task) => (
                  <View key={task.id} style={[styles.row, styles.taskRow, styles.archivedRow]}>
                    <View style={styles.rowMain}>
                      <Text style={styles.archivedTitle} numberOfLines={1}>{task.title}</Text>
                    </View>
                    <Pressable style={styles.restoreButton} onPress={() => restoreTask(task.id)}>
                      <Text style={styles.restoreButtonText}>RESTORE</Text>
                    </Pressable>
                  </View>
                ))}
              </ArchiveSection>
            ) : null
          }
          renderItem={({ item }) => (
            <Pressable
              style={[styles.row, styles.taskRow]}
              onPress={() => {
                setSelectedTaskId(item.id);
                setScreen("taskDetail");
              }}
            >
              <TaskCheckbox checked={item.status === "done"} onPress={() => toggleTask(item.id)} />
              <View style={styles.rowMain}>
                <Text style={[styles.rowTitle, item.status === "done" ? styles.doneText : null]} numberOfLines={1}>
                  {item.title}
                </Text>
              </View>
            </Pressable>
          )}
        />
        <View style={styles.composer}>
          <TextInput
            value={taskDraft}
            onChangeText={setTaskDraft}
            placeholder="Task title"
            placeholderTextColor={colors.muted}
            style={[styles.input, styles.composerInput]}
            returnKeyType="done"
            onSubmitEditing={addTask}
          />
          <Pressable style={styles.actionButton} onPress={addTask}>
            <Text style={styles.actionButtonText}>ADD</Text>
          </Pressable>
        </View>
      </View>
    ) : null;

  const taskDetailContent =
    screen === "taskDetail" && selectedTask ? (
      <ScrollView
        style={styles.content}
        contentContainerStyle={styles.detailContent}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode={Platform.OS === "ios" ? "interactive" : "none"}
      >
        <Pressable onPress={() => setScreen("tasks")}>
          <Text style={styles.backText}>{"< TASKS"}</Text>
        </Pressable>
        <Field label="TITLE" value={selectedTask.title} onChangeText={(title) => updateSelectedTask({ title })} />
        <Field label="NOTE" value={selectedTask.note} onChangeText={(note) => updateSelectedTask({ note })} multiline />
        <Field
          label="DUE"
          value={selectedTask.dueDate}
          onChangeText={(dueDate) => updateSelectedTask({ dueDate })}
          placeholder="YYYY-MM-DD"
        />
        <Field
          label="TAGS"
          value={selectedTask.tags.join(" ")}
          onChangeText={(value) => updateSelectedTask({ tags: value.split(/\s+/).filter(Boolean) })}
          placeholder="mobile sync"
        />
        <Pressable style={styles.dangerButton} onPress={confirmArchiveSelectedTask}>
          <Text style={styles.dangerButtonText}>ARCHIVE TASK</Text>
        </Pressable>
      </ScrollView>
    ) : null;

  const footer = (
    <View style={styles.footer}>
      <View style={styles.footerIdentity}>
        <Text style={styles.footerLabel}>ACCOUNT</Text>
        <Text style={styles.footerEmail}>{account?.email ?? (email.trim() || "not signed in")}</Text>
      </View>
      <View style={styles.footerBottomRow}>
        <View style={styles.footerStatusWrap}>
          <Text style={styles.footerLabel}>STATUS</Text>
          <Text
            style={[
              styles.footerStatus,
              statusTone === "error" ? styles.footerStatusError : null,
              statusTone === "success" ? styles.footerStatusSuccess : null,
            ]}
            numberOfLines={2}
          >
            {status}
          </Text>
        </View>
        <View style={styles.footerActions}>
          {vaultKeyHex ? (
            <Pressable
              disabled={busy}
              style={[styles.footerButton, busy ? styles.disabledButton : null]}
              onPress={handleSync}
            >
              <Text style={styles.footerButtonText}>SYNC</Text>
            </Pressable>
          ) : null}
          {accessToken ? (
            <Pressable
              disabled={busy}
              style={[styles.footerButton, busy ? styles.disabledButton : null]}
              onPress={handleLogout}
            >
              <Text style={styles.footerButtonText}>LOGOUT</Text>
            </Pressable>
          ) : null}
        </View>
      </View>
    </View>
  );

  return (
    <SafeAreaView style={styles.shell}>
      <StatusBar style="light" />
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={styles.keyboard}>
        <View style={styles.topbar}>
          <View style={styles.topbarText}>
            <Text style={styles.kicker}>LEMONTODO</Text>
            <Text style={styles.title}>{screenTitle(screen, selectedProject?.name)}</Text>
          </View>
          <View style={styles.statusPill}>
            <Text style={styles.statusText}>{plan.toUpperCase()}</Text>
          </View>
        </View>

        {screen === "server" ? (
          <AuthPanel
            eyebrow="SERVER"
            title="Connect workspace"
            caption="Use the same server URL as TUI and Web."
            actionLabel={busy ? "Checking" : "Continue"}
            statusMessage={status}
            statusTone={statusTone}
            disabled={busy}
            onAction={handleServerContinue}
          >
            <Field label="SERVER URL" value={serverUrl} onChangeText={setServerUrl} autoCapitalize="none" />
          </AuthPanel>
        ) : null}

        {screen === "login" ? (
          <AuthPanel
            eyebrow="ACCOUNT"
            title="Sign in"
            caption={serverInfo ? `Protocol ${serverInfo.protocol_version}` : "Server is ready."}
            actionLabel="Continue"
            secondaryLabel="Back"
            tertiaryLabel={serverInfo?.auth.registration_allowed ? "Register" : undefined}
            statusMessage={status}
            statusTone={statusTone}
            disabled={busy}
            onAction={() => {
              if (!email.trim()) {
                setStatus("Email is required.");
                setStatusTone("error");
                return;
              }
              setMasterPassword("");
              setStatus("Email confirmed, enter master password.");
              setStatusTone("muted");
              setScreen("unlock");
            }}
            onSecondary={() => {
              cancelAuthOperation();
              setStatus("Ready");
              setStatusTone("muted");
              setScreen("server");
            }}
            onTertiary={() => {
              cancelAuthOperation();
              setMasterPassword("");
              setConfirmMasterPassword("");
              setStatus("Ready to register");
              setStatusTone("muted");
              setScreen("register");
            }}
          >
            <Field
              label="EMAIL"
              value={email}
              onChangeText={setEmail}
              autoCapitalize="none"
              keyboardType="email-address"
            />
          </AuthPanel>
        ) : null}

        {screen === "register" ? (
          <AuthPanel
            eyebrow="ACCOUNT"
            title="Register"
            caption="Create the account and encrypted vault metadata on this server."
            actionLabel={busy ? "Registering" : "Register"}
            secondaryLabel="Back"
            statusMessage={status}
            statusTone={statusTone}
            disabled={busy}
            onAction={handleRegister}
            onSecondary={() => {
              cancelAuthOperation();
              setStatus("Ready");
              setStatusTone("muted");
              setScreen("login");
            }}
          >
            <Field
              label="EMAIL"
              value={email}
              onChangeText={setEmail}
              autoCapitalize="none"
              keyboardType="email-address"
            />
            <Field
              label="MASTER PASSWORD"
              value={masterPassword}
              onChangeText={setMasterPassword}
              secureTextEntry
            />
            <Field
              label="CONFIRM MASTER PASSWORD"
              value={confirmMasterPassword}
              onChangeText={setConfirmMasterPassword}
              secureTextEntry
              returnKeyType="done"
              onSubmitEditing={() => {
                if (!busy) {
                  void handleRegister();
                }
              }}
            />
          </AuthPanel>
        ) : null}

        {screen === "unlock" ? (
          <AuthPanel
            eyebrow="VAULT"
            title="Unlock local vault"
            caption="Current server auth uses the master-password derived auth hash."
            actionLabel={busy ? "Unlocking" : "Unlock"}
            secondaryLabel="Back"
            statusMessage={status}
            statusTone={statusTone}
            disabled={busy}
            onAction={handleUnlock}
            onSecondary={() => {
              cancelAuthOperation();
              setStatus("Ready");
              setStatusTone("muted");
              setScreen("login");
            }}
          >
            <Field label="MASTER PASSWORD" value={masterPassword} onChangeText={setMasterPassword} secureTextEntry />
          </AuthPanel>
        ) : null}

        {screen === "projects" || screen === "tasks" || screen === "taskDetail" ? (
          <KeyboardAvoidingView
            behavior={Platform.OS === "android" ? "height" : undefined}
            style={styles.workspaceKeyboard}
          >
            {workspaceContent}
            {taskContent}
            {taskDetailContent}
            {footer}
          </KeyboardAvoidingView>
        ) : (
          footer
        )}
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

function AuthPanel(props: {
  eyebrow: string;
  title: string;
  caption: string;
  actionLabel: string;
  statusMessage?: string;
  statusTone?: "muted" | "error" | "success";
  secondaryLabel?: string;
  tertiaryLabel?: string;
  disabled?: boolean;
  onAction: () => void;
  onSecondary?: () => void;
  onTertiary?: () => void;
  children: ReactNode;
}) {
  return (
    <ScrollView
      contentContainerStyle={styles.authScrollContent}
      keyboardShouldPersistTaps="always"
      keyboardDismissMode={Platform.OS === "ios" ? "interactive" : "none"}
      showsVerticalScrollIndicator={false}
    >
      <View style={styles.authWrap}>
        <View style={styles.card}>
          <Text style={styles.panelLabel}>{props.eyebrow}</Text>
          <Text style={styles.cardTitle}>{props.title}</Text>
          <Text style={styles.caption}>{props.caption}</Text>
          {props.statusMessage ? (
            <View
              style={[
                styles.authStatusBox,
                props.statusTone === "error"
                  ? styles.authStatusBoxError
                  : props.statusTone === "success"
                    ? styles.authStatusBoxSuccess
                    : null,
              ]}
            >
              <Text
                style={[
                  styles.authStatus,
                  props.statusTone === "error"
                    ? styles.authStatusError
                    : props.statusTone === "success"
                      ? styles.authStatusSuccess
                      : null,
                ]}
              >
                {props.statusMessage}
              </Text>
            </View>
          ) : null}
          <View style={styles.form}>{props.children}</View>
          <View style={styles.authActions}>
            {props.tertiaryLabel && props.onTertiary ? (
              <Pressable
                disabled={props.disabled}
                style={[styles.secondaryButton, props.disabled ? styles.disabledButton : null]}
                onPress={props.onTertiary}
              >
                <Text style={styles.secondaryButtonText}>{props.tertiaryLabel}</Text>
              </Pressable>
            ) : null}
            {props.secondaryLabel && props.onSecondary ? (
              <Pressable
                style={styles.secondaryButton}
                onPress={props.onSecondary}
              >
                <Text style={styles.secondaryButtonText}>{props.secondaryLabel}</Text>
              </Pressable>
            ) : null}
            <Pressable
              disabled={props.disabled}
              style={[styles.actionButton, props.disabled ? styles.disabledButton : null]}
              onPress={props.onAction}
            >
              <View style={styles.actionButtonInner}>
                {props.disabled ? <ActivityIndicator color={colors.green} size="small" /> : null}
                <Text style={styles.actionButtonText}>{props.actionLabel}</Text>
              </View>
            </Pressable>
          </View>
        </View>
      </View>
    </ScrollView>
  );
}

function Field(props: React.ComponentProps<typeof TextInput> & { label: string }) {
  const { label, style, ...inputProps } = props;
  const [focused, setFocused] = useState(false);
  return (
    <View style={styles.field}>
      <Text style={[styles.fieldLabel, focused ? styles.fieldLabelFocused : null]}>{label}</Text>
      <TextInput
        placeholderTextColor={colors.muted}
        selectionColor={colors.green}
        autoCorrect={false}
        spellCheck={false}
        style={[styles.input, focused ? styles.inputFocused : null, inputProps.multiline ? styles.textarea : null, style]}
        onFocus={(event) => {
          setFocused(true);
          inputProps.onFocus?.(event);
        }}
        onBlur={(event) => {
          setFocused(false);
          inputProps.onBlur?.(event);
        }}
        {...inputProps}
      />
    </View>
  );
}

function ArchiveSection(props: { label: string; children: ReactNode }) {
  return (
    <View style={styles.archiveSection}>
      <View style={styles.archiveHeader}>
        <Text style={styles.archiveLabel}>{props.label}</Text>
        <Text style={styles.archiveHint}>RESTORABLE</Text>
      </View>
      <View style={styles.archiveList}>{props.children}</View>
    </View>
  );
}

function TaskCheckbox(props: { checked: boolean; onPress: () => void }) {
  const scale = useRef(new Animated.Value(1)).current;
  const fill = useRef(new Animated.Value(props.checked ? 1 : 0)).current;

  useEffect(() => {
    const animation = Animated.parallel([
      Animated.sequence([
        Animated.spring(scale, {
          toValue: props.checked ? 0.9 : 0.96,
          speed: 30,
          bounciness: 0,
          useNativeDriver: false,
        }),
        Animated.spring(scale, {
          toValue: 1,
          speed: 20,
          bounciness: props.checked ? 8 : 6,
          useNativeDriver: false,
        }),
      ]),
      Animated.timing(fill, {
        toValue: props.checked ? 1 : 0,
        duration: props.checked ? 170 : 130,
        useNativeDriver: false,
      }),
    ]);
    animation.start();
    return () => animation.stop();
  }, [fill, props.checked, scale]);

  const animatedStyle = {
    transform: [{ scale }],
    backgroundColor: fill.interpolate({
      inputRange: [0, 1],
      outputRange: [colors.bg, colors.greenSoft],
    }),
    borderColor: fill.interpolate({
      inputRange: [0, 1],
      outputRange: [colors.line, colors.green],
    }),
  } as const;

  const markStyle = {
    opacity: fill,
    transform: [
      {
        scale: fill.interpolate({
          inputRange: [0, 1],
          outputRange: [0.72, 1],
        }),
      },
    ],
  } as const;

  return (
    <Pressable onPress={props.onPress} hitSlop={8}>
      <Animated.View style={[styles.checkbox, animatedStyle]}>
        <Animated.Text style={[styles.checkboxText, markStyle]}>x</Animated.Text>
      </Animated.View>
    </Pressable>
  );
}

function screenTitle(screen: Screen, projectName?: string) {
  if (screen === "tasks") {
    return projectName ?? "Tasks";
  }
  if (screen === "taskDetail") {
    return "Task detail";
  }
  if (screen === "projects") {
    return "Projects";
  }
  if (screen === "register") {
    return "Register";
  }
  return "Mobile client";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Operation failed.";
}

const monoFont = Platform.select({ ios: "Menlo", android: "monospace", default: "monospace" });
const colors = {
  bg: "#0d1117",
  panel: "#161b22",
  line: "#30363d",
  text: "#e6edf3",
  muted: "#8b949e",
  green: "#3fb950",
  greenSoft: "rgba(63, 185, 80, 0.14)",
  red: "#f85149",
};

const styles = StyleSheet.create({
  shell: {
    flex: 1,
    backgroundColor: colors.bg,
  },
  keyboard: {
    flex: 1,
    padding: 12,
    gap: 10,
  },
  authScrollContent: {
    flexGrow: 1,
    paddingBottom: 24,
  },
  topbar: {
    minHeight: 58,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.panel,
    paddingHorizontal: 12,
    paddingVertical: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  topbarText: {
    flex: 1,
    minWidth: 0,
  },
  kicker: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 11,
  },
  title: {
    color: colors.text,
    fontFamily: monoFont,
    fontSize: 17,
    fontWeight: "700",
  },
  statusPill: {
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: 8,
    paddingVertical: 5,
    backgroundColor: colors.greenSoft,
  },
  statusText: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 11,
    fontWeight: "700",
  },
  authWrap: {
    flex: 1,
    justifyContent: "center",
    paddingVertical: 16,
  },
  card: {
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.panel,
    padding: 16,
    gap: 10,
  },
  panelLabel: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 11,
  },
  cardTitle: {
    color: colors.text,
    fontFamily: monoFont,
    fontSize: 24,
    fontWeight: "800",
  },
  caption: {
    color: colors.muted,
    fontSize: 13,
    lineHeight: 19,
  },
  authStatus: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 12,
    lineHeight: 18,
  },
  authStatusBox: {
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.bg,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  authStatusBoxError: {
    borderColor: colors.red,
  },
  authStatusBoxSuccess: {
    borderColor: colors.green,
    backgroundColor: colors.greenSoft,
  },
  authStatusError: {
    color: colors.red,
  },
  authStatusSuccess: {
    color: colors.green,
  },
  form: {
    gap: 10,
    marginTop: 6,
  },
  field: {
    gap: 6,
  },
  fieldLabel: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 11,
  },
  fieldLabelFocused: {
    color: colors.green,
  },
  input: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.bg,
    color: colors.text,
    paddingHorizontal: 10,
    paddingVertical: 10,
    fontFamily: monoFont,
    fontSize: 14,
  },
  inputFocused: {
    borderColor: colors.green,
    backgroundColor: colors.greenSoft,
  },
  composerInput: {
    flex: 1,
  },
  textarea: {
    minHeight: 132,
    textAlignVertical: "top",
  },
  disabledInput: {
    opacity: 0.55,
  },
  authActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: 10,
    marginTop: 4,
  },
  actionButton: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: colors.green,
    backgroundColor: colors.greenSoft,
    paddingHorizontal: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  actionButtonText: {
    color: colors.green,
    fontFamily: monoFont,
    fontWeight: "800",
  },
  actionButtonInner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  disabledButton: {
    opacity: 0.4,
  },
  secondaryButton: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  secondaryButtonText: {
    color: colors.muted,
    fontFamily: monoFont,
    fontWeight: "800",
  },
  content: {
    flex: 1,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.panel,
    padding: 10,
  },
  panelHeader: {
    minHeight: 34,
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
  },
  muted: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 12,
  },
  listContent: {
    gap: 8,
    paddingBottom: 12,
  },
  row: {
    minHeight: 56,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.bg,
    paddingHorizontal: 10,
    paddingVertical: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  rowSelected: {
    borderColor: colors.green,
    backgroundColor: colors.greenSoft,
  },
  rowMain: {
    flex: 1,
    minWidth: 0,
    justifyContent: "center",
  },
  rowTitle: {
    color: colors.text,
    fontFamily: monoFont,
    fontSize: 15,
    lineHeight: 20,
    fontWeight: "700",
  },
  rowMeta: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 12,
    marginTop: 4,
  },
  taskRow: {
    minHeight: 48,
    maxHeight: 48,
    paddingVertical: 0,
  },
  doneText: {
    color: colors.muted,
    textDecorationLine: "line-through",
  },
  smallButton: {
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: 8,
    paddingVertical: 7,
  },
  smallButtonText: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 10,
    fontWeight: "800",
  },
  archiveSection: {
    marginTop: 12,
    paddingTop: 10,
    borderTopWidth: 1,
    borderTopColor: colors.line,
    gap: 8,
  },
  archiveHeader: {
    minHeight: 24,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  archiveLabel: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 11,
    fontWeight: "700",
  },
  archiveHint: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 9,
    fontWeight: "700",
  },
  archiveList: {
    gap: 8,
  },
  archivedRow: {
    borderStyle: "dashed",
    opacity: 0.82,
  },
  archivedTitle: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 14,
    lineHeight: 20,
    fontWeight: "700",
  },
  restoreButton: {
    minHeight: 32,
    borderWidth: 1,
    borderColor: colors.green,
    backgroundColor: colors.greenSoft,
    paddingHorizontal: 9,
    alignItems: "center",
    justifyContent: "center",
  },
  restoreButtonText: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 10,
    fontWeight: "800",
  },
  composer: {
    flexDirection: "row",
    gap: 8,
    paddingTop: 8,
    borderTopWidth: 1,
    borderTopColor: colors.line,
  },
  checkbox: {
    width: 26,
    height: 26,
    borderWidth: 1,
    borderColor: colors.line,
    alignItems: "center",
    justifyContent: "center",
  },
  checkboxText: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 13,
    fontWeight: "900",
  },
  backText: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 12,
    fontWeight: "800",
  },
  detailContent: {
    gap: 12,
    paddingBottom: 24,
  },
  dangerButton: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: colors.red,
    alignItems: "center",
    justifyContent: "center",
  },
  dangerButtonText: {
    color: colors.red,
    fontFamily: monoFont,
    fontWeight: "800",
  },
  footer: {
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.panel,
    paddingHorizontal: 10,
    paddingVertical: 9,
    gap: 8,
  },
  footerIdentity: {
    gap: 3,
  },
  footerBottomRow: {
    flexDirection: "row",
    alignItems: "flex-end",
    justifyContent: "space-between",
    gap: 10,
  },
  footerLabel: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 10,
  },
  footerEmail: {
    color: colors.text,
    fontFamily: monoFont,
    fontSize: 12,
    lineHeight: 18,
  },
  footerStatusWrap: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  footerStatus: {
    color: colors.muted,
    fontFamily: monoFont,
    fontSize: 11,
    lineHeight: 16,
  },
  footerStatusError: {
    color: colors.red,
  },
  footerStatusSuccess: {
    color: colors.green,
  },
  footerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  footerButton: {
    minHeight: 28,
    borderWidth: 1,
    borderColor: colors.green,
    paddingHorizontal: 8,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: colors.greenSoft,
  },
  footerButtonText: {
    color: colors.green,
    fontFamily: monoFont,
    fontSize: 11,
    fontWeight: "800",
  },
  workspaceKeyboard: {
    flex: 1,
    gap: 10,
  },
});
