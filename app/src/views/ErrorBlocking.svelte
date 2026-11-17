<script lang="ts">
  let { message, onRetry }: { message: string; onRetry: () => void } = $props();
</script>

<section role="alert" class="alert alert-danger stack">
  <h2>Identity unavailable</h2>
  <p>{message}</p>
  <button class="btn btn-primary" onclick={onRetry}>Retry</button>
</section>
