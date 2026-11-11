import { invoke } from "@tauri-apps/api/core";

export interface MyTicketDto {
  ticket: string;
  qr_svg: string;
}

export interface IncomingRequestDto {
  node_id: string;
  display_name: string | null;
}

export interface ConnectionStateDto {
  node_id: string;
  direction: "inbound" | "outbound";
  state: "connecting" | "connected" | "disconnected" | "failed";
  reason: "declined" | "timeout" | "unreachable" | "protocol" | "expired" | "closed" | null;
}

export interface LanPeerDto {
  node_id: string;
  display_name: string | null;
  verified: boolean;
  ticket: string;
}

export interface PresenceDto {
  enabled: boolean;
}

export type NetCmdError =
  | { type: "NotReady" }
  | { type: "InvalidTicket" }
  | { type: "SelfDial" }
  | { type: "NoSuchRequest" }
  | { type: "SettingsUnavailable" }
  | { type: "Internal" };

export const netMyTicket = () => invoke<MyTicketDto>("net_my_ticket");

export const netConnectTicket = (ticket: string) =>
  invoke<{ node_id: string }>("net_connect_ticket", { ticket });

export const netRespondIncoming = (nodeId: string, accept: boolean) =>
  invoke<void>("net_respond_incoming", { nodeId, accept });

export const netPendingIncoming = () => invoke<IncomingRequestDto[]>("net_pending_incoming");

export const netPresenceGet = () => invoke<PresenceDto>("net_presence_get");

export const netPresenceSet = (enabled: boolean) =>
  invoke<PresenceDto>("net_presence_set", { enabled });

export const netLanPeers = () => invoke<LanPeerDto[]>("net_lan_peers");

export function describeNetError(err: unknown): string {
  const e = err as NetCmdError | undefined;
  switch (e?.type) {
    case "NotReady":
      return "The network is still starting. Try again in a moment.";
    case "InvalidTicket":
      return "That ticket is not valid.";
    case "SelfDial":
      return "That's your own ticket.";
    case "NoSuchRequest":
      return "That request is no longer pending.";
    case "SettingsUnavailable":
      return "Settings are unavailable right now.";
    default:
      return "Something went wrong.";
  }
}
