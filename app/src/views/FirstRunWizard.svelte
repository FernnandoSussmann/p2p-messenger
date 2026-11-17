<script lang="ts">
  import { get } from "svelte/store";
  import {
    describeError,
    identityProvision,
    identitySetDisplayName,
    identityThemeSet,
    type IdentityCmdError,
  } from "../identity";
  import { displayNameMessages } from "../lib/displayNameMessages";
  import { applyTheme, pendingTheme, type Theme } from "../lib/theme";
  import ThemePicker from "./ThemePicker.svelte";

  let { onDone }: { onDone: () => void } = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);
  let name = $state("");
  let provisioned = false;

  function choose(t: Theme) {
    pendingTheme.set(t);
    applyTheme(t);
  }

  async function submit(ev: SubmitEvent) {
    ev.preventDefault();
    if (busy) return;
    if (name.trim() === "") {
      error = displayNameMessages.Empty;
      return;
    }
    busy = true;
    error = null;
    try {
      if (!provisioned) {
        try {
          await identityProvision();
          provisioned = true;
        } catch (e) {
          if ((e as IdentityCmdError | undefined)?.type === "AlreadyProvisioned") {
            pendingTheme.set(null);
            onDone();
          } else {
            error = describeError(e);
          }
          return;
        }
      }
      try {
        await identitySetDisplayName(name);
      } catch (e) {
        const err = e as IdentityCmdError | undefined;
        error =
          err?.type === "InvalidDisplayName" ? displayNameMessages[err.kind] : describeError(e);
        return;
      }
      try {
        await identityThemeSet(get(pendingTheme) ?? "midnight");
        pendingTheme.set(null);
      } catch {
        // cosmetic preference: never blocks entry; pendingTheme keeps applying
      }
      onDone();
    } finally {
      busy = false;
    }
  }
</script>

<section class="stack setup">
  <h2>Welcome</h2>
  <p>Create your identity on this device.</p>
  <p>Choose a theme</p>
  <ThemePicker current={$pendingTheme ?? "midnight"} onChoose={choose} disabled={busy} />
  <form class="stack" onsubmit={submit}>
    <label class="field">
      Display name
      <input class="input" type="text" bind:value={name} disabled={busy} />
    </label>
    {#if name.trim() !== ""}
      <button type="submit" aria-label="Continue" class="btn btn-primary btn-icon setup-continue" disabled={busy}>
        <svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M13 6l6 6-6 6" /></svg>
      </button>
    {/if}
  </form>
  {#if error}
    <p role="alert" class="alert alert-danger">{error}</p>
  {/if}
</section>
