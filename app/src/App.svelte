<script lang="ts">
  import { onMount } from "svelte";
  import { describeError, identityStatus } from "./identity";
  import { appView, wizardView as Wizard } from "./lib/router";
  import ErrorBlocking from "./views/ErrorBlocking.svelte";
  import MainScreen from "./views/MainScreen.svelte";

  async function refresh() {
    appView.set({ kind: "loading" });
    try {
      const status = await identityStatus();
      if (status.type === "Error") {
        appView.set({ kind: "blocked", message: status.reason });
      } else if (status.type === "Absent") {
        appView.set({ kind: "wizard" });
      } else {
        appView.set({ kind: "main" });
      }
    } catch (err) {
      appView.set({ kind: "blocked", message: describeError(err) });
    }
  }

  onMount(refresh);
</script>

<main>
  {#if $appView.kind === "loading"}
    <p>Loading...</p>
  {:else if $appView.kind === "blocked"}
    <ErrorBlocking message={$appView.message} onRetry={refresh} />
  {:else if $appView.kind === "wizard"}
    <Wizard onDone={() => appView.set({ kind: "main" })} />
  {:else}
    <MainScreen />
  {/if}
</main>

<style>
  main {
    font-family: sans-serif;
    margin: 2rem;
  }
</style>
