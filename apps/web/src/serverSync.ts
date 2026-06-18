import type { WorkspaceState } from "./workspace";

const PROTOCOL_VERSION = 1;
const DEVICE_ID_KEY = "lemontodo.web.device_id";
const CURSOR_KEY_PREFIX = "lemontodo.web.cursor";
const BASELINE_KEY_PREFIX = "lemontodo.web.baseline";
const TOKEN_KEY = "lemontodo.web.token";
const TOKEN_EXPIRY_KEY = "lemontodo.web.token_expires_at";
const UNLOCK_KEY_PREFIX = "lemontodo.web.unlock";
const WEB_TOKEN_TTL_MS = 1000 * 60 * 60 * 24 * 7;
const WEB_UNLOCK_TTL_MS = 1000 * 60 * 60 * 12;

export interface AccountStatus {
  user_id: string;
  email: string;
  is_admin: boolean;
  has_vault_key: boolean;
}

interface VaultMetadataResponse {
  has_vault_key: boolean;
  encrypted_vault_key: unknown | null;
}

interface SyncObject {
  id: string;
  operation_id: string;
  device_id: string;
  object_id: string;
  object_revision: number;
  object_type: string;
  operation_type: string;
  created_at: string;
  envelope: {
    version: number;
    cipher: string;
    nonce: string;
    ciphertext: string;
  };
}

interface SyncPullResponse {
  cursor: string;
  objects: SyncObject[];
}

interface SyncPushResponse {
  cursor: string;
  rejected?: Array<{ operation_id: string; reason: string }>;
}

interface TodoList {
  id: string;
  name: string;
  revision: number;
  created_at: string;
  updated_at: string;
}

type TaskStatus = "open" | "done" | "archived";

interface TodoTask {
  id: string;
  list_id: string;
  revision: number;
  title: string;
  note_markdown: string;
  status: TaskStatus;
  tags: string[];
  due_date: string | null;
  sort_key: string;
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
}

interface TodoSnapshot {
  version: number;
  exported_at: string;
  lists: TodoList[];
  tasks: TodoTask[];
}

interface SyncOperation {
  id: string;
  device_id: string;
  object_id: string;
  object_revision: number;
  object_type: "task" | "list" | "snapshot";
  operation_type: "create" | "update" | "delete" | "archive" | "import_snapshot";
  payload: Record<string, unknown>;
  created_at: string;
  synced_at: string | null;
}

interface PushPlan {
  operations: SyncOperation[];
  nextBaseline: TodoSnapshot;
}

interface CachedUnlockSession {
  vaultKeyHex: string;
  expiresAt: string;
}

export function readAccessTokenFromLocation(): string | null {
  const url = new URL(window.location.href);
  const token = url.searchParams.get("access_token");
  if (!token) {
    return readStoredAccessToken();
  }
  persistAccessToken(token);
  url.searchParams.delete("access_token");
  window.history.replaceState({}, "", url.toString());
  return token;
}

export async function fetchAccount(accessToken: string): Promise<AccountStatus> {
  const response = await fetch(`/v1/account/me?access_token=${encodeURIComponent(accessToken)}`);
  if (!response.ok) {
    throw new Error((await response.text()) || "Session check failed.");
  }
  return response.json() as Promise<AccountStatus>;
}

export async function unlockVaultKey(
  accessToken: string,
  masterPassword: string,
): Promise<string> {
  const response = await fetch(
    `/v1/account/vault-key?access_token=${encodeURIComponent(accessToken)}`,
  );
  if (!response.ok) {
    throw new Error((await response.text()) || "Vault metadata fetch failed.");
  }
  const metadata = (await response.json()) as VaultMetadataResponse;
  if (!metadata.encrypted_vault_key) {
    throw new Error("Server account has no vault metadata.");
  }
  const wasm = await loadRegisterWasm();
  return wasm.unwrap_vault_key_hex(
    JSON.stringify(metadata.encrypted_vault_key),
    masterPassword,
  ) as string;
}

