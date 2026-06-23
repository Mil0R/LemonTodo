import * as Crypto from "expo-crypto";

import { normalizeServerUrl } from "./client";
import { decryptPayload, encryptPayload, type CryptoEnvelope } from "./crypto";
import { debugError, debugLog } from "./debug";
import { readStorageValue, writeStorageValue } from "./storage";

const PROTOCOL_VERSION = 1;
const REQUEST_TIMEOUT_MS = 15_000;
const CURSOR_KEY_PREFIX = "lemontodo.mobile.cursor";
const BASELINE_KEY_PREFIX = "lemontodo.mobile.baseline";

export type TaskStatus = "open" | "done" | "archived";

export interface Project {
  id: string;
  name: string;
  archived: boolean;
  createdAt: string;
}

export interface Task {
  id: string;
  projectId: string;
  title: string;
  note: string;
  dueDate: string;
  tags: string[];
  status: TaskStatus;
  createdAt: string;
  updatedAt: string;
}

export interface Workspace {
  projects: Project[];
  tasks: Task[];
  selectedProjectId: string;
  selectedTaskId: string | null;
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
  envelope: CryptoEnvelope;
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

interface SyncPullResponse {
  cursor: string;
  objects: SyncObject[];
}

interface SyncPushResponse {
  cursor: string;
  rejected?: Array<{ operation_id: string; reason: string }>;
}

interface PushPlan {
  operations: SyncOperation[];
  nextBaseline: TodoSnapshot;
}

interface TodoList {
  id: string;
  name: string;
  revision: number;
  created_at: string;
  updated_at: string;
}

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

export async function pullWorkspace(input: {
  serverUrl: string;
  accessToken: string;
  accountEmail: string;
  vaultKeyHex: string;
  deviceId: string;
  current: Workspace;
}): Promise<Workspace | null> {
  debugLog("SYNC", "pull preparation start", { account: input.accountEmail.slice(0, 1) + "***" });
  const cursor = await readCursor(input.accountEmail);
  const endpoint = "/v1/sync/pull";
  const response = await fetchWithTimeout(`${normalizeServerUrl(input.serverUrl)}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      protocol_version: PROTOCOL_VERSION,
      device_id: input.deviceId,
      access_token: input.accessToken,
      cursor,
      limit: 100,
    }),
  }, endpoint);
  const payload = await readJson<SyncPullResponse>(response, "Pull failed.");
  debugLog("SYNC", "pull response parsed", { objectCount: payload.objects.length, hasCursor: Boolean(payload.cursor) });
  await writeCursor(input.accountEmail, payload.cursor);

  if (payload.objects.length === 0) {
    const baseline = await readBaseline(input.accountEmail);
    if (!baseline) {
      return null;
    }
    const workspace = snapshotToWorkspace(baseline, input.current);
    return workspacesEqual(workspace, input.current) ? null : workspace;
  }

  const operations = payload.objects.map((object) => decryptOperation(input.vaultKeyHex, object));
  const nextBaseline = replayOperationsOnSnapshot(
    (await readBaseline(input.accountEmail)) ?? createEmptySnapshot(),
    operations,
  );
  await writeBaseline(input.accountEmail, nextBaseline);
  const workspace = snapshotToWorkspace(nextBaseline, input.current);
  return workspacesEqual(workspace, input.current) ? null : workspace;
}

export async function pushWorkspace(input: {
  serverUrl: string;
  accessToken: string;
  accountEmail: string;
  vaultKeyHex: string;
  deviceId: string;
  current: Workspace;
}): Promise<string> {
  const baseline = await readBaseline(input.accountEmail);
  const pushPlan = diffSnapshot(input.deviceId, baseline ?? createEmptySnapshot(), materializeSnapshot(input.current, baseline));
  if (pushPlan.operations.length === 0) {
    return (await readCursor(input.accountEmail)) ?? "";
  }
  const objects = pushPlan.operations.map((operation) => encryptOperation(input.vaultKeyHex, operation));
  const endpoint = "/v1/sync/push";
  const response = await fetchWithTimeout(`${normalizeServerUrl(input.serverUrl)}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      protocol_version: PROTOCOL_VERSION,
      device_id: input.deviceId,
      access_token: input.accessToken,
      base_cursor: await readCursor(input.accountEmail),
      objects,
    }),
  }, endpoint);
  const payload = await readJson<SyncPushResponse>(response, "Push failed.");
  if (payload.rejected?.length) {
    throw new Error(`Push rejected: ${payload.rejected.map((entry) => entry.reason).join(", ")}`);
  }
  await writeCursor(input.accountEmail, payload.cursor);
  await writeBaseline(input.accountEmail, pushPlan.nextBaseline);
  return payload.cursor;
}

