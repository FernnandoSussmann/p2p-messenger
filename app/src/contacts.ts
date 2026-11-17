import { invoke } from "@tauri-apps/api/core";
import type { DisplayNameKind } from "./identity";
import { nicknameMessages } from "./lib/nicknameMessages";

export interface ContactDto {
  node_id: string;
  nickname: string;
  /** Not rendered in the contact list (user decision). Used by the naming
   *  prompt and shown on the contact-info screen only. */
  self_name: string | null;
  verified: boolean;
  added_at_ms: number;
  last_seen_ms: number | null;
}

export interface RemoveContactDto {
  node_id: string;
  history_erased: boolean;
}

export type ContactsCmdError =
  | { type: "NotReady" }
  | { type: "InvalidNodeId" }
  | { type: "SelfContact" }
  | { type: "InvalidNickname"; kind: DisplayNameKind }
  | { type: "AlreadyExists" }
  | { type: "NotFound" }
  | { type: "Full" }
  | { type: "StoreUnavailable" }
  | { type: "HistoryEraseFailed" };

export const contactsList = () => invoke<ContactDto[]>("contacts_list");
export const contactsAdd = (nodeId: string, nickname: string) =>
  invoke<ContactDto>("contacts_add", { nodeId, nickname });
export const contactsRename = (nodeId: string, nickname: string) =>
  invoke<ContactDto>("contacts_rename", { nodeId, nickname });
export const contactsSetVerified = (nodeId: string, verified: boolean) =>
  invoke<ContactDto>("contacts_set_verified", { nodeId, verified });
export const contactsRemove = (nodeId: string, clearHistory: boolean) =>
  invoke<RemoveContactDto>("contacts_remove", { nodeId, clearHistory });
export const contactsIsKnown = (nodeId: string) =>
  invoke<boolean>("contacts_is_known", { nodeId });

export function describeContactsError(err: unknown): string {
  const e = err as ContactsCmdError | undefined;
  switch (e?.type) {
    case "NotReady":
      return "Contacts are not available yet. Try again in a moment.";
    case "InvalidNodeId":
      return "That is not a valid NodeId.";
    case "SelfContact":
      return "That's you. You can't add yourself as a contact.";
    case "InvalidNickname":
      return nicknameMessages[e.kind] ?? "Something went wrong.";
    case "AlreadyExists":
      return "This person is already a contact.";
    case "NotFound":
      return "That contact no longer exists.";
    case "Full":
      return "Your contact list is full.";
    case "StoreUnavailable":
      return "Contacts could not be saved right now. Try again.";
    case "HistoryEraseFailed":
      return "Your message history could not be deleted, so the contact was not removed. Try again, or remove them without deleting history.";
    default:
      return "Something went wrong.";
  }
}