export async function pushWorkspaceSnapshot(
  accessToken: string,
  accountEmail: string,
  vaultKeyHex: string,
  workspace: WorkspaceState,
): Promise<string> {
  const baseline = readBaseline(accountEmail);
  const pushPlan = buildPushPlan(workspace, baseline);
  if (pushPlan.operations.length === 0) {
    return readCursor(accountEmail) ?? "";
  }

  const cursor = readCursor(accountEmail);
  const deviceId = getOrCreateStoredUuid(DEVICE_ID_KEY);
  const objects = await Promise.all(
    pushPlan.operations.map((operation) => encryptOperation(vaultKeyHex, operation)),
  );
  const response = await fetch("/v1/sync/push", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      protocol_version: PROTOCOL_VERSION,
      device_id: deviceId,
      access_token: accessToken,
      base_cursor: cursor,
      objects,
    }),
  });
  if (!response.ok) {
    throw new Error((await response.text()) || "Push failed.");
  }
  const payload = (await response.json()) as SyncPushResponse;
  if ((payload.rejected?.length ?? 0) > 0) {
    const reasons = payload.rejected?.map((entry) => entry.reason).join(", ");
    throw new Error(`Push rejected: ${reasons}`);
  }
  writeCursor(accountEmail, payload.cursor);
  writeBaseline(accountEmail, pushPlan.nextBaseline);
  return payload.cursor;
}

export async function pullWorkspaceSnapshot(
  accessToken: string,
  accountEmail: string,
  vaultKeyHex: string,
  baseWorkspace: WorkspaceState | null,
): Promise<WorkspaceState | null> {
  const cursor = readCursor(accountEmail);
  const deviceId = getOrCreateStoredUuid(DEVICE_ID_KEY);
  const response = await fetch("/v1/sync/pull", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      protocol_version: PROTOCOL_VERSION,
      device_id: deviceId,
      access_token: accessToken,
      cursor,
      limit: 100,
    }),
  });
  if (!response.ok) {
    throw new Error((await response.text()) || "Pull failed.");
  }
  const payload = (await response.json()) as SyncPullResponse;
  writeCursor(accountEmail, payload.cursor);
  if (payload.objects.length === 0) {
    const baseline = readBaseline(accountEmail);
    if (baseline && baseWorkspace) {
      const baselineWorkspace = snapshotToWorkspace(baseline, baseWorkspace);
      if (workspaceContentChanged(baseWorkspace, baselineWorkspace)) {
        return baselineWorkspace;
      }
    }
    return null;
  }

  const operations = await Promise.all(
    payload.objects.map((object) => decryptOperation(vaultKeyHex, object)),
  );
  const nextBaseline = replayOperationsOnSnapshot(
    readBaseline(accountEmail) ?? createEmptySnapshot(),
    operations,
  );
  writeBaseline(accountEmail, nextBaseline);
  return snapshotToWorkspace(nextBaseline, baseWorkspace ?? createEmptyWorkspace());
}

export function clearWebSession(): void {
  window.sessionStorage.removeItem("lemontodo_web_token");
  window.localStorage.removeItem(TOKEN_KEY);
  window.localStorage.removeItem(TOKEN_EXPIRY_KEY);
}

export function cacheUnlockedVaultKey(accountEmail: string, vaultKeyHex: string): void {
  const cached: CachedUnlockSession = {
    vaultKeyHex,
    expiresAt: new Date(Date.now() + WEB_UNLOCK_TTL_MS).toISOString(),
  };
  window.localStorage.setItem(unlockKey(accountEmail), JSON.stringify(cached));
}

export function readCachedVaultKey(accountEmail: string): string | null {
  const raw = window.localStorage.getItem(unlockKey(accountEmail));
  if (!raw) {
    return null;
  }
  try {
    const cached = JSON.parse(raw) as CachedUnlockSession;
    if (Date.parse(cached.expiresAt) <= Date.now()) {
      window.localStorage.removeItem(unlockKey(accountEmail));
      return null;
    }
    return cached.vaultKeyHex;
  } catch {
    window.localStorage.removeItem(unlockKey(accountEmail));
    return null;
  }
}

