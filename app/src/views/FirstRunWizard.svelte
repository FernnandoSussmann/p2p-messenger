<script lang="ts">
  // Contract: `onDone` must be called once identity is provisioned and the
  // display name is set; the app then navigates to the main screen.
  let { onDone }: { onDone: () => void } = $props();
</script>

<section>
  <h2>Welcome</h2>
  <button onclick={onDone}>Continue</button>
</section>
