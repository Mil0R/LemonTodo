import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { argon2idAsync } from "@noble/hashes/argon2.js";
import { bytesToHex, hexToBytes, utf8ToBytes } from "@noble/hashes/utils.js";
import * as ExpoCrypto from "expo-crypto";
import { fromByteArray, toByteArray } from "base64-js";
import { NativeModules, Platform } from "react-native";

import { debugLog } from "./debug";

const VAULT_KEY_LEN = 32;
const AUTH_MEMORY_COST_KIB = 64 * 1024;
const AUTH_TIME_COST = 3;
const AUTH_PARALLELISM = 1;
const ARGON2_VERSION = 0x13;
const ARGON2_ASYNC_TICK_MS = 4;
const VAULT_KEY_AAD = "lemontodo:vault-key:v1";

interface NativeArgon2Result {
  rawHash: string;
  encodedHash: string;
}

interface NativeArgon2Module {
  argon2(
    password: string,
    salt: string,
    options: {
      iterations: number;
      memory: number;
      parallelism: number;
      hashLength: number;
      mode: "argon2id";
      saltEncoding: "hex";
    },
  ): Promise<NativeArgon2Result>;
}

export interface CryptoEnvelope {
  version: number;
  cipher: string;
  nonce: string;
  ciphertext: string;
}

export interface KdfParams {
  algorithm: string;
  version: number;
  memory_cost_kib: number;
  time_cost: number;
  parallelism: number;
  salt: string;
}

export interface EncryptedVaultKey {
  kdf: KdfParams;
  envelope: CryptoEnvelope;
}

export function generateVaultKeyHex(): string {
  return bytesToHex(ExpoCrypto.getRandomBytes(VAULT_KEY_LEN));
}

export async function deriveAuthHash(email: string, masterPassword: string): Promise<string> {
  const normalizedEmail = email.trim().toLowerCase();
  if (!normalizedEmail) {
    throw new Error("Email is required.");
  }
  if (!masterPassword) {
    throw new Error("Master password is required.");
  }
  const salt = `lemontodo:auth:v1:${normalizedEmail}`;
  const startedAt = Date.now();
  debugLog("CRYPTO", "Argon2 auth derivation start", { memoryKiB: AUTH_MEMORY_COST_KIB, iterations: AUTH_TIME_COST });
  const output = await deriveArgon2Key(masterPassword, utf8ToBytes(salt), {
    memoryCostKiB: AUTH_MEMORY_COST_KIB,
    timeCost: AUTH_TIME_COST,
    parallelism: AUTH_PARALLELISM,
    kind: "auth",
  });
  debugLog("CRYPTO", "Argon2 auth derivation complete", { durationMs: Date.now() - startedAt });
  return bytesToHex(output);
}

export async function unwrapVaultKeyHex(
  encryptedVaultKey: EncryptedVaultKey,
  masterPassword: string,
): Promise<string> {
  const wrappingKey = await deriveWrappingKey(masterPassword, encryptedVaultKey.kdf);
  const plaintext = decryptEnvelope(
    wrappingKey,
    encryptedVaultKey.envelope,
    utf8ToBytes(VAULT_KEY_AAD),
  );
  if (plaintext.length !== VAULT_KEY_LEN) {
    throw new Error(`Vault key must be ${VAULT_KEY_LEN} bytes.`);
  }
  return bytesToHex(plaintext);
}

export async function wrapVaultKeyHex(
  vaultKeyHex: string,
  masterPassword: string,
): Promise<EncryptedVaultKey> {
  const kdf = generateKdfParams();
  const wrappingKey = await deriveWrappingKey(masterPassword, kdf);
  return {
    kdf,
    envelope: encryptEnvelope(
      wrappingKey,
      hexToBytes(vaultKeyHex),
      utf8ToBytes(VAULT_KEY_AAD),
    ),
  };
}

export function decryptPayload(vaultKeyHex: string, envelope: CryptoEnvelope, aad: string): string {
  const plaintext = decryptEnvelope(hexToBytes(vaultKeyHex), envelope, utf8ToBytes(aad));
  return new TextDecoder().decode(plaintext);
}

export function encryptPayload(vaultKeyHex: string, plaintext: string, aad: string): CryptoEnvelope {
  return encryptEnvelope(hexToBytes(vaultKeyHex), utf8ToBytes(plaintext), utf8ToBytes(aad));
}