export function clearCachedVaultKey(accountEmail: string): void {
  window.localStorage.removeItem(unlockKey(accountEmail));
}

function buildPushPlan(workspace: WorkspaceState, baseline: TodoSnapshot | null): PushPlan {
  const deviceId = getOrCreateStoredUuid(DEVICE_ID_KEY);
  const current = materializeSnapshot(workspace, baseline);
  const base = baseline ?? createEmptySnapshot();
  if (JSON.stringify(base) === JSON.stringify(current)) {
    return {
      operations: [],
      nextBaseline: current,
    };
  }
  return diffSnapshot(deviceId, base, current);
}

function diffSnapshot(
  deviceId: string,
  baseline: TodoSnapshot,
  current: TodoSnapshot,
): PushPlan {
  const operations: SyncOperation[] = [];
  const nextLists = new Map(current.lists.map((entry) => [entry.id, entry]));
  const nextTasks = new Map(current.tasks.map((entry) => [entry.id, entry]));
  const baseLists = new Map(baseline.lists.map((entry) => [entry.id, entry]));
  const baseTasks = new Map(baseline.tasks.map((entry) => [entry.id, entry]));

  for (const list of current.lists) {
    const previous = baseLists.get(list.id);
    if (!previous) {
      operations.push(
        createOperation(deviceId, list.id, list.revision, "list", "create", { list }, list.updated_at),
      );
      continue;
    }
    if (previous.name !== list.name) {
      operations.push(
        createOperation(deviceId, list.id, list.revision, "list", "update", { list }, list.updated_at),
      );
    }
  }
  for (const list of baseline.lists) {
    if (!nextLists.has(list.id)) {
      const deletedAt = new Date().toISOString();
      operations.push(
        createOperation(
          deviceId,
          list.id,
          list.revision + 1,
          "list",
          "delete",
          {
            list: {
              ...list,
              revision: list.revision + 1,
              updated_at: deletedAt,
            },
          },
          deletedAt,
        ),
      );
    }
  }

  for (const task of current.tasks) {
    const previous = baseTasks.get(task.id);
    if (!previous) {
      operations.push(
        createOperation(deviceId, task.id, task.revision, "task", "create", { task }, task.updated_at),
      );
      continue;
    }
    if (tasksEqual(previous, task)) {
      continue;
    }
    const operationType =
      task.status === "archived" && previous.status !== "archived" ? "archive" : "update";
    operations.push(
      createOperation(deviceId, task.id, task.revision, "task", operationType, { task }, task.updated_at),
    );
  }
  for (const task of baseline.tasks) {
    if (!nextTasks.has(task.id)) {
      const deletedAt = new Date().toISOString();
      operations.push(
        createOperation(
          deviceId,
          task.id,
          task.revision + 1,
          "task",
          "delete",
          {
            task: {
              ...task,
              revision: task.revision + 1,
              updated_at: deletedAt,
              deleted_at: deletedAt,
            },
          },
          deletedAt,
        ),
      );
    }
  }

  return { operations, nextBaseline: current };
}

function createOperation(
  deviceId: string,
  objectId: string,
  objectRevision: number,
  objectType: SyncOperation["object_type"],
  operationType: SyncOperation["operation_type"],
  payload: Record<string, unknown>,
  createdAt: string,
): SyncOperation {
  return {
    id: crypto.randomUUID(),
    device_id: deviceId,
    object_id: objectId,
    object_revision: objectRevision,
    object_type: objectType,
    operation_type: operationType,
    payload,
    created_at: createdAt,
    synced_at: null,
  };
}

