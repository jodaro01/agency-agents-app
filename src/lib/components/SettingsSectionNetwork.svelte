<script lang="ts">
  /**
   * SettingsSectionNetwork.svelte — Agency Agents network controls.
   * Paperclip access is opt-in and native requests are gated by Offline Mode.
   */

  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import CheckCircle from "@lucide/svelte/icons/check-circle-2";
  import XCircle from "@lucide/svelte/icons/x-circle";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";

  import {
    paperclipAgentsList,
    paperclipDisconnect,
    paperclipLoginCancel,
    paperclipLoginPoll,
    paperclipLoginStart,
    paperclipProjectsList,
    paperclipStatus,
  } from "$lib/api";
  import { errorText } from "$lib/types";
  import type {
    PaperclipAgentSummary,
    PaperclipLoginStart as PaperclipLoginChallenge,
    PaperclipProjectSummary,
    PaperclipStatus as PaperclipConnection,
  } from "$lib/types";
  import { settings } from "$lib/stores/settings.svelte";
  import SettingsSectionUpdates from "$lib/components/SettingsSectionUpdates.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";

  const offlineModeDescription = $derived(i18n.t("network.offlineDescription"));

  let paperclipConnection: PaperclipConnection | null = $state(null);
  let paperclipApiBaseUrl = $state("");
  let paperclipCompanyId = $state("");
  let paperclipChallenge: PaperclipLoginChallenge | null = $state(null);
  let paperclipAgents: PaperclipAgentSummary[] = $state([]);
  let paperclipProjects: PaperclipProjectSummary[] = $state([]);
  let paperclipBusy = $state(false);
  let paperclipError = $state("");
  let paperclipMessage = $state("");

  function pc(key: string, fallback: string): string {
    return i18n.optional(`paperclip.${key}`, fallback);
  }

  async function loadPaperclip() {
    try {
      paperclipConnection = await paperclipStatus();
      paperclipApiBaseUrl = paperclipConnection.apiBaseUrl ?? "";
      paperclipCompanyId = paperclipConnection.companyId ?? "";
      if (paperclipConnection.configured && !settings.effective.paranoidMode) {
        await refreshPaperclip();
      }
    } catch (error) {
      paperclipError = errorText(error);
    }
  }

  async function refreshPaperclip() {
    if (!paperclipConnection?.configured) return;
    if (settings.effective.paranoidMode) {
      paperclipError = pc("offline", "Offline Mode blocks Paperclip requests.");
      return;
    }
    paperclipBusy = true;
    paperclipError = "";
    try {
      const [agents, projects] = await Promise.all([
        paperclipAgentsList(),
        paperclipProjectsList(),
      ]);
      paperclipAgents = agents;
      paperclipProjects = projects;
      paperclipMessage = pc("refreshed", "Paperclip roster refreshed.");
    } catch (error) {
      paperclipAgents = [];
      paperclipProjects = [];
      paperclipError = errorText(error);
    } finally {
      paperclipBusy = false;
    }
  }

  async function openPaperclipApproval() {
    if (!paperclipChallenge) return;
    try {
      await openUrl(paperclipChallenge.approvalUrl);
    } catch (error) {
      paperclipError = errorText(error);
    }
  }

  async function pollPaperclipLogin() {
    if (!paperclipChallenge) return;
    paperclipBusy = true;
    try {
      const result = await paperclipLoginPoll();
      if (result.status === "pending") {
        setTimeout(() => void pollPaperclipLogin(), paperclipChallenge?.pollIntervalMs ?? 1000);
        return;
      }
      paperclipChallenge = null;
      if (result.status === "approved") {
        paperclipConnection = await paperclipStatus();
        paperclipMessage = pc("connected", "Connected to Paperclip.");
        await refreshPaperclip();
      } else {
        paperclipMessage = result.status === "cancelled"
          ? pc("cancelled", "Paperclip sign-in was cancelled.")
          : pc("expired", "Paperclip sign-in expired. Start again.");
      }
    } catch (error) {
      paperclipError = errorText(error);
    } finally {
      paperclipBusy = false;
    }
  }

  async function beginPaperclipLogin() {
    if (settings.effective.paranoidMode) return;
    paperclipBusy = true;
    paperclipError = "";
    paperclipMessage = "";
    try {
      paperclipChallenge = await paperclipLoginStart(paperclipApiBaseUrl, paperclipCompanyId);
      await openPaperclipApproval();
      void pollPaperclipLogin();
    } catch (error) {
      paperclipChallenge = null;
      paperclipError = errorText(error);
      paperclipBusy = false;
    }
  }

  async function cancelPaperclipLogin() {
    paperclipBusy = true;
    try {
      await paperclipLoginCancel();
      paperclipChallenge = null;
      paperclipMessage = pc("cancelled", "Paperclip sign-in was cancelled.");
    } catch (error) {
      paperclipError = errorText(error);
    } finally {
      paperclipBusy = false;
    }
  }

  async function disconnectPaperclip() {
    paperclipBusy = true;
    paperclipError = "";
    try {
      await paperclipDisconnect();
      paperclipConnection = await paperclipStatus();
      paperclipAgents = [];
      paperclipProjects = [];
      paperclipMessage = pc("disconnected", "Paperclip access was revoked and removed.");
    } catch (error) {
      paperclipError = errorText(error);
    } finally {
      paperclipBusy = false;
    }
  }

  onMount(() => {
    void loadPaperclip();
  });

  function toggleParanoid(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).checked;
    void settings.save({ paranoidMode: v });
  }

  function handleReset() {
    void settings.reset();
  }

  type PathStatus = { label: string; desc: string; allowed: boolean };
  let pathStatuses = $derived.by<PathStatus[]>(() => {
    const paranoid = settings.effective.paranoidMode;
    const paths: PathStatus[] = [
      {
        label: "github.com · codeload.github.com",
        desc: i18n.t("network.githubSourceDesc"),
        allowed: !paranoid,
      },
      {
        label: "api.github.com",
        desc: i18n.t("network.githubApiDesc"),
        allowed: !paranoid,
      },
      {
        label: "raw.githubusercontent.com · objects.githubusercontent.com",
        desc: i18n.t("network.githubAssetsDesc"),
        allowed: !paranoid,
      },
      {
        label: "agencyagents.app",
        desc: i18n.t("network.updaterDesc"),
        allowed: !paranoid,
      },
      {
        label: paperclipConnection?.apiBaseUrl ?? "Paperclip (HTTPS, opt-in)",
        desc: pc("networkDescription", "Paperclip agent and project roster, after browser approval."),
        allowed: !paranoid && Boolean(paperclipConnection?.configured),
      },
      {
        label: i18n.t("network.defaultBrowser"),
        desc: i18n.t("network.browserDesc"),
        allowed: true,
      },
    ];
    return paths;
  });
