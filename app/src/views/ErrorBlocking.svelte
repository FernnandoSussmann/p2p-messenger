<script lang="ts">
  let { message, onRetry }: { message: string; onRetry: () => void } = $props();
</script>

<section role="alert">
  <h2>Identity unavailable</h2>
  <p>{message}</p>
  <button onclick={onRetry}>Retry</button>
</section>