async function encryptOperation(vaultKeyHex: string, operation: SyncOperation): Promise<SyncObject> {
  const wasm = await loadRegisterWasm();
  const object: SyncObject = {
    id: crypto.randomUUID(),
    operation_id: operation.id,
    device_id: operation.device_id,
    object_id: operation.object_id,
    object_revision: operation.object_revision,
    object_type: operation.object_type,
    operation_type: operation.operation_type,
    created_at: operation.created_at,
    envelope: {
      version: 1,
      cipher: "xchacha20poly1305",
      nonce: "",
      ciphertext: "",
    },
  };
  object.envelope = JSON.parse(
    wasm.encrypt_payload(vaultKeyHex, JSON.stringify(operation), aadForObject(object)),
  ) as SyncObject["envelope"];
  return object;
}

async function decryptOperation(
  vaultKeyHex: string,
  object: SyncObject,
): Promise<SyncOperation> {
  const wasm = await loadRegisterWasm();
  return JSON.parse(
    wasm.decrypt_payload(vaultKeyHex, JSON.stringify(object.envelope), aadForObject(object)),
  ) as SyncOperation;
}

function replayOperationsOnSnapshot(
  baseline: TodoSnapshot,
  operations: SyncOperation[],
): TodoSnapshot {
  const sorted = [...operations].sort((left, right) => {
    if (left.created_at !== right.created_at) {
      return left.created_at.localeCompare(right.created_at);
    }
    return operationOrder(left) - operationOrder(right) || left.object_revision - right.object_revision;
  });

  let snapshot = cloneSnapshot(baseline);
  for (const operation of sorted) {
    switch (`${operation.object_type}:${operation.operation_type}`) {
      case "list:create":
      case "list:update":
        snapshot = applyRemoteList(snapshot, operation);
        break;
      case "list:delete":
        snapshot = applyRemoteListDelete(snapshot, operation);
        break;
      case "task:create":
      case "task:update":
      case "task:archive":
        snapshot = applyRemoteTask(snapshot, operation);
        break;
      case "task:delete":
        snapshot = applyRemoteTaskDelete(snapshot, operation);
        break;
      case "snapshot:import_snapshot":
        snapshot = applyRemoteSnapshot(snapshot, operation);
        break;
      default:
        break;
    }
  }
  snapshot.exported_at = sorted[sorted.length - 1]?.created_at ?? snapshot.exported_at;
  return snapshot;
}

function applyRemoteList(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.list as TodoList | undefined;
  if (!payload) {
    return snapshot;
  }
  const lists = snapshot.lists.some((list) => list.id === payload.id)
    ? snapshot.lists.map((list) => (list.id === payload.id ? payload : list))
    : [...snapshot.lists, payload];
  return {
    ...snapshot,
    lists: sortLists(lists),
  };
}

function applyRemoteTask(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.task as TodoTask | undefined;
  if (!payload) {
    return snapshot;
  }
  const tasks = snapshot.tasks.some((task) => task.id === payload.id)
    ? snapshot.tasks.map((task) => (task.id === payload.id ? payload : task))
    : [...snapshot.tasks, payload];
  const lists = snapshot.lists.some((list) => list.id === payload.list_id)
    ? snapshot.lists
    : [
        ...snapshot.lists,
        {
          id: payload.list_id,
          name: "Imported",
          revision: 1,
          created_at: payload.created_at,
          updated_at: payload.updated_at,
        },
      ];
  return {
    ...snapshot,
    lists: sortLists(lists),
    tasks: sortTasks(tasks),
  };
}

function applyRemoteSnapshot(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.snapshot as TodoSnapshot | undefined;
  if (!payload) {
    return snapshot;
  }
  return cloneSnapshot(payload);
}

function applyRemoteListDelete(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.list as TodoList | undefined;
  if (!payload) {
    return snapshot;
  }
  return {
    ...snapshot,
    lists: snapshot.lists.filter((list) => list.id !== payload.id),
    tasks: snapshot.tasks.filter((task) => task.list_id !== payload.id),
  };
}