function encryptOperation(vaultKeyHex: string, operation: SyncOperation): SyncObject {
  const object: SyncObject = {
    id: Crypto.randomUUID(),
    operation_id: operation.id,
    device_id: operation.device_id,
    object_id: operation.object_id,
    object_revision: operation.object_revision,
    object_type: operation.object_type,
    operation_type: operation.operation_type,
    created_at: operation.created_at,
    envelope: { version: 1, cipher: "xchacha20poly1305", nonce: "", ciphertext: "" },
  };
  object.envelope = encryptPayload(vaultKeyHex, JSON.stringify(operation), aadForObject(object));
  return object;
}

function decryptOperation(vaultKeyHex: string, object: SyncObject): SyncOperation {
  return JSON.parse(decryptPayload(vaultKeyHex, object.envelope, aadForObject(object))) as SyncOperation;
}

function diffSnapshot(deviceId: string, baseline: TodoSnapshot, current: TodoSnapshot): PushPlan {
  if (JSON.stringify(baseline) === JSON.stringify(current)) {
    return { operations: [], nextBaseline: current };
  }
  const operations: SyncOperation[] = [];
  const nextLists = new Map(current.lists.map((entry) => [entry.id, entry]));
  const nextTasks = new Map(current.tasks.map((entry) => [entry.id, entry]));
  const baseLists = new Map(baseline.lists.map((entry) => [entry.id, entry]));
  const baseTasks = new Map(baseline.tasks.map((entry) => [entry.id, entry]));

  for (const list of current.lists) {
    const previous = baseLists.get(list.id);
    if (!previous) {
      operations.push(createOperation(deviceId, list.id, list.revision, "list", "create", { list }, list.updated_at));
    } else if (previous.name !== list.name) {
      operations.push(createOperation(deviceId, list.id, list.revision, "list", "update", { list }, list.updated_at));
    }
  }
  for (const list of baseline.lists) {
    if (!nextLists.has(list.id)) {
      const deletedAt = new Date().toISOString();
      operations.push(
        createOperation(deviceId, list.id, list.revision + 1, "list", "delete", {
          list: { ...list, revision: list.revision + 1, updated_at: deletedAt },
        }, deletedAt),
      );
    }
  }
  for (const task of current.tasks) {
    const previous = baseTasks.get(task.id);
    if (!previous) {
      operations.push(createOperation(deviceId, task.id, task.revision, "task", "create", { task }, task.updated_at));
    } else if (!tasksEqual(previous, task)) {
      const operationType = task.status === "archived" && previous.status !== "archived" ? "archive" : "update";
      operations.push(createOperation(deviceId, task.id, task.revision, "task", operationType, { task }, task.updated_at));
    }
  }
  for (const task of baseline.tasks) {
    if (!nextTasks.has(task.id)) {
      const deletedAt = new Date().toISOString();
      operations.push(
        createOperation(deviceId, task.id, task.revision + 1, "task", "delete", {
          task: { ...task, revision: task.revision + 1, updated_at: deletedAt, deleted_at: deletedAt },
        }, deletedAt),
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
    id: Crypto.randomUUID(),
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

function materializeSnapshot(workspace: Workspace, baseline: TodoSnapshot | null): TodoSnapshot {
  const timestamp = new Date().toISOString();
  const baselineLists = new Map(baseline?.lists.map((entry) => [entry.id, entry]) ?? []);
  const baselineTasks = new Map(baseline?.tasks.map((entry) => [entry.id, entry]) ?? []);
  // List archive is not part of sync protocol v1. Keep archived projects in the
  // encrypted snapshot so a local archive never becomes a destructive delete.
  const lists = workspace.projects.map((project) => {
    const previous = baselineLists.get(project.id);
    const createdAt = previous?.created_at ?? project.createdAt;
    const changed = previous !== undefined && previous.name !== project.name;
    return {
      id: project.id,
      name: project.name,
      revision: previous === undefined ? 1 : changed ? previous.revision + 1 : previous.revision,
      created_at: createdAt,
      updated_at: changed ? timestamp : previous?.updated_at ?? createdAt,
    };
  });
  const projectIds = new Set(workspace.projects.map((project) => project.id));
  const tasks = workspace.tasks
    .filter((task) => projectIds.has(task.projectId))
    .map((task) => {
      const previous = baselineTasks.get(task.id);
      const changed =
        previous !== undefined &&
        JSON.stringify(comparableTaskFromTodo(previous)) !== JSON.stringify(comparableTask(task));
      return {
        id: task.id,
        list_id: task.projectId,
        revision: previous === undefined ? 1 : changed ? previous.revision + 1 : previous.revision,
        title: task.title,
        note_markdown: task.note,
        status: task.status,
        tags: normalizeTags(task.tags),
        due_date: task.dueDate || null,
        sort_key: task.createdAt,
        created_at: previous?.created_at ?? task.createdAt,
        updated_at: changed ? task.updatedAt : previous?.updated_at ?? task.updatedAt,
        deleted_at: null,
      };
    });
  return { version: 1, exported_at: timestamp, lists: sortLists(lists), tasks: sortTasks(tasks) };
}

function replayOperationsOnSnapshot(baseline: TodoSnapshot, operations: SyncOperation[]): TodoSnapshot {
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
      default:
        break;
    }
  }
  return { ...snapshot, exported_at: sorted[sorted.length - 1]?.created_at ?? snapshot.exported_at };
}

function applyRemoteList(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.list as TodoList | undefined;
  if (!payload) return snapshot;
  const lists = snapshot.lists.some((list) => list.id === payload.id)
    ? snapshot.lists.map((list) => (list.id === payload.id ? payload : list))
    : [...snapshot.lists, payload];
  return { ...snapshot, lists: sortLists(lists) };
}

function applyRemoteTask(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.task as TodoTask | undefined;
  if (!payload) return snapshot;
  const inbox = uniqueInboxList(snapshot.lists);
  const task = !snapshot.lists.some((list) => list.id === payload.list_id) && inbox
    ? { ...payload, list_id: inbox.id }
    : payload;
  const tasks = snapshot.tasks.some((entry) => entry.id === task.id)
    ? snapshot.tasks.map((entry) => (entry.id === task.id ? task : entry))
    : [...snapshot.tasks, task];
  return { ...snapshot, tasks: sortTasks(tasks) };
}

function applyRemoteListDelete(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.list as TodoList | undefined;
  if (!payload) return snapshot;
  return {
    ...snapshot,
    lists: snapshot.lists.filter((list) => list.id !== payload.id),
    tasks: snapshot.tasks.filter((task) => task.list_id !== payload.id),
  };
}

function applyRemoteTaskDelete(snapshot: TodoSnapshot, operation: SyncOperation): TodoSnapshot {
  const payload = operation.payload.task as TodoTask | undefined;
  if (!payload) return snapshot;
  return { ...snapshot, tasks: snapshot.tasks.filter((task) => task.id !== payload.id) };
}

function snapshotToWorkspace(snapshot: TodoSnapshot, previous: Workspace): Workspace {
  const previousProjects = new Map(previous.projects.map((project) => [project.id, project]));
  const projects = sortProjects(
    snapshot.lists.map((list) => ({
      id: list.id,
      name: list.name,
      archived: previousProjects.get(list.id)?.archived ?? false,
      createdAt: list.created_at,
    })),
  );
  const canonicalProjectId = projects.some((project) => project.id === previous.selectedProjectId)
    ? previous.selectedProjectId
    : projects[0]?.id ?? "";
  const tasks = sortWorkspaceTasks(
    snapshot.tasks
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
      })),
  );
  return {
    projects,
    tasks,
    selectedProjectId: canonicalProjectId,
    selectedTaskId: tasks.some((task) => task.id === previous.selectedTaskId)
      ? previous.selectedTaskId
      : tasks.find((task) => task.projectId === canonicalProjectId)?.id ?? null,
  };
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

