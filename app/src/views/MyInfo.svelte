<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import {
    identityMyInfo,
    identitySetDisplayName,
    describeError,
    type MyInfoDto,
  } from "../identity";
  import { netMyTicket } from "../net";

  let info = $state<MyInfoDto | null>(null);
  let loadError = $state<string | null>(null);
  let draft = $state("");
  let nameError = $state<string | null>(null);
  let saving = $state(false);
  let notice = $state<string | null>(null);
  let networkStarting = $state(false);

  async function load() {
    try {
      const base = await identityMyInfo();
      try {
        info = { ...base, ...(await netMyTicket()) };
        networkStarting = false;
      } catch {
        info = base;
        networkStarting = true;
      }
      draft = info.display_name ?? "";
      loadError = null;
    } catch (e) {
      loadError = describeError(e);
    }
  }

  async function refreshTicket() {
    try {
      const live = await netMyTicket();
      if (info) info = { ...info, ...live };
      networkStarting = false;
    } catch {
      networkStarting = true;
    }
  }

  onMount(() => {
    load();
    const unlisten = listen("my_ticket_changed", refreshTicket);
    return () => {
      unlisten.then((u) => u());
    };
  });

  async function saveName(ev: Event) {
    ev.preventDefault();
    nameError = null;
    saving = true;
    try {
      await identitySetDisplayName(draft);
      await load();
    } catch (e) {
      nameError = describeError(e);
    } finally {
      saving = false;
    }
  }

  async function copyTicket() {
    if (info) await copy(info.ticket, "Ticket copied");
  }

  async function copyNodeId() {
    if (info) await copy(info.node_id, "NodeId copied");
  }

  async function copy(value: string, msg: string) {
    try {
      await writeText(value);
      notice = msg;
    } catch {
      notice = "Copy failed";
    }
  }
</script>

<section>
  <h2>My Info</h2>
  {#if loadError}
    <p role="alert">{loadError}</p>
  {:else if info}
    <form onsubmit={saveName}>
      <label>
        Display name
        <input type="text" bind:value={draft} />
      </label>
      <button type="submit" disabled={saving}>Save</button>
      {#if nameError}<p role="alert">{nameError}</p>{/if}
    </form>
    <p>Current name: <span data-testid="display-name">{info.display_name ?? ""}</span></p>

    <h3>Connection ticket</h3>
    {#if networkStarting}<p>Network starting…</p>{/if}
    <button type="button" onclick={copyTicket}>Copy ticket</button>
    <pre data-testid="ticket" style="white-space: pre-wrap; word-break: break-all">{info.ticket}</pre>
    <div data-testid="qr" aria-label="Ticket QR code">{@html info.qr_svg}</div>

    <h3>NodeId</h3>
    <code data-testid="node-id" style="word-break: break-all">{info.node_id}</code>
    <button type="button" onclick={copyNodeId}>Copy NodeId</button>

    {#if notice}<p role="status">{notice}</p>{/if}
  {:else}
    <p>Loading…</p>
  {/if}
</section>
