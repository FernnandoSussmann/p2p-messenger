import { invoke } from "@tauri-apps/api/core";

export type StorageErrorKind =
  | "KeyMissing"
  | "Missing"
  | "KeychainDenied"
  | "KeychainUnavailable"
  | "NotEncrypted"
  | "WrongKeyOrCorrupt"
  | "Corrupt"
  | "NewerSchema"
  | "MigrationFailed"
  | "OtherAccount"
  | "LegacyImportFailed"
  | "Io";

export type StorageStatusDto =
  | { type: "Ready" }
  | { type: "NotProvisioned" }
  | { type: "Error"; kind: StorageErrorKind; reason: string };

export const storageStatus = () => invoke<StorageStatusDto>("storage_status");

export const storageRetry = () => invoke<StorageStatusDto>("storage_retry");
