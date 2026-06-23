import * as Crypto from "expo-crypto";
import {
  documentDirectory,
  getInfoAsync,
  makeDirectoryAsync,
  readAsStringAsync,
  writeAsStringAsync,
} from "expo-file-system/legacy";
import { Platform } from "react-native";
import type { Workspace } from "./sync";

const DEVICE_ID_KEY = "lemontodo.mobile.device_id";
const SESSION_KEY = "lemontodo.mobile.session";
const WORKSPACE_KEY_PREFIX = "lemontodo.mobile.workspace";
const STORAGE_DIR_NAME = "lemontodo";

export interface StoredSession {
  serverUrl: string;
  email: string;
  accessToken: string;
}

export async function getOrCreateDeviceId(): Promise<string> {
  const existing = await readStorageValue(DEVICE_ID_KEY);
  if (existing) {
    return existing;
  }
  const next = Crypto.randomUUID();
  await writeStorageValue(DEVICE_ID_KEY, next);
  return next;
}

export async function saveSession(session: StoredSession): Promise<void> {
  await writeStorageValue(SESSION_KEY, JSON.stringify(session));
}

export async function loadSession(): Promise<StoredSession | null> {
  const raw = await readStorageValue(SESSION_KEY);
  if (!raw) {
    return null;
  }
  try {
    return JSON.parse(raw) as StoredSession;
  } catch {
    await removeStorageValue(SESSION_KEY);
    return null;
  }
}

export async function clearSession(): Promise<void> {
  await removeStorageValue(SESSION_KEY);
}

export async function saveWorkspace(accountEmail: string, workspace: Workspace): Promise<void> {
  await writeStorageValue(workspaceKey(accountEmail), JSON.stringify(workspace));
}

export async function loadWorkspace(accountEmail: string): Promise<Workspace | null> {
  const raw = await readStorageValue(workspaceKey(accountEmail));
  if (!raw) {
    return null;
  }
  try {
    return JSON.parse(raw) as Workspace;
  } catch {
    await removeStorageValue(workspaceKey(accountEmail));
    return null;
  }
}

export async function readStorageValue(key: string): Promise<string | null> {
  return readValue(key);
}

export async function writeStorageValue(key: string, value: string): Promise<void> {
  await writeValue(key, value);
}

export async function removeStorageValue(key: string): Promise<void> {
  await removeValue(key);
}

function workspaceKey(accountEmail: string): string {
  return `${WORKSPACE_KEY_PREFIX}:${accountEmail.trim().toLowerCase()}`;
}

async function readValue(key: string): Promise<string | null> {
  if (Platform.OS === "web") {
    return typeof localStorage === "undefined" ? null : localStorage.getItem(key);
  }
  const path = await storageFilePath(key);
  const info = await getInfoAsync(path);
  if (!info.exists) {
    return null;
  }
  return readAsStringAsync(path);
}

async function writeValue(key: string, value: string): Promise<void> {
  if (Platform.OS === "web") {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(key, value);
    }
    return;
  }
  const path = await storageFilePath(key);
  await writeAsStringAsync(path, value);
}

async function removeValue(key: string): Promise<void> {
  if (Platform.OS === "web") {
    if (typeof localStorage !== "undefined") {
      localStorage.removeItem(key);
    }
    return;
  }
  const path = await storageFilePath(key);
  const info = await getInfoAsync(path);
  if (info.exists) {
    await writeAsStringAsync(path, "");
  }
}

async function storageFilePath(key: string): Promise<string> {
  const dir = await ensureStorageDir();
  const safeKey = key.replace(/[^a-z0-9._-]+/gi, "_");
  return `${dir}${safeKey}.json`;
}

async function ensureStorageDir(): Promise<string> {
  if (!documentDirectory) {
    throw new Error("Local storage is unavailable on this device.");
  }
  const dir = `${documentDirectory}${STORAGE_DIR_NAME}/`;
  const info = await getInfoAsync(dir);
  if (!info.exists) {
    await makeDirectoryAsync(dir, { intermediates: true });
  }
  return dir;
}