async function readCursor(accountEmail: string): Promise<string | null> {
  return readStorageValue(`${CURSOR_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`);
}

async function writeCursor(accountEmail: string, cursor: string): Promise<void> {
  await writeStorageValue(`${CURSOR_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`, cursor);
}

async function readBaseline(accountEmail: string): Promise<TodoSnapshot | null> {
  const raw = await readStorageValue(`${BASELINE_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as TodoSnapshot;
  } catch {
    return null;
  }
}

async function writeBaseline(accountEmail: string, baseline: TodoSnapshot): Promise<void> {
  await writeStorageValue(
    `${BASELINE_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`,
    JSON.stringify(baseline),
  );
}

async function readJson<T>(response: Response, fallback: string): Promise<T> {
  if (!response.ok) {
    throw new Error((await response.text()) || fallback);
  }
  return response.json() as Promise<T>;
}

async function fetchWithTimeout(input: RequestInfo | URL, init?: RequestInit, endpoint = "sync request"): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  const startedAt = Date.now();
  debugLog("SYNC", "request start", { endpoint, timeoutMs: REQUEST_TIMEOUT_MS });
  try {
    const response = await fetch(input, { ...init, signal: controller.signal });
    debugLog("SYNC", "response received", {
      endpoint,
      status: response.status,
      ok: response.ok,
      durationMs: Date.now() - startedAt,
    });
    return response;
  } catch (error) {
    debugError("SYNC", "request failed", error, { endpoint, durationMs: Date.now() - startedAt });
    if (error instanceof Error && error.name === "AbortError") {
      throw new Error("Sync request timed out. Check server URL and network.");
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

function createEmptySnapshot(): TodoSnapshot {
  return { version: 1, exported_at: new Date(0).toISOString(), lists: [], tasks: [] };
}

function tasksEqual(left: TodoTask, right: TodoTask): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function comparableTask(task: Task) {
  return {
    projectId: task.projectId,
    title: task.title,
    note: task.note,
    tags: normalizeTags(task.tags),
    dueDate: task.dueDate,
    status: task.status,
  };
}

function comparableTaskFromTodo(task: TodoTask) {
  return {
    projectId: task.list_id,
    title: task.title,
    note: task.note_markdown,
    tags: normalizeTags(task.tags),
    dueDate: task.due_date ?? "",
    status: task.status,
  };
}

function normalizeTags(tags: string[]): string[] {
  return [...tags].map((tag) => tag.trim().toLowerCase()).filter(Boolean).sort();
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
    default:
      return 8;
  }
}

function sortLists(lists: TodoList[]): TodoList[] {
  return [...lists].sort((left, right) => right.created_at.localeCompare(left.created_at));
}

function sortProjects(projects: Project[]): Project[] {
  return [...projects].sort((left, right) => right.createdAt.localeCompare(left.createdAt));
}

function sortTasks(tasks: TodoTask[]): TodoTask[] {
  return [...tasks].sort((left, right) => left.created_at.localeCompare(right.created_at));
}

function sortWorkspaceTasks(tasks: Task[]): Task[] {
  return [...tasks].sort((left, right) => {
    if (left.createdAt !== right.createdAt) {
      return left.createdAt.localeCompare(right.createdAt);
    }
    return left.id.localeCompare(right.id);
  });
}

function uniqueInboxList(lists: TodoList[]): TodoList | null {
  const matches = lists.filter((list) => list.name.trim().toLowerCase() === "inbox");
  return matches.length === 1 ? matches[0] : null;
}

function cloneSnapshot(snapshot: TodoSnapshot): TodoSnapshot {
  return JSON.parse(JSON.stringify(snapshot)) as TodoSnapshot;
}

function workspacesEqual(left: Workspace, right: Workspace): boolean {
  return (
    left.selectedProjectId === right.selectedProjectId &&
    left.selectedTaskId === right.selectedTaskId &&
    JSON.stringify(left.projects) === JSON.stringify(right.projects) &&
    JSON.stringify(left.tasks) === JSON.stringify(right.tasks)
  );
}
