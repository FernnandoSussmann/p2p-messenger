<script lang="ts">
  let { verified }: { verified: unknown } = $props();
</script>

{#if verified === true}
  <span
    class="badge verified"
    data-testid="badge-verified"
    title="You marked this contact as verified. The app did not check this cryptographically."
    >Verified by you</span
  >
{:else}
  <span
    class="badge unverified"
    data-testid="badge-unverified"
    title="Not verified: you have not confirmed this NodeId belongs to them.">Unverified</span
  >
{/if}

<style>
  .badge {
    display: inline-block;
    flex-shrink: 0;
    white-space: nowrap;
    border: 1px solid currentColor;
    border-radius: 999px;
    padding: 0 0.6em;
    font-size: 0.8em;
  }
  .verified {
    color: #0b6b2e;
  }
  .unverified {
    color: #8a3b00;
    background: #fff3e0;
  }
</style>
