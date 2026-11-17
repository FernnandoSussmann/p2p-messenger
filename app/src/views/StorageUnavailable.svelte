<script lang="ts">
  import { storageRetry } from "../storage";

  let { message, onResolved }: { message: string; onResolved: () => void } = $props();
  let busy = $state(false);
  let latest = $state<string | null>(null);
  const current = $derived(latest ?? message);

  async function retry() {
    busy = true;
    try {
      const s = await storageRetry();
      if (s.type === "Error") latest = s.reason;
      else onResolved();
    } finally {
      busy = false;
    }
  }
</script>

<section role="alert" class="alert alert-danger stack" aria-labelledby="storage-title">
  <h2 id="storage-title">Encrypted storage unavailable</h2>
  <p>{current}</p>
  <p>Your messages, contacts and settings were not deleted, reset or replaced.</p>
  <button type="button" class="btn btn-primary" disabled={busy} onclick={retry}>Retry</button>
</section>