function applyRemoteTaskDelete(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.task as TodoTask | undefined;
  if (!payload) {
    return snapshot;
  }
  return {
    ...snapshot,
    tasks: snapshot.tasks.filter((task) => task.id !== payload.id),
  };
}

function materializeSnapshot(
  workspace: WorkspaceState,
  baseline: TodoSnapshot | null,
): TodoSnapshot {
  const now = new Date().toISOString();
  const baselineLists = new Map(baseline?.lists.map((entry) => [entry.id, entry]) ?? []);
  const baselineTasks = new Map(baseline?.tasks.map((entry) => [entry.id, entry]) ?? []);

  const lists = workspace.projects.map((project) => {
    const previous = baselineLists.get(project.id);
    const createdAt = previous?.created_at ?? firstTaskTimestamp(workspace, project.id) ?? now;
    const updatedAt =
      previous?.name !== project.name
        ? now
        : previous?.updated_at ?? latestTaskTimestamp(workspace, project.id) ?? createdAt;
    const revision =
      previous === undefined ? 1 : previous.name === project.name ? previous.revision : previous.revision + 1;
    return {
      id: project.id,
      name: project.name,
      revision,
      created_at: createdAt,
      updated_at: updatedAt,
    };
  });

  const tasks = workspace.tasks.map((task) => {
    const previous = baselineTasks.get(task.id);
    const currentTags = normalizeTags(task.tags);
    const previousComparable = previous
      ? comparableTask({
          projectId: previous.list_id,
          title: previous.title,
          note: previous.note_markdown,
          tags: previous.tags,
          dueDate: previous.due_date ?? "",
          status: previous.status,
        })
      : null;
    const currentComparable = comparableTask(task);
    const changed =
      previous !== undefined &&
      JSON.stringify(previousComparable) !== JSON.stringify(currentComparable);
    return {
      id: task.id,
      list_id: task.projectId,
      revision: previous === undefined ? 1 : changed ? previous.revision + 1 : previous.revision,
      title: task.title,
      note_markdown: task.note,
      status: task.status,
      tags: currentTags,
      due_date: task.dueDate || null,
      sort_key: task.createdAt,
      created_at: previous?.created_at ?? task.createdAt,
      updated_at: changed ? task.updatedAt : previous?.updated_at ?? task.updatedAt,
      deleted_at: null,
    };
  });

  return {
    version: 1,
    exported_at: now,
    lists: sortLists(lists),
    tasks: sortTasks(tasks),
  };
}

function snapshotToWorkspace(
  snapshot: TodoSnapshot,
  previous: WorkspaceState,
): WorkspaceState {
  const projects = sortProjects(snapshot.lists.map((list) => ({ id: list.id, name: list.name })));
  const tasks = snapshot.tasks
    .filter((task) => task.deleted_at === null)
    .map((task) => ({
      id: task.id,
      projectId: task.list_id,
      title: task.title,
      note: task.note_markdown,
      tags: normalizeTags(task.tags),
      dueDate: task.due_date ?? "",
      status: task.status,
      createdAt: task.created_at,
      updatedAt: task.updated_at,
    }));
  const currentProjectId =
    projects.find((project) => project.id === previous.currentProjectId)?.id ??
    projects[0]?.id ??
    "";
  const selectedTaskId =
    tasks.find((task) => task.id === previous.selectedTaskId)?.id ??
    tasks.find((task) => task.projectId === currentProjectId)?.id ??
    tasks[0]?.id ??
    null;
  const selectedTask = tasks.find((task) => task.id === selectedTaskId) ?? null;
  const revision = inferWorkspaceRevision(snapshot);

  return {
    revision,
    savedRevision: revision,
    currentProjectId,
    selectedTaskId,
    search: previous.search,
    projectDraft:
      projects.find((project) => project.id === currentProjectId)?.name ?? "Inbox",
    draft: selectedTask
      ? {
          title: selectedTask.title,
          note: selectedTask.note,
          tags: selectedTask.tags.join(" "),
          dueDate: selectedTask.dueDate,
          projectId: selectedTask.projectId,
          status: selectedTask.status,
        }
      : {
          title: "",
          note: "",
          tags: "",
          dueDate: "",
          projectId: currentProjectId,
          status: "open",
        },
    projects,
    tasks: sortWorkspaceTasks(tasks),
    sync: { ...previous.sync },
  };
}