</script>

<div class="section">
  <h2>{i18n.t("network.title")}</h2>

  {#if settings.loading && !settings.data}
    <p class="lead">{i18n.t("network.loading")}</p>
  {:else if settings.corruptOnDisk}
    <div class="callout corrupt" role="alert">
      <div class="callout-head">
        <AlertTriangle size={18} />
        <strong>{i18n.t("network.corruptTitle")}</strong>
      </div>
      <p class="callout-body">{i18n.t("network.corruptBody")}</p>
      {#if settings.error}<p class="callout-error">{settings.error}</p>{/if}
      <button type="button" class="btn-danger" onclick={handleReset} disabled={settings.loading}>
        <RefreshCw size={14} /> {i18n.t("network.reset")}
      </button>
    </div>
  {:else if settings.data}
    <div class="field">
      <label class="toggle" title={offlineModeDescription}>
        <input
          type="checkbox"
          checked={settings.data.paranoidMode}
          onchange={toggleParanoid}
          disabled={settings.loading}
          aria-describedby="offline-mode-hint"
        />
        <span class="toggle-track" aria-hidden="true"></span>
        <span class="toggle-label">{i18n.t("network.offlineMode")}</span>
      </label>
      <p class="hint" id="offline-mode-hint">{offlineModeDescription}</p>
      {#if settings.data.paranoidMode}
        <div class="callout warn" role="status">
          <AlertTriangle size={16} />
          <span>{i18n.t("network.offlineOn")}</span>
        </div>
      {/if}
    </div>

    <div class="field disclosure">
      <span class="field-label">{i18n.t("network.whereConnects")}</span>
      <ol class="paths">
        {#each pathStatuses as p, i (p.label)}
          <li>
            <span class="num">{i + 1}.</span>
            <span class="status" aria-label={p.allowed ? "allowed" : "blocked"}>
              {#if p.allowed}<CheckCircle size={14} class="ok" />{:else}<XCircle size={14} class="bad" />{/if}
            </span>
            <div>
              <code class="path-label">{p.label}</code>
              <p class="path-desc">{p.desc}</p>
            </div>
          </li>
        {/each}
      </ol>
      <p class="hint">{i18n.t("network.everyCall")}</p>
    </div>

    <section class="paperclip-card" aria-labelledby="paperclip-title">
      <div class="paperclip-heading">
        <div>
          <h3 id="paperclip-title">{pc("title", "Paperclip")}</h3>
          <p class="hint">{pc("intro", "Connect to the Droplet Paperclip instance and inspect its agents and active projects.")}</p>
        </div>
        {#if paperclipConnection?.configured}
          <span class="connected">{pc("connectedBadge", "Connected")}</span>
        {/if}
      </div>

      {#if !paperclipConnection?.configured}
        <label class="paperclip-label">
          {pc("serverLabel", "Paperclip HTTPS URL")}
          <input
            type="url"
            bind:value={paperclipApiBaseUrl}
            placeholder="https://pclip.co"
            autocomplete="url"
            spellcheck="false"
            disabled={paperclipBusy || Boolean(paperclipChallenge) || settings.data.paranoidMode}
          />
        </label>
        <label class="paperclip-label">
          {pc("companyLabel", "Paperclip company UUID")}
          <input
            type="text"
            bind:value={paperclipCompanyId}
            placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
            autocomplete="off"
            spellcheck="false"
            disabled={paperclipBusy || Boolean(paperclipChallenge) || settings.data.paranoidMode}
          />
        </label>
        <p class="hint">{pc("approvalHint", "Paperclip opens a browser approval. The company ID scopes the requested access; the API token is stored in macOS Keychain.")}</p>
        <button
          type="button"
          class="btn"
          onclick={beginPaperclipLogin}
          disabled={paperclipBusy || Boolean(paperclipChallenge) || settings.data.paranoidMode || !paperclipApiBaseUrl.trim() || !paperclipCompanyId.trim()}
        >
          {paperclipBusy ? pc("waiting", "Waiting for approval…") : pc("connect", "Connect to Paperclip")}
        </button>
      {/if}

      {#if paperclipChallenge}
        <div class="paperclip-actions">
          <p class="hint">{pc("approvalPending", "Approve the request in your browser. This window checks for approval automatically.")}</p>
          <button type="button" class="btn-secondary" onclick={openPaperclipApproval}>
            {pc("openApproval", "Open approval page")}
          </button>
          <button type="button" class="btn-secondary" onclick={pollPaperclipLogin} disabled={paperclipBusy || settings.data.paranoidMode}>
            {pc("checkApproval", "Check approval")}
          </button>
          <button type="button" class="btn-secondary" onclick={cancelPaperclipLogin} disabled={paperclipBusy || settings.data.paranoidMode}>
            {pc("cancel", "Cancel sign-in")}
          </button>
        </div>
      {/if}

      {#if paperclipConnection?.configured}
        <div class="paperclip-actions">
          <button type="button" class="btn-secondary" onclick={refreshPaperclip} disabled={paperclipBusy || settings.data.paranoidMode}>
            <RefreshCw size={14} /> {pc("refresh", "Refresh roster")}
          </button>
          <button type="button" class="btn-secondary" onclick={disconnectPaperclip} disabled={paperclipBusy || settings.data.paranoidMode}>
            {pc("disconnect", "Revoke and disconnect")}
          </button>
        </div>

        <div class="paperclip-columns">
          <div>
            <h4>{pc("agents", "Agents")} ({paperclipAgents.length})</h4>
            {#if paperclipAgents.length}
              <ul class="paperclip-list">
                {#each paperclipAgents as agent (agent.id)}
                  <li>
                    <strong>{agent.name}</strong>
                    <span>{[agent.title, agent.adapterType, agent.status].filter(Boolean).join(" · ")}</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="hint">{pc("noAgents", "No agents loaded yet.")}</p>
            {/if}
          </div>
          <div>
            <h4>{pc("projects", "Active projects")} ({paperclipProjects.length})</h4>
            {#if paperclipProjects.length}
              <ul class="paperclip-list">
                {#each paperclipProjects as project (project.id)}
                  <li>
                    <strong>{project.name}</strong>
                    <span>{project.status ?? project.urlKey ?? ""}</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="hint">{pc("noProjects", "No active projects loaded yet.")}</p>
            {/if}
          </div>
        </div>
      {/if}

      {#if paperclipMessage}<p class="hint" role="status">{paperclipMessage}</p>{/if}
      {#if paperclipError}<p class="callout-error" role="alert">{paperclipError}</p>{/if}
      <p class="hint">{pc("readOnlyNote", "This first native bridge reads agent and project rosters. It does not edit live Paperclip configurations.")}</p>
    </section>

    {#if settings.error}<p class="callout-error">{settings.error}</p>{/if}

    <SettingsSectionUpdates />
  {/if}
</div>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-5); max-width: 680px; }
  h2 { font-size: var(--text-h1); font-weight: var(--fw-semibold); color: var(--color-text-primary); margin-bottom: var(--space-2); }
  h3 { font-size: var(--text-h1); margin: 0; color: var(--color-text-primary); }
  h4 { font-size: var(--text-body); margin: 0 0 var(--space-2); color: var(--color-text-primary); }
  .lead { font-size: var(--text-body); color: var(--color-text-secondary); line-height: var(--lh-normal); }
  .field { display: flex; flex-direction: column; gap: var(--space-2); }
  .field-label { font-size: var(--text-body); font-weight: var(--fw-medium); color: var(--color-text-primary); }
  .hint { font-size: var(--text-body-sm); color: var(--color-text-muted); line-height: var(--lh-snug); }
  .paperclip-card { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); }
  .paperclip-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); }
  .paperclip-heading p { margin-top: var(--space-1); }
  .paperclip-label { display: flex; flex-direction: column; gap: var(--space-1); color: var(--color-text-primary); font-size: var(--text-body-sm); font-weight: var(--fw-medium); }
  .paperclip-label input { min-width: 0; padding: 8px 10px; color: var(--color-text-primary); background: var(--color-surface-sunken); border: 1px solid var(--color-border); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: var(--text-mono); }
  .paperclip-actions { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .paperclip-columns { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--space-4); }
  .paperclip-list { max-height: 240px; overflow: auto; display: flex; flex-direction: column; gap: 6px; padding: 0; margin: 0; list-style: none; }
  .paperclip-list li { display: flex; flex-direction: column; gap: 2px; padding: 6px 8px; border-radius: var(--radius-sm); background: var(--color-surface-sunken); }
  .paperclip-list li strong { color: var(--color-text-primary); font-size: var(--text-body-sm); }
  .paperclip-list li span { color: var(--color-text-muted); font-size: var(--text-body-sm); overflow-wrap: anywhere; }
  .connected { color: #58a55c; font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .toggle { display: inline-flex; align-items: center; gap: var(--space-2); cursor: pointer; user-select: none; }
  .toggle input { position: absolute; opacity: 0; pointer-events: none; }
  .toggle-track {
    width: 36px; height: 20px; background: var(--color-surface-sunken);
    border: 1px solid var(--color-border); border-radius: 999px; position: relative;
    transition: background-color var(--motion-duration-fast) var(--motion-ease-out);
  }
  .toggle-track::after {
    content: ""; position: absolute; top: 1px; left: 1px; width: 16px; height: 16px;
    background: var(--color-surface-raised); border-radius: 50%; box-shadow: var(--shadow-xs);
    transition: transform var(--motion-duration-fast) var(--motion-ease-out);
  }
  .toggle input:checked + .toggle-track { background: var(--color-brand); border-color: var(--color-brand); }
  .toggle input:checked + .toggle-track::after { transform: translateX(16px); background: white; }
  .toggle-label { font-size: var(--text-body); font-weight: var(--fw-medium); color: var(--color-text-primary); }
  .callout { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border-radius: var(--radius-md); border: 1px solid var(--color-border); }
  .callout-head { display: inline-flex; align-items: center; gap: var(--space-2); color: var(--color-text-primary); font-size: var(--text-body); }
  .callout-body { font-size: var(--text-body-sm); color: var(--color-text-secondary); line-height: var(--lh-snug); }
  .callout-error { font-family: var(--font-mono); font-size: var(--text-mono); color: var(--color-text-muted); word-break: break-word; margin-top: var(--space-1); }
  .corrupt { background: color-mix(in srgb, var(--color-danger) 8%, var(--color-surface-sunken)); border-color: color-mix(in srgb, var(--color-danger) 35%, var(--color-border)); }
  .warn { flex-direction: row; align-items: center; background: var(--color-surface-sunken); font-size: var(--text-body-sm); color: var(--color-text-secondary); }
  .btn { width: max-content; display: inline-flex; align-items: center; gap: 6px; padding: 7px 12px; border-radius: var(--radius-md); background: var(--color-brand); color: white; font-size: var(--text-body-sm); font-weight: var(--fw-medium); cursor: pointer; }
  .btn-secondary { width: max-content; display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); color: var(--color-text-primary); font-size: var(--text-body-sm); cursor: pointer; }
  .btn:disabled, .btn-secondary:disabled { opacity: .55; cursor: not-allowed; }
  .btn-danger {
    display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: var(--radius-md);
    background: var(--color-danger); color: white; font-size: var(--text-body-sm); font-weight: var(--fw-medium); cursor: pointer; width: max-content;
  }
  .btn-danger:disabled { opacity: 0.6; cursor: not-allowed; }
  .btn-danger:hover:not(:disabled) { filter: brightness(1.05); }
  .disclosure { gap: var(--space-3); }
  .paths { list-style: none; display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); background: var(--color-surface-sunken); border: 1px solid var(--color-border); border-radius: var(--radius-md); margin: 0; }
  .paths li { display: grid; grid-template-columns: 22px 18px 1fr; gap: var(--space-2); align-items: start; }
  .num { font-variant-numeric: tabular-nums; color: var(--color-text-muted); font-size: var(--text-body-sm); padding-top: 2px; }
  .status { display: inline-flex; align-items: center; padding-top: 2px; }
  .status :global(.ok) { color: #58a55c; }
  .status :global(.bad) { color: #d24a4a; }
  .path-label { font-family: var(--font-mono); font-size: var(--text-mono); color: var(--color-text-primary); background: var(--color-surface-raised); padding: 1px 6px; border-radius: var(--radius-sm); border: 1px solid var(--color-border); }
  .path-desc { margin-top: 4px; font-size: var(--text-body-sm); color: var(--color-text-secondary); line-height: var(--lh-snug); }
</style>
