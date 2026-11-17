<script lang="ts">
  import { onMount } from "svelte";
  import { describeError, identityStatus } from "./identity";
  import { appView, resetScreen } from "./lib/router";
  import { loadTheme } from "./lib/theme";
  import { storageStatus } from "./storage";
  import FirstRunWizard from "./views/FirstRunWizard.svelte";
  import ErrorBlocking from "./views/ErrorBlocking.svelte";
  import MainScreen from "./views/MainScreen.svelte";
  import StorageUnavailable from "./views/StorageUnavailable.svelte";

  async function refresh() {
    appView.set({ kind: "loading" });
    try {
      const status = await identityStatus();
      if (status.type === "Error") {
        appView.set({ kind: "blocked", message: status.reason });
      } else if (status.type === "Absent") {
        appView.set({ kind: "wizard" });
      } else {
        // Present always goes to main, even with no stored name (crash between
        // provision and naming): My Info is where the name is then set. No
        // extra blocking screen, and the wizard is never re-shown. An
        // unavailable encrypted store blocks everything until it is retried.
        const storage = await storageStatus();
        if (storage.type === "Error") {
          appView.set({ kind: "storage", message: storage.reason });
        } else {
          await loadTheme();
          resetScreen();
          appView.set({ kind: "main" });
        }
      }
    } catch (err) {
      appView.set({ kind: "blocked", message: describeError(err) });
    }
  }

  onMount(refresh);
</script>

<main class="app-column">
  {#if $appView.kind === "loading"}
    <p>Loading...</p>
  {:else if $appView.kind === "blocked"}
    <ErrorBlocking message={$appView.message} onRetry={refresh} />
  {:else if $appView.kind === "storage"}
    <StorageUnavailable message={$appView.message} onResolved={refresh} />
  {:else if $appView.kind === "wizard"}
    <FirstRunWizard onDone={refresh} />
  {:else}
    <MainScreen />
  {/if}
</main>