function createEmptyWorkspace(): WorkspaceState {
  return {
    revision: 1,
    savedRevision: 1,
    currentProjectId: "",
    selectedTaskId: null,
    search: "",
    projectDraft: "",
    draft: {
      title: "",
      note: "",
      tags: "",
      dueDate: "",
      projectId: "",
      status: "open",
    },
    projects: [],
    tasks: [],
    sync: {
      enabled: true,
      status: "saved",
      message: "Ready",
      lastSyncedAt: null,
    },
  };
}

function createEmptySnapshot(): TodoSnapshot {
  return {
    version: 1,
    exported_at: new Date(0).toISOString(),
    lists: [],
    tasks: [],
  };
}

function readCursor(accountEmail: string): string | null {
  return window.localStorage.getItem(cursorKey(accountEmail));
}

function writeCursor(accountEmail: string, cursor: string): void {
  window.localStorage.setItem(cursorKey(accountEmail), cursor);
}

function cursorKey(accountEmail: string): string {
  return `${CURSOR_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`;
}

function unlockKey(accountEmail: string): string {
  return `${UNLOCK_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`;
}

function baselineKey(accountEmail: string): string {
  return `${BASELINE_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`;
}

function readBaseline(accountEmail: string): TodoSnapshot | null {
  const raw = window.localStorage.getItem(baselineKey(accountEmail));
  if (!raw) {
    return null;
  }
  try {
    return JSON.parse(raw) as TodoSnapshot;
  } catch {
    return null;
  }
}

function writeBaseline(accountEmail: string, baseline: TodoSnapshot): void {
  window.localStorage.setItem(baselineKey(accountEmail), JSON.stringify(baseline));
}

function getOrCreateStoredUuid(key: string): string {
  const current = window.localStorage.getItem(key);
  if (current) {
    return current;
  }
  const value = crypto.randomUUID();
  window.localStorage.setItem(key, value);
  return value;
}

function readStoredAccessToken(): string | null {
  const token = window.sessionStorage.getItem("lemontodo_web_token");
  if (token) {
    return token;
  }
  const persistentToken = window.localStorage.getItem(TOKEN_KEY);
  const expiresAt = window.localStorage.getItem(TOKEN_EXPIRY_KEY);
  if (!persistentToken || !expiresAt) {
    return null;
  }
  if (Date.parse(expiresAt) <= Date.now()) {
    clearWebSession();
    return null;
  }
  window.sessionStorage.setItem("lemontodo_web_token", persistentToken);
  return persistentToken;
}

function persistAccessToken(token: string): void {
  const expiresAt = new Date(Date.now() + WEB_TOKEN_TTL_MS).toISOString();
  window.sessionStorage.setItem("lemontodo_web_token", token);
  window.localStorage.setItem(TOKEN_KEY, token);
  window.localStorage.setItem(TOKEN_EXPIRY_KEY, expiresAt);
}

function aadForObject(object: SyncObject): string {
  return [
    "lemontodo-sync:v1",
    object.operation_id,
    object.device_id,
    object.object_id,
    object.object_revision,
    object.object_type,
    object.operation_type,
  ].join(":");
}

function workspaceContentChanged(left: WorkspaceState, right: WorkspaceState): boolean {
  return JSON.stringify(workspaceContentFingerprint(left)) !== JSON.stringify(workspaceContentFingerprint(right));
}

function workspaceContentFingerprint(state: WorkspaceState) {
  return {
    projects: [...state.projects].sort((left, right) => left.id.localeCompare(right.id)),
    tasks: [...state.tasks].sort((left, right) => left.id.localeCompare(right.id)),
  };
}

