<script lang="ts">
  import { onMount } from "svelte";
  import {
    describeError,
    identityMyInfo,
    identityProvision,
    identityStatus,
    type MyInfoDto,
  } from "./identity";

  type View =
    | { kind: "loading" }
    | { kind: "blocked"; message: string }
    | { kind: "absent" }
    | { kind: "present"; info: MyInfoDto };

  let view: View = $state({ kind: "loading" });
  let error = $state("");

  async function refresh() {
    try {
      const status = await identityStatus();
      if (status.type === "Error") {
        view = { kind: "blocked", message: status.reason };
      } else if (status.type === "Absent") {
        view = { kind: "absent" };
      } else {
        view = { kind: "present", info: await identityMyInfo() };
      }
    } catch (err) {
      view = { kind: "blocked", message: describeError(err) };
    }
  }

  async function create() {
    error = "";
    try {
      await identityProvision();
      await refresh();
    } catch (err) {
      error = describeError(err);
    }
  }

  onMount(refresh);
</script>

<main>
  <h1>p2p-messenger</h1>
  {#if view.kind === "loading"}
    <p>Loading...</p>
  {:else if view.kind === "blocked"}
    <p role="alert">Identity unavailable: {view.message}</p>
    <p>Resolve the problem and restart the app. No identity will be created.</p>
  {:else if view.kind === "absent"}
    <button onclick={create}>Create identity</button>
    {#if error}<p role="alert">{error}</p>{/if}
  {:else}
    <p>Name: {view.info.display_name ?? "(not set)"}</p>
    <p>NodeId: <code>{view.info.node_id}</code></p>
  {/if}
</main>

<style>
  main {
    font-family: sans-serif;
    margin: 2rem;
  }
  code {
    word-break: break-all;
  }
</style>
