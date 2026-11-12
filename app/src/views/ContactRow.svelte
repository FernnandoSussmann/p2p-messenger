<script lang="ts">
  import type { ContactDto, RemoveContactDto } from "../contacts";
  import { contactsRename, contactsSetVerified, describeContactsError } from "../contacts";
  import { refreshContacts } from "../lib/contactsStore";
  import { lanPeers, connections } from "../lib/netStore";
  import { openChat, unread } from "../lib/chatStore";
  import { netConnectTicket, describeNetError } from "../net";
  import TrustBadge from "./TrustBadge.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import NicknameForm from "./NicknameForm.svelte";
  import VerifyContactDialog from "./VerifyContactDialog.svelte";
  import RemoveContactDialog from "./RemoveContactDialog.svelte";

  interface Props {
    contact: ContactDto;
    onNotice: (msg: string) => void;
  }
  let { contact, onNotice }: Props = $props();

  let renaming = $state(false);
  let verifying = $state(false);
  let removing = $state(false);
  let busy = $state(false);
  let renameError = $state<string | null>(null);
  let error = $state<string | null>(null);

  const lanTicket = $derived($lanPeers.find((p) => p.node_id === contact.node_id)?.ticket);
  const isConnected = $derived($connections[contact.node_id]?.state === "connected");

  async function connect() {
    error = null;
    busy = true;
    try {
      await netConnectTicket(lanTicket!);
    } catch (e) {
      error = describeNetError(e);
    } finally {
      busy = false;
    }
  }

  async function rename(nickname: string) {
    renameError = null;
    busy = true;
    try {
      await contactsRename(contact.node_id, nickname);
      await refreshContacts();
      renaming = false;
    } catch (e) {
      renameError = describeContactsError(e);
    } finally {
      busy = false;
    }
  }

  async function unverify() {
    error = null;
    try {
      await contactsSetVerified(contact.node_id, false);
      await refreshContacts();
    } catch (e) {
      error = describeContactsError(e);
    }
  }

  function removed(r: RemoveContactDto) {
    removing = false;
    const kept = r.history_erased ? "was deleted" : "was kept";
    onNotice(`Removed “${contact.nickname}”. Your message history with them ${kept}.`);
  }
</script>

<!-- Only the local nickname is shown here (user decision). contact.self_name is
     intentionally not rendered; it belongs to the future contact-info screen. -->
<li data-testid="contact-row" class="row">
  <div class="title">
    <TrustBadge verified={contact.verified} />
    <strong class="nickname" data-testid="contact-nickname"><bdi>{contact.nickname}</bdi></strong>
  </div>
  <NodeIdLine nodeId={contact.node_id} />
  <div class="actions">
    <button type="button" onclick={() => openChat(contact.node_id)}
      >{($unread[contact.node_id] ?? 0) > 0 ? `Message (${$unread[contact.node_id]})` : "Message"}</button
    >
    {#if lanTicket && !isConnected}
      <button type="button" disabled={busy} onclick={connect}>Connect</button>
    {/if}
    <button type="button" onclick={() => { renameError = null; renaming = true; }}>Rename</button>
    {#if contact.verified === true}
      <button type="button" onclick={unverify}>Mark unverified</button>
    {:else}
      <button type="button" onclick={() => (verifying = true)}>Mark verified…</button>
    {/if}
    <button type="button" onclick={() => (removing = true)}>Remove…</button>
  </div>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if renaming}
    <NicknameForm
      initial={contact.nickname}
      selfName={contact.self_name}
      submitLabel="Save nickname"
      {busy}
      error={renameError}
      onSubmit={rename}
      onCancel={() => (renaming = false)}
    />
  {/if}
  {#if verifying}
    <VerifyContactDialog {contact} onDone={() => (verifying = false)} onCancel={() => (verifying = false)} />
  {/if}
  {#if removing}
    <RemoveContactDialog {contact} onDone={removed} onCancel={() => (removing = false)} />
  {/if}
</li>

<style>
  .row {
    margin-bottom: 1em;
  }
  .title {
    display: flex;
    gap: 0.5em;
    min-width: 0;
  }
  .nickname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
</style>