function tasksEqual(left: TodoTask, right: TodoTask): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function operationOrder(operation: SyncOperation): number {
  switch (`${operation.object_type}:${operation.operation_type}`) {
    case "list:create":
      return 0;
    case "list:update":
      return 1;
    case "task:create":
      return 2;
    case "task:update":
      return 3;
    case "task:archive":
      return 4;
    case "task:delete":
      return 5;
    case "list:delete":
      return 6;
    case "snapshot:import_snapshot":
      return 7;
    default:
      return 8;
  }
}

function inferSnapshotRevision(snapshot: TodoSnapshot): number {
  const taskRevision = snapshot.tasks.reduce((max, task) => Math.max(max, task.revision), 0);
  const listRevision = snapshot.lists.reduce((max, list) => Math.max(max, list.revision), 0);
  return Math.max(taskRevision, listRevision, snapshot.tasks.length + snapshot.lists.length, 1);
}

function inferWorkspaceRevision(snapshot: TodoSnapshot): number {
  return inferSnapshotRevision(snapshot);
}

function firstTaskTimestamp(workspace: WorkspaceState, projectId: string): string | null {
  const timestamps = workspace.tasks
    .filter((task) => task.projectId === projectId)
    .map((task) => task.createdAt)
    .sort();
  return timestamps[0] ?? null;
}

function latestTaskTimestamp(workspace: WorkspaceState, projectId: string): string | null {
  const timestamps = workspace.tasks
    .filter((task) => task.projectId === projectId)
    .map((task) => task.updatedAt)
    .sort();
  return timestamps[timestamps.length - 1] ?? null;
}

function comparableTask(task: {
  projectId: string;
  title: string;
  note: string;
  tags: string[];
  dueDate: string;
  status: TaskStatus;
}) {
  return {
    projectId: task.projectId,
    title: task.title,
    note: task.note,
    tags: normalizeTags(task.tags),
    dueDate: task.dueDate,
    status: task.status,
  };
}

function normalizeTags(tags: string[]): string[] {
  return [...tags].map((tag) => tag.trim().toLowerCase()).filter(Boolean).sort();
}

function sortLists(lists: TodoList[]): TodoList[] {
  return [...lists].sort((left, right) => left.name.localeCompare(right.name));
}

function sortProjects(projects: Array<{ id: string; name: string }>): Array<{ id: string; name: string }> {
  return [...projects].sort((left, right) => left.name.localeCompare(right.name));
}

function sortTasks(tasks: TodoTask[]): TodoTask[] {
  return [...tasks].sort((left, right) => right.updated_at.localeCompare(left.updated_at));
}

function sortWorkspaceTasks(
  tasks: Array<{
    id: string;
    projectId: string;
    title: string;
    note: string;
    tags: string[];
    dueDate: string;
    status: TaskStatus;
    createdAt: string;
    updatedAt: string;
  }>,
) {
  const order = { open: 0, done: 1, archived: 2 } as const;
  return [...tasks].sort((left, right) => {
    if (left.status !== right.status) {
      return order[left.status] - order[right.status];
    }
    return right.updatedAt.localeCompare(left.updatedAt);
  });
}

function cloneSnapshot(snapshot: TodoSnapshot): TodoSnapshot {
  return JSON.parse(JSON.stringify(snapshot)) as TodoSnapshot;
}

async function loadRegisterWasm(): Promise<{
  default: (input: string) => Promise<void>;
  unwrap_vault_key_hex: (encryptedVaultKeyJson: string, masterPassword: string) => string;
  encrypt_payload: (vaultKeyHex: string, plaintext: string, aad: string) => string;
  decrypt_payload: (vaultKeyHex: string, envelopeJson: string, aad: string) => string;
}> {
  const wasmModulePath = "/register/register_wasm.js";
  const wasm = await import(/* @vite-ignore */ wasmModulePath);
  await wasm.default("/register/register_wasm_bg.wasm");
  return wasm;
}
