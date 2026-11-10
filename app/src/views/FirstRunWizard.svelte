<script lang="ts">
  import {
    describeError,
    identityProvision,
    identitySetDisplayName,
    type IdentityCmdError,
  } from "../identity";
  import { displayNameMessages } from "../lib/displayNameMessages";

  let { onDone }: { onDone: () => void } = $props();

  let step = $state<"generate" | "name" | "land">("generate");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let name = $state("");

  async function generate() {
    busy = true;
    error = null;
    try {
      await identityProvision();
      step = "name";
    } catch (e) {
      if ((e as IdentityCmdError | undefined)?.type === "AlreadyProvisioned") {
        onDone();
      } else {
        error = describeError(e);
      }
    } finally {
      busy = false;
    }
  }

  async function submitName(ev: SubmitEvent) {
    ev.preventDefault();
    if (busy) return;
    if (name.trim() === "") {
      error = displayNameMessages.Empty;
      return;
    }
    busy = true;
    error = null;
    try {
      await identitySetDisplayName(name);
      step = "land";
      onDone();
    } catch (e) {
      const err = e as IdentityCmdError | undefined;
      error = err?.type === "InvalidDisplayName" ? displayNameMessages[err.kind] : describeError(e);
    } finally {
      busy = false;
    }
  }
</script>

<section>
  {#if step === "generate"}
    <h2>Welcome</h2>
    <p>Create your identity on this device.</p>
    <button onclick={generate} disabled={busy}>Create identity</button>
  {:else if step === "name"}
    <h2>Choose a display name</h2>
    <form onsubmit={submitName}>
      <label>
        Display name
        <input type="text" bind:value={name} disabled={busy} />
      </label>
      <button type="submit" disabled={busy}>Continue</button>
    </form>
  {:else}
    <h2>All set</h2>
  {/if}
  {#if error}
    <p role="alert">{error}</p>
  {/if}
</section>
