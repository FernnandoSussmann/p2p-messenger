<script lang="ts">
  let { verified }: { verified: unknown } = $props();
</script>

{#if verified === true}
  <span
    class="badge badge-success"
    data-testid="badge-verified"
    title="You marked this contact as verified. The app did not check this cryptographically."
    >Verified by you</span
  >
{:else}
  <span
    class="badge badge-warning"
    data-testid="badge-unverified"
    title="Not verified: you have not confirmed this NodeId belongs to them.">Unverified</span
  >
{/if}