function encryptEnvelope(key: Uint8Array, plaintext: Uint8Array, aad: Uint8Array): CryptoEnvelope {
  const nonce = ExpoCrypto.getRandomBytes(24);
  const ciphertext = xchacha20poly1305(key, nonce, aad).encrypt(plaintext);
  return {
    version: 1,
    cipher: "xchacha20poly1305",
    nonce: fromByteArray(nonce),
    ciphertext: fromByteArray(ciphertext),
  };
}

function generateKdfParams(): KdfParams {
  return {
    algorithm: "argon2id",
    version: 19,
    memory_cost_kib: 64 * 1024,
    time_cost: 3,
    parallelism: 1,
    salt: fromByteArray(ExpoCrypto.getRandomBytes(16)),
  };
}

async function deriveWrappingKey(masterPassword: string, kdf: KdfParams): Promise<Uint8Array> {
  if (kdf.algorithm !== "argon2id") {
    throw new Error(`Unsupported KDF algorithm: ${kdf.algorithm}`);
  }
  if (kdf.version !== 19) {
    throw new Error(`Unsupported Argon2 version: ${kdf.version}`);
  }
  const startedAt = Date.now();
  debugLog("CRYPTO", "Argon2 vault derivation start", {
    memoryKiB: kdf.memory_cost_kib,
    iterations: kdf.time_cost,
  });
  const key = await deriveArgon2Key(masterPassword, decodeBase64(kdf.salt), {
    memoryCostKiB: kdf.memory_cost_kib,
    timeCost: kdf.time_cost,
    parallelism: kdf.parallelism,
    kind: "vault",
  });
  debugLog("CRYPTO", "Argon2 vault derivation complete", { durationMs: Date.now() - startedAt });
  return key;
}

async function deriveArgon2Key(
  password: string,
  salt: Uint8Array,
  options: {
    memoryCostKiB: number;
    timeCost: number;
    parallelism: number;
    kind: "auth" | "vault";
  },
): Promise<Uint8Array> {
  if (Platform.OS === "web") {
    return argon2idAsync(utf8ToBytes(password), salt, {
      m: options.memoryCostKiB,
      t: options.timeCost,
      p: options.parallelism,
      dkLen: VAULT_KEY_LEN,
      version: ARGON2_VERSION,
      maxmem: 128 * 1024 * 1024,
      asyncTick: ARGON2_ASYNC_TICK_MS,
      onProgress: createProgressLogger(options.kind),
    });
  }

  const nativeArgon2 = NativeModules.RNArgon2 as NativeArgon2Module | undefined;
  if (!nativeArgon2?.argon2) {
    throw new Error(
      "Native Argon2 is unavailable in Expo Go. Install the LemonTodo development build with `npm run android`.",
    );
  }
  debugLog("CRYPTO", `native Argon2 ${options.kind} derivation dispatched`);
  const result = await nativeArgon2.argon2(password, bytesToHex(salt), {
    iterations: options.timeCost,
    memory: options.memoryCostKiB,
    parallelism: options.parallelism,
    hashLength: VAULT_KEY_LEN,
    mode: "argon2id",
    saltEncoding: "hex",
  });
  if (!/^[0-9a-f]{64}$/i.test(result.rawHash)) {
    throw new Error("Native Argon2 returned an invalid hash.");
  }
  return hexToBytes(result.rawHash);
}

function createProgressLogger(kind: "auth" | "vault"): (progress: number) => void {
  let nextMilestone = 0.25;
  return (progress) => {
    if (progress < nextMilestone) {
      return;
    }
    debugLog("CRYPTO", `Argon2 ${kind} derivation progress`, {
      percent: Math.min(100, Math.round(progress * 100)),
    });
    nextMilestone += 0.25;
  };
}

function decryptEnvelope(key: Uint8Array, envelope: CryptoEnvelope, aad: Uint8Array): Uint8Array {
  if (envelope.version !== 1) {
    throw new Error(`Unsupported envelope version: ${envelope.version}`);
  }
  if (envelope.cipher !== "xchacha20poly1305") {
    throw new Error(`Unsupported cipher: ${envelope.cipher}`);
  }
  const nonce = decodeBase64(envelope.nonce);
  if (nonce.length !== 24) {
    throw new Error(`XChaCha20-Poly1305 nonce must be 24 bytes.`);
  }
  return xchacha20poly1305(key, nonce, aad).decrypt(decodeBase64(envelope.ciphertext));
}

function decodeBase64(value: string): Uint8Array {
  return toByteArray(value);
}
