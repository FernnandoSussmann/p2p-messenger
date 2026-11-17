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
  import { chooseTheme, theme } from "../lib/theme";
  import ThemePicker from "./ThemePicker.svelte";

  let info = $state<MyInfoDto | null>(null);
  let loadError = $state<string | null>(null);
  let draft = $state("");
  let nameError = $state<string | null>(null);
  let saving = $state(false);
  let notice = $state<string | null>(null);
  let networkStarting = $state(false);
  let themeError = $state<string | null>(null);

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

  async function pickTheme(t: Parameters<typeof chooseTheme>[0]) {
    const err = await chooseTheme(t);
    themeError = err === null ? null : describeError(err);
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

<section class="stack">
  <h2 class="visually-hidden" tabindex="-1" data-screen-focus>My Info</h2>
  <div class="row my-info-head">
    {#if info}<p>Current name: <span data-testid="display-name">{info.display_name ?? ""}</span></p>{/if}
    <ThemePicker current={$theme} onChoose={pickTheme} />
  </div>
  {#if themeError}<p role="alert" class="alert alert-danger">{themeError}</p>{/if}
  {#if loadError}
    <p role="alert" class="alert alert-danger">{loadError}</p>
  {:else if info}
    <form class="my-info-name" onsubmit={saveName}>
      <label class="field">
        <span>Display name</span>
        <input class="input" type="text" bind:value={draft} />
      </label>
      <button type="submit" class="btn btn-primary" disabled={saving}>Save</button>
      {#if nameError}<p role="alert" class="alert alert-danger">{nameError}</p>{/if}
    </form>

    <hr class="divider" />
    <h3>Connection ticket</h3>
    {#if networkStarting}<p class="muted">Network starting…</p>{/if}
    <div class="qr-frame" data-testid="qr" aria-label="Ticket QR code">{@html info.qr_svg}</div>
    <div class="my-info-center">
      <button type="button" class="btn" onclick={copyTicket}>Copy ticket</button>
    </div>
    <pre data-testid="ticket" class="mono-block muted">{info.ticket}</pre>

    <hr class="divider" />
    <h3>NodeId</h3>
    <code data-testid="node-id" class="node-id">{info.node_id}</code>
    <div>
      <button type="button" class="btn" onclick={copyNodeId}>Copy NodeId</button>
    </div>

    {#if notice}<p role="status" class="alert alert-info">{notice}</p>{/if}
  {:else}
    <p>Loading…</p>
  {/if}
</section>
