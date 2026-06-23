import type { EncryptedVaultKey } from "./crypto";
import { deriveAuthHash, generateVaultKeyHex, wrapVaultKeyHex } from "./crypto";
import { debugEmail, debugError, debugLog, debugServerUrl } from "./debug";

const REQUEST_TIMEOUT_MS = 15_000;

export type Plan = "free" | "premium";

export interface ServerInfo {
  protocol_version: number;
  capabilities: string[];
  auth: {
    password_auth: boolean;
    registration_allowed: boolean;
  };
  limits: {
    max_push_objects: number;
    max_pull_objects: number;
    max_object_bytes: number;
  };
}

export interface AccountStatus {
  user_id: string;
  email: string;
  is_admin: boolean;
  has_vault_key: boolean;
  plan: Plan;
  billing: {
    enabled: boolean;
    provider: "stripe" | "creem" | "dodopayments" | null;
    monthly_price_cents: number;
    currency: string;
  };
}

export interface LoginSession {
  user_id: string;
  email: string;
  access_token: string;
}

export interface RegisterResult {
  user_id: string;
  email: string;
  is_admin: boolean;
  vaultKeyHex: string;
}

export interface VaultMetadataResponse {
  has_vault_key: boolean;
  encrypted_vault_key: EncryptedVaultKey | null;
}

export function normalizeServerUrl(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, "");
  if (!trimmed) {
    throw new Error("Server URL is required.");
  }
  return trimmed;
}

export async function fetchServerInfo(serverUrl: string): Promise<ServerInfo> {
  const endpoint = "/v1/server-info";
  debugLog("HTTP", "server-info request", { server: debugServerUrl(serverUrl) });
  const response = await fetchWithTimeout(`${normalizeServerUrl(serverUrl)}${endpoint}`, undefined, endpoint);
  return readJson<ServerInfo>(response, "Server check failed.");
}

export async function login(
  serverUrl: string,
  email: string,
  masterPassword: string,
  deviceId: string,
): Promise<LoginSession> {
  const normalizedEmail = email.trim();
  debugLog("AUTH", "login auth hash start", { email: debugEmail(normalizedEmail) });
  const authHash = await deriveAuthHash(normalizedEmail, masterPassword);
  debugLog("AUTH", "login auth hash complete", { email: debugEmail(normalizedEmail) });
  const endpoint = "/v1/account/login";
  const response = await fetchWithTimeout(`${normalizeServerUrl(serverUrl)}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      email: normalizedEmail,
      auth_hash: authHash,
      device_id: deviceId,
      device_name: "mobile",
    }),
  }, endpoint);
  return readJson<LoginSession>(response, "Login failed.");
}

export async function register(
  serverUrl: string,
  email: string,
  masterPassword: string,
): Promise<RegisterResult> {
  const normalizedEmail = email.trim();
  const vaultKeyHex = generateVaultKeyHex();
  // Mobile runtimes cannot reliably hold two 64 MiB Argon2 workspaces at once.
  debugLog("AUTH", "registration auth derivation start", { email: debugEmail(normalizedEmail) });
  const authHash = await deriveAuthHash(normalizedEmail, masterPassword);
  debugLog("AUTH", "registration auth derivation complete", { email: debugEmail(normalizedEmail) });
  await yieldToEventLoop();
  debugLog("AUTH", "registration vault derivation start", { email: debugEmail(normalizedEmail) });
  const encryptedVaultKey = await wrapVaultKeyHex(vaultKeyHex, masterPassword);
  debugLog("AUTH", "registration vault derivation complete", { email: debugEmail(normalizedEmail) });
  const endpoint = "/v1/account/register";
  const response = await fetchWithTimeout(`${normalizeServerUrl(serverUrl)}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      email: normalizedEmail,
      auth_hash: authHash,
      encrypted_vault_key: encryptedVaultKey,
    }),
  }, endpoint);
  const registered = await readJson<Omit<RegisterResult, "vaultKeyHex">>(response, "Registration failed.");
  return { ...registered, vaultKeyHex };
}

export async function fetchAccount(serverUrl: string, accessToken: string): Promise<AccountStatus> {
  const endpoint = "/v1/account/me";
  const response = await fetchWithTimeout(
    `${normalizeServerUrl(serverUrl)}/v1/account/me?access_token=${encodeURIComponent(accessToken)}`,
    undefined,
    endpoint,
  );
  return readJson<AccountStatus>(response, "Account check failed.");
}

export async function fetchVaultMetadata(
  serverUrl: string,
  accessToken: string,
): Promise<VaultMetadataResponse> {
  const endpoint = "/v1/account/vault-key";
  const response = await fetchWithTimeout(
    `${normalizeServerUrl(serverUrl)}/v1/account/vault-key?access_token=${encodeURIComponent(accessToken)}`,
    undefined,
    endpoint,
  );
  return readJson<VaultMetadataResponse>(response, "Vault metadata fetch failed.");
}

export async function logout(serverUrl: string, accessToken: string): Promise<void> {
  const endpoint = "/v1/account/logout";
  const response = await fetchWithTimeout(`${normalizeServerUrl(serverUrl)}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ access_token: accessToken }),
  }, endpoint);
  if (!response.ok) {
    throw new Error((await response.text()) || "Logout failed.");
  }
}

async function readJson<T>(response: Response, fallback: string): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new Error(text || fallback);
  }
  return response.json() as Promise<T>;
}

async function fetchWithTimeout(input: RequestInfo | URL, init?: RequestInit, endpoint = "request"): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  const startedAt = Date.now();
  debugLog("HTTP", "request start", { endpoint, method: init?.method ?? "GET", timeoutMs: REQUEST_TIMEOUT_MS });
  try {
    const response = await fetch(input, { ...init, signal: controller.signal });
    debugLog("HTTP", "response received", {
      endpoint,
      status: response.status,
      ok: response.ok,
      durationMs: Date.now() - startedAt,
    });
    return response;
  } catch (error) {
    debugError("HTTP", "request failed", error, { endpoint, durationMs: Date.now() - startedAt });
    if (error instanceof Error && error.name === "AbortError") {
      throw new Error("Request timed out. Check server URL and network.");
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

function yieldToEventLoop(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}
