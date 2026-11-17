<script lang="ts">
  import { namingQueue, dropNaming, refreshContacts } from "../lib/contactsStore";
  import { contactsAdd, contactsRename, describeContactsError } from "../contacts";
  import NicknameForm from "./NicknameForm.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import TrustBadge from "./TrustBadge.svelte";

  const head = $derived($namingQueue[0]);
  const more = $derived($namingQueue.length - 1);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    void head?.node_id;
    error = null;
  });

  async function save(nickname: string) {
    const req = head;
    if (!req) return;
    error = null;
    busy = true;
    try {
      if (req.nickname === null) {
        try {
          await contactsAdd(req.node_id, nickname);
        } catch (e) {
          if ((e as { type?: string } | undefined)?.type !== "AlreadyExists") throw e;
        }
        await refreshContacts();
      } else if (req.nickname !== nickname) {
        await contactsRename(req.node_id, nickname);
        await refreshContacts();
      }
      dropNaming(req.node_id);
    } catch (e) {
      error = describeContactsError(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if head}
  <div class="dialog" role="dialog" aria-label="Name this contact" data-testid="naming-prompt">
    <p><TrustBadge verified={false} /> Connected. What do you want to call them?</p>
    <NodeIdLine nodeId={head.node_id} />
    {#key head.node_id}
      <NicknameForm
        initial={head.nickname ?? undefined}
        selfName={head.self_name}
        submitLabel="Save contact"
        cancelLabel="Not now"
        {busy}
        {error}
        onSubmit={save}
        onCancel={() => dropNaming(head.node_id)}
      />
    {/key}
    {#if more > 0}<p class="muted">+{more} more</p>{/if}
  </div>
{/if}
