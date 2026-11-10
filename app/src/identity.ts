import { invoke } from "@tauri-apps/api/core";
import { displayNameMessages } from "./lib/displayNameMessages";

export type StatusDto =
  | { type: "Present"; account_id: string; node_id: string }
  | { type: "Absent" }
  | { type: "Error"; reason: string };

export interface ProvisionDto {
  node_id: string;
  account_id: string;
}

export interface MyInfoDto {
  display_name: string | null;
  node_id: string;
  ticket: string;
  qr_svg: string;
}

export type DisplayNameKind =
  | "Empty"
  | "TooLong"
  | "ControlCharacter"
  | "BidiOverride";

export type IdentityCmdError =
  | { type: "AlreadyProvisioned" }
  | { type: "RecoverableErrorState"; reason: string }
  | { type: "KeychainAccessDenied" }
  | { type: "InvalidDisplayName"; kind: DisplayNameKind }
  | { type: "Internal" };

export const identityStatus = () => invoke<StatusDto>("identity_status");

export const identityProvision = () => invoke<ProvisionDto>("identity_provision");

export const identitySetDisplayName = (name: string) =>
  invoke<void>("identity_set_display_name", { name });

export const identityMyInfo = () => invoke<MyInfoDto>("identity_my_info");

export function describeError(err: unknown): string {
  const e = err as IdentityCmdError | undefined;
  switch (e?.type) {
    case "AlreadyProvisioned":
      return "An identity already exists on this device.";
    case "RecoverableErrorState":
      return e.reason;
    case "KeychainAccessDenied":
      return "Access to the system keychain was denied.";
    case "InvalidDisplayName":
      return displayNameMessages[e.kind];
    default:
      return "Something went wrong.";
  }
}
