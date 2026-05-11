const tauriCore = window.__TAURI__?.core;
const invoke = tauriCore?.invoke
  ? (cmd, args) => tauriCore.invoke(cmd, args)
  : async () => {
      throw new Error("Tauri runtime is not available.");
    };

const state = {
  rangeMinutes: 60,
  logCursor: 0,
  logEntries: [],
  logsPaused: false,
  autoScroll: true,
  logLevels: {
    INFO: true,
    WARN: true,
    ERROR: true,
    DEBUG: false,
  },
  logSearch: "",
  shimText: "",
  configPath: "",
  inspection: null,
  preview: null,
  runtime: null,
  doctorReport: null,
  doctorLoading: false,
  catalogModels: [],
  selectedModelSlug: null,
  baseTomlDirty: false,
  savedShimText: "",
  previewTimer: null,
  previewRequestId: 0,
  lastAppliedPreviewId: 0,
  runtimeRefreshing: false,
  runtimeActionPending: false,
  logRefreshing: false,
  applyPending: false,
  starterConfigText: "",
  defaultConfigExists: false,
  targetScope: "project",
  browseSupported: true,
  browseMessage: "",
};

const els = {
  titleStatus: document.getElementById("title-status"),
  statusChip: document.getElementById("status-chip"),
  statusLabel: document.getElementById("status-label"),
  listenLabel: document.getElementById("listen-label"),
  runtimeProvider: document.getElementById("runtime-provider"),
  runtimeModel: document.getElementById("runtime-model"),
  runtimeEndpoint: document.getElementById("runtime-endpoint"),
  runtimeBackend: document.getElementById("runtime-backend"),
  runtimeUpstream: document.getElementById("runtime-upstream"),
  runtimeListen: document.getElementById("runtime-listen"),
  requestCount: document.getElementById("request-count"),
  completedCount: document.getElementById("completed-count"),
  storeCount: document.getElementById("store-count"),
  errorCount: document.getElementById("error-count"),
  runtimeUptime: document.getElementById("runtime-uptime"),
  lastErrorField: document.getElementById("last-error-field"),
  chart: document.getElementById("token-chart"),
  chartEmpty: document.getElementById("chart-empty"),
  chartWindowLabel: document.getElementById("chart-window-label"),
  chartLastSample: document.getElementById("chart-last-sample"),
  partialBadge: document.getElementById("partial-usage-badge"),
  logSearch: document.getElementById("log-search"),
  pauseLogsBtn: document.getElementById("pause-logs-btn"),
  exportLogsBtn: document.getElementById("export-logs-btn"),
  logStream: document.getElementById("log-stream"),
  autoScrollToggle: document.getElementById("auto-scroll-toggle"),
  logCount: document.getElementById("log-count"),
  statusMessage: document.getElementById("status-message"),
  integrationStatus: document.getElementById("integration-status"),
  activeConfigPath: document.getElementById("active-config-path"),
  startBtn: document.getElementById("start-btn"),
  stopBtn: document.getElementById("stop-btn"),
  restartBtn: document.getElementById("restart-btn"),
  settingsBtn: document.getElementById("settings-btn"),
  doctorBtn: document.getElementById("doctor-btn"),
  settingsShell: document.getElementById("settings-shell"),
  doctorShell: document.getElementById("doctor-shell"),
  confirmShell: document.getElementById("confirm-shell"),
  settingsCloseTitleBtn: document.getElementById("settings-close-title-btn"),
  settingsCloseBtn: document.getElementById("settings-close-btn"),
  doctorCloseTitleBtn: document.getElementById("doctor-close-title-btn"),
  doctorCloseBtn: document.getElementById("doctor-close-btn"),
  confirmCloseBtn: document.getElementById("confirm-close-btn"),
  confirmTitle: document.getElementById("confirm-title"),
  confirmMessage: document.getElementById("confirm-message"),
  confirmDetail: document.getElementById("confirm-detail"),
  confirmCancelBtn: document.getElementById("confirm-cancel-btn"),
  confirmOkBtn: document.getElementById("confirm-ok-btn"),
  configPath: document.getElementById("config-path"),
  browseConfigBtn: document.getElementById("browse-config-btn"),
  loadConfigBtn: document.getElementById("load-config-btn"),
  saveConfigBtn: document.getElementById("save-config-btn"),
  projectDir: document.getElementById("project-dir"),
  browseProjectDirBtn: document.getElementById("browse-project-dir-btn"),
  codexHome: document.getElementById("codex-home"),
  browseCodexHomeBtn: document.getElementById("browse-codex-home-btn"),
  envKey: document.getElementById("env-key"),
  trustProject: document.getElementById("trust-project"),
  targetScopeProject: document.getElementById("target-scope-project"),
  targetScopeUser: document.getElementById("target-scope-user"),
  generalProvider: document.getElementById("general-provider"),
  generalModel: document.getElementById("general-model"),
  generalEndpoint: document.getElementById("general-endpoint"),
  generalBackend: document.getElementById("general-backend"),
  generalListen: document.getElementById("general-listen"),
  generalUpstream: document.getElementById("general-upstream"),
  generalWriteScope: document.getElementById("general-write-scope"),
  generalApplyState: document.getElementById("general-apply-state"),
  surfaceWebSearchCapability: document.getElementById("surface-web-search-capability"),
  surfaceSearchCapability: document.getElementById("surface-search-capability"),
  surfaceParallelCapability: document.getElementById("surface-parallel-capability"),
  surfaceSummaryCapability: document.getElementById("surface-summary-capability"),
  surfaceImageCapability: document.getElementById("surface-image-capability"),
  surfacePatchCapability: document.getElementById("surface-patch-capability"),
  surfaceReasoningCapability: document.getElementById("surface-reasoning-capability"),
  shimEditor: document.getElementById("shim-editor"),
  tomlTargetPath: document.getElementById("toml-target-path"),
  catalogTargetPath: document.getElementById("catalog-target-path"),
  integrationTomlTarget: document.getElementById("integration-toml-target"),
  integrationCatalogTarget: document.getElementById("integration-catalog-target"),
  tomlBaseEditor: document.getElementById("toml-base-editor"),
  mergedPreview: document.getElementById("merged-preview"),
  integrationSummary: document.getElementById("integration-summary"),
  applyBtn: document.getElementById("apply-btn"),
  settingsTabButtons: Array.from(document.querySelectorAll("[data-settings-tab]")),
  settingsPanels: Array.from(document.querySelectorAll("[data-settings-panel]")),
  rangeButtons: Array.from(document.querySelectorAll(".range-btn")),
  logLevelInputs: Array.from(document.querySelectorAll("input[data-level]")),
  modelsCount: document.getElementById("models-count"),
  modelsDefaultLabel: document.getElementById("models-default-label"),
  modelsTableBody: document.getElementById("models-table-body"),
  modelDetailTitle: document.getElementById("model-detail-title"),
  modelDetailList: document.getElementById("model-detail-list"),
  modelDetailJson: document.getElementById("model-detail-json"),
  doctorSummary: document.getElementById("doctor-summary"),
  doctorProgress: document.getElementById("doctor-progress"),
  doctorTableBody: document.getElementById("doctor-table-body"),
  rerunDoctorBtn: document.getElementById("rerun-doctor-btn"),
};

let confirmResolver = null;

async function bootstrap() {
  bindEvents();

  try {
    const defaults = await invoke("get_defaults");
    state.starterConfigText = defaults.starter_config_text || "";
    state.defaultConfigExists = Boolean(defaults.default_config_exists);
    state.browseSupported = defaults.browse_supported !== false;
    state.browseMessage = defaults.browse_message || "";

    if (defaults.default_config_path) {
      state.configPath = defaults.default_config_path;
      els.configPath.value = defaults.default_config_path;
      els.activeConfigPath.textContent = defaults.default_config_path;
    }
    if (defaults.default_codex_home) {
      els.codexHome.value = defaults.default_codex_home;
    }
    if (defaults.current_directory) {
      els.projectDir.value = defaults.current_directory;
    }

    state.targetScope = blankToNull(els.projectDir.value) ? "project" : "user";
    renderTargetScope();
    updateBrowseAvailability();

    if (state.configPath) {
      if (state.defaultConfigExists) {
        await loadConfig(state.configPath, false);
      } else {
        await loadStarterDocument(
          state.configPath,
          "No default shim config exists yet. Starter template loaded; save YAML to create it.",
        );
      }
    }
  } catch (error) {
    showMessage(`Defaults unavailable: ${error.message ?? error}`, true);
  }

  await refreshRuntime();
  await refreshLogs();
  window.setInterval(refreshRuntime, 1000);
  window.setInterval(refreshLogs, 700);
  updateControls();
}

function bindEvents() {
  els.startBtn.addEventListener("click", () => startRuntime(false));
  els.restartBtn.addEventListener("click", () => startRuntime(true));
  els.stopBtn.addEventListener("click", stopRuntime);

  els.settingsBtn.addEventListener("click", () => openDialog("settings"));
  els.doctorBtn.addEventListener("click", async () => {
    openDialog("doctor");
    await runDoctor();
  });

  els.settingsCloseTitleBtn.addEventListener("click", closeDialogs);
  els.settingsCloseBtn.addEventListener("click", closeDialogs);
  els.doctorCloseTitleBtn.addEventListener("click", closeDialogs);
  els.doctorCloseBtn.addEventListener("click", closeDialogs);
  els.confirmCloseBtn.addEventListener("click", () => resolveConfirm(false));
  els.confirmCancelBtn.addEventListener("click", () => resolveConfirm(false));
  els.confirmOkBtn.addEventListener("click", () => resolveConfirm(true));

  [els.settingsShell, els.doctorShell, els.confirmShell].forEach((shell) => {
    shell.addEventListener("click", (event) => {
      if (event.target !== shell) {
        return;
      }
      if (shell === els.confirmShell) {
        resolveConfirm(false);
      } else {
        closeDialogs();
      }
    });
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      if (!els.confirmShell.classList.contains("hidden")) {
        resolveConfirm(false);
        return;
      }
      closeDialogs();
    }
  });

  els.rangeButtons.forEach((button) => {
    button.addEventListener("click", async () => {
      state.rangeMinutes = Number(button.dataset.range);
      els.rangeButtons.forEach((node) => node.classList.toggle("active", node === button));
      await refreshRuntime();
    });
  });

  els.logLevelInputs.forEach((input) => {
    input.addEventListener("change", () => {
      state.logLevels[input.dataset.level] = input.checked;
      renderLogs();
    });
  });

  els.logSearch.addEventListener("input", () => {
    state.logSearch = els.logSearch.value.trim().toLowerCase();
    renderLogs();
  });

  els.pauseLogsBtn.addEventListener("click", () => {
    state.logsPaused = !state.logsPaused;
    els.pauseLogsBtn.textContent = state.logsPaused ? "Resume Stream" : "Pause Stream";
  });

  els.autoScrollToggle.addEventListener("change", () => {
    state.autoScroll = els.autoScrollToggle.checked;
    if (state.autoScroll) {
      scrollLogsToBottom();
    }
  });

  els.logStream.addEventListener("scroll", () => {
    if (!isNearBottom(els.logStream)) {
      state.autoScroll = false;
      els.autoScrollToggle.checked = false;
    } else if (!state.logsPaused) {
      state.autoScroll = true;
      els.autoScrollToggle.checked = true;
    }
  });

  els.exportLogsBtn.addEventListener("click", exportLogs);

  els.browseConfigBtn.addEventListener("click", async () => {
    await browseIntoField("config_file", els.configPath);
  });
  els.browseProjectDirBtn.addEventListener("click", async () => {
    const changed = await browseIntoField("directory", els.projectDir);
    if (changed) {
      handleTargetPathChange();
    }
  });
  els.browseCodexHomeBtn.addEventListener("click", async () => {
    const changed = await browseIntoField("directory", els.codexHome);
    if (changed) {
      handleTargetPathChange();
    }
  });

  els.loadConfigBtn.addEventListener("click", async () => {
    if (!els.configPath.value.trim()) {
      showMessage("Enter a config path first.", true);
      return;
    }
    await loadConfig(els.configPath.value.trim(), true);
  });

  els.saveConfigBtn.addEventListener("click", saveConfig);
  els.applyBtn.addEventListener("click", applyIntegration);
  els.rerunDoctorBtn.addEventListener("click", runDoctor);

  els.settingsTabButtons.forEach((button) => {
    button.addEventListener("click", () => switchSettingsTab(button.dataset.settingsTab));
  });

  els.shimEditor.addEventListener("input", () => {
    state.shimText = els.shimEditor.value;
    schedulePreview();
    updateControls();
  });

  els.tomlBaseEditor.addEventListener("input", () => {
    state.baseTomlDirty = true;
    schedulePreview();
  });

  [els.shimEditor, els.tomlBaseEditor].forEach((editor) => {
    editor.addEventListener("keydown", handleEditorTabKey);
  });

  [els.projectDir, els.codexHome].forEach((element) => {
    element.addEventListener("input", handleTargetPathChange);
    element.addEventListener("change", handleTargetPathChange);
  });

  [els.envKey].forEach((element) => {
    element.addEventListener("input", schedulePreview);
    element.addEventListener("change", schedulePreview);
  });

  els.generalListen.addEventListener("input", () => {
    syncListenToYaml();
    schedulePreview();
  });
  els.generalListen.addEventListener("change", () => {
    syncListenToYaml();
    schedulePreview();
  });

  els.trustProject.addEventListener("change", () => {
    state.baseTomlDirty = false;
    schedulePreview();
  });

  [els.targetScopeProject, els.targetScopeUser].forEach((input) => {
    input.addEventListener("change", () => {
      if (!input.checked) {
        return;
      }
      state.targetScope = input.value;
      if (state.targetScope !== "project") {
        els.trustProject.checked = false;
      }
      state.baseTomlDirty = false;
      renderTargetScope();
      schedulePreview();
    });
  });

  els.modelsTableBody.addEventListener("click", (event) => {
    const row = event.target.closest("tr[data-slug]");
    if (!row) {
      return;
    }
    state.selectedModelSlug = row.dataset.slug;
    renderModels();
    renderSummary();
  });
}

function updateBrowseAvailability() {
  const disabled = !state.browseSupported;
  [els.browseConfigBtn, els.browseProjectDirBtn, els.browseCodexHomeBtn].forEach((button) => {
    button.disabled = disabled;
    if (disabled) {
      button.title = state.browseMessage;
    } else {
      button.removeAttribute("title");
    }
  });
}

function openDialog(name) {
  closeDialogs();
  if (name === "settings") {
    switchSettingsTab("general");
    els.settingsShell.classList.remove("hidden");
  } else if (name === "doctor") {
    els.doctorShell.classList.remove("hidden");
  }
}

function closeDialogs() {
  if (!els.confirmShell.classList.contains("hidden")) {
    resolveConfirm(false);
  }
  els.settingsShell.classList.add("hidden");
  els.doctorShell.classList.add("hidden");
}

function confirmManagedWrite(title, message, detail = "") {
  if (confirmResolver) {
    resolveConfirm(false);
  }

  els.confirmTitle.textContent = title;
  els.confirmMessage.textContent = message;
  els.confirmDetail.textContent = detail;
  els.confirmShell.classList.remove("hidden");
  els.confirmOkBtn.focus();

  return new Promise((resolve) => {
    confirmResolver = resolve;
  });
}

function resolveConfirm(result) {
  if (!confirmResolver) {
    els.confirmShell.classList.add("hidden");
    return;
  }
  const resolve = confirmResolver;
  confirmResolver = null;
  els.confirmShell.classList.add("hidden");
  resolve(Boolean(result));
}

function switchSettingsTab(name) {
  els.settingsTabButtons.forEach((button) => {
    button.classList.toggle("active", button.dataset.settingsTab === name);
  });
  els.settingsPanels.forEach((panel) => {
    panel.classList.toggle("active", panel.dataset.settingsPanel === name);
  });
  els.saveConfigBtn.classList.toggle("hidden", name !== "yaml");
}

async function browseIntoField(kind, field) {
  try {
    const path = await invoke("browse_path", {
      request: {
        kind,
        initial_path: blankToNull(field.value),
      },
    });
    if (!path) {
      return false;
    }
    field.value = path;
    return true;
  } catch (error) {
    showMessage(`Browse failed: ${error.message ?? error}`, true);
    return false;
  }
}

async function loadStarterDocument(path, message) {
  state.configPath = path;
  state.shimText = state.starterConfigText;
  state.savedShimText = "";
  state.baseTomlDirty = false;
  els.configPath.value = path;
  els.shimEditor.value = state.starterConfigText;
  els.activeConfigPath.textContent = path;

  if (state.starterConfigText.trim()) {
    await refreshPreview();
  } else {
    clearPreviewState();
  }

  showMessage(message);
}

async function loadConfig(path, announce) {
  try {
    const doc = await invoke("load_shim_config", { path });
    state.configPath = doc.path;
    state.shimText = doc.text;
    state.savedShimText = doc.text;
    state.baseTomlDirty = false;
    els.configPath.value = doc.path;
    els.shimEditor.value = doc.text;
    els.activeConfigPath.textContent = doc.path;
    applyInspection(doc.inspection);
    await refreshPreview();
    if (announce) {
      showMessage(`Loaded ${doc.path}`);
    }
  } catch (error) {
    const text = String(error.message ?? error);
    if (/No such file|failed to read/i.test(text) && state.starterConfigText.trim()) {
      await loadStarterDocument(
        path,
        `No config exists at ${path}. Starter template loaded; save YAML to create it.`,
      );
      return;
    }
    showMessage(`Load failed: ${text}`, true);
  } finally {
    updateControls();
  }
}

async function saveConfig() {
  if (!els.configPath.value.trim()) {
    showMessage("Enter a config path first.", true);
    return;
  }
  try {
    const doc = await invoke("save_shim_config", {
      request: {
        path: els.configPath.value.trim(),
        config_text: els.shimEditor.value,
      },
    });
    state.configPath = doc.path;
    state.shimText = doc.text;
    state.savedShimText = doc.text;
    state.defaultConfigExists = true;
    state.baseTomlDirty = false;
    els.activeConfigPath.textContent = doc.path;
    els.shimEditor.value = doc.text;
    applyInspection(doc.inspection);
    await refreshPreview();
    showMessage(`Saved ${doc.path}`);
  } catch (error) {
    showMessage(`Save failed: ${error.message ?? error}`, true);
  } finally {
    updateControls();
  }
}

function applyInspection(inspection) {
  state.inspection = inspection;
  state.catalogModels = parseCatalogModels(inspection.catalog_json);

  const preferredSlug =
    state.selectedModelSlug ||
    state.preview?.target_model ||
    inspection.summary?.model ||
    state.catalogModels[0]?.slug ||
    null;

  state.selectedModelSlug = state.catalogModels.some((model) => model.slug === preferredSlug)
    ? preferredSlug
    : state.catalogModels[0]?.slug ?? null;

  renderSummary();
  renderModels();
}

function clearPreviewState() {
  state.preview = null;
  state.inspection = null;
  state.catalogModels = [];
  state.selectedModelSlug = null;
  els.tomlTargetPath.value = "—";
  els.catalogTargetPath.value = "—";
  els.integrationTomlTarget.value = "—";
  els.integrationCatalogTarget.value = "—";
  els.mergedPreview.innerHTML = '<div class="merged-line empty">No merged preview yet.</div>';
  els.integrationSummary.textContent = "No merged preview yet.";
  renderSummary();
  renderModels();
}

function buildIntegrationOptions() {
  const projectDir =
    state.targetScope === "project" ? blankToNull(els.projectDir.value) : null;
  return {
    provider_id: "codex_shim",
    project_dir: projectDir,
    codex_home: blankToNull(els.codexHome.value),
    trust_project: projectDir ? els.trustProject.checked : false,
    env_key: blankToNull(els.envKey.value),
    web_search: null,
    base_toml_override: state.baseTomlDirty ? els.tomlBaseEditor.value : null,
  };
}

function handleTargetPathChange() {
  state.baseTomlDirty = false;
  schedulePreview();
}

function schedulePreview() {
  if (state.previewTimer) {
    clearTimeout(state.previewTimer);
  }
  const requestId = ++state.previewRequestId;
  state.previewTimer = window.setTimeout(() => {
    refreshPreview(requestId);
  }, 220);
}

async function refreshPreview(requestId = ++state.previewRequestId) {
  const configText = els.shimEditor.value;
  if (!configText.trim()) {
    clearPreviewState();
    updateControls();
    return;
  }

  let inspection;
  try {
    inspection = await invoke("inspect_shim_config", {
      request: { config_text: configText },
    });
  } catch (error) {
    if (requestId < state.previewRequestId) {
      return;
    }
    clearPreviewState();
    showMessage(`Preview failed: ${error.message ?? error}`, true);
    updateControls();
    return;
  }

  if (requestId < state.previewRequestId) {
    return;
  }

  applyInspection(inspection);

  try {
    const preview = await invoke("preview_codex_integration", {
      request: {
        config_text: configText,
        config_path: blankToNull(els.configPath.value),
        options: buildIntegrationOptions(),
      },
    });

    if (requestId < state.previewRequestId) {
      return;
    }

    const targetChanged = state.preview?.target_path !== preview.target_path;
    state.preview = preview;
    state.lastAppliedPreviewId = requestId;
    state.catalogModels = parseCatalogModels(preview.catalog_json);

    if (!state.selectedModelSlug || !state.catalogModels.some((model) => model.slug === state.selectedModelSlug)) {
      state.selectedModelSlug = preview.target_model || state.catalogModels[0]?.slug || null;
    }

    els.tomlTargetPath.value = preview.target_path;
    els.catalogTargetPath.value = preview.catalog_path;
    els.integrationTomlTarget.value = preview.target_path;
    els.integrationCatalogTarget.value = preview.catalog_path;

    if (targetChanged || !state.baseTomlDirty) {
      els.tomlBaseEditor.value = preview.original_toml;
      state.baseTomlDirty = false;
    }

    renderMergedPreview(
      els.tomlBaseEditor.value || preview.original_toml,
      preview.merged_toml,
    );
    renderSummary();
    renderModels();
  } catch (error) {
    if (requestId < state.previewRequestId) {
      return;
    }

    state.preview = null;
    state.catalogModels = parseCatalogModels(inspection.catalog_json);
    if (!state.catalogModels.some((model) => model.slug === state.selectedModelSlug)) {
      state.selectedModelSlug = state.catalogModels[0]?.slug ?? null;
    }
    els.tomlTargetPath.value = "—";
    els.catalogTargetPath.value = "—";
    els.integrationTomlTarget.value = "—";
    els.integrationCatalogTarget.value = "—";
    els.mergedPreview.innerHTML = '<div class="merged-line empty">Preview unavailable.</div>';
    els.integrationSummary.textContent = "Preview unavailable.";
    renderSummary();
    renderModels();
    showMessage(`Preview failed: ${error.message ?? error}`, true);
  } finally {
    if (requestId >= state.lastAppliedPreviewId) {
      updateControls();
    }
  }
}

async function applyIntegration() {
  const configText = els.shimEditor.value;
  if (!configText.trim()) {
    showMessage("Shim config is empty.", true);
    return;
  }

  const integration = integrationState();
  if (integration.pending) {
    const approved = await confirmManagedWrite(
      "Apply Integration",
      `Write Codex integration to ${state.preview?.target_path || scopeWriteLabel()}?`,
      "Only shim-managed keys will be updated.",
    );
    if (!approved) {
      return;
    }
  }

  try {
    state.applyPending = true;
    updateControls();
    await invoke("apply_codex_integration", {
      request: {
        config_text: configText,
        config_path: blankToNull(els.configPath.value),
        options: buildIntegrationOptions(),
      },
    });
    state.baseTomlDirty = false;
    await refreshPreview();
    showMessage(`Codex integration applied to ${scopeWriteLabel().toLowerCase()}.`);
  } catch (error) {
    showMessage(`Apply failed: ${error.message ?? error}`, true);
  } finally {
    state.applyPending = false;
    updateControls();
  }
}

async function runDoctor() {
  if (state.targetScope !== "project") {
    state.doctorReport = null;
    renderDoctor();
    showMessage("Doctor is only available for project-level Codex targets.", true);
    return;
  }

  const configText = els.shimEditor.value;
  if (!configText.trim()) {
    showMessage("Shim config is empty.", true);
    return;
  }

  state.doctorLoading = true;
  renderDoctor();
  try {
    const report = await invoke("doctor_desktop", {
      request: {
        config_text: configText,
        config_path: blankToNull(els.configPath.value),
        options: buildIntegrationOptions(),
      },
    });
    state.doctorReport = report;
    showMessage(
      report.checks.some((item) => item.status === "unsupported")
        ? "Doctor found issues."
        : "Doctor passed.",
      report.checks.some((item) => item.status === "unsupported"),
    );
  } catch (error) {
    state.doctorReport = {
      checks: [
        {
          status: "unsupported",
          subject: "doctor",
          detail: String(error.message ?? error),
        },
      ],
    };
    showMessage(`Doctor failed: ${error.message ?? error}`, true);
  } finally {
    state.doctorLoading = false;
    renderDoctor();
    updateControls();
  }
}

async function startRuntime(isRestart) {
  const configText = els.shimEditor.value;
  if (!configText.trim()) {
    showMessage("Shim config is empty.", true);
    return;
  }
  try {
    state.runtimeActionPending = true;
    updateControls();
    await invoke(isRestart ? "restart_runtime" : "start_runtime", {
      request: { config_text: configText },
    });
    await refreshRuntime();

    const integration = integrationState();
    if (integration.pending) {
      showMessage(
        `${isRestart ? "Runtime restarted" : "Runtime started"} from the current YAML. ${integration.shortLabel} on the Codex target.`,
      );
    } else {
      showMessage(isRestart ? "Runtime restarted." : "Runtime started.");
    }
  } catch (error) {
    showMessage(`Runtime failed: ${error.message ?? error}`, true);
  } finally {
    state.runtimeActionPending = false;
    updateControls();
  }
}

async function stopRuntime() {
  try {
    state.runtimeActionPending = true;
    updateControls();
    await invoke("stop_runtime");
    showMessage("Runtime stopped.");
    await refreshRuntime();
  } catch (error) {
    showMessage(`Stop failed: ${error.message ?? error}`, true);
  } finally {
    state.runtimeActionPending = false;
    updateControls();
  }
}

async function refreshRuntime() {
  if (state.runtimeRefreshing) {
    return;
  }
  state.runtimeRefreshing = true;
  try {
    const runtime = await invoke("get_runtime_snapshot", {
      request: { range_minutes: state.rangeMinutes },
    });
    state.runtime = runtime;
    renderRuntime(runtime);
  } catch (error) {
    showMessage(`Runtime snapshot failed: ${error.message ?? error}`, true);
  } finally {
    state.runtimeRefreshing = false;
  }
}

async function refreshLogs() {
  if (state.logsPaused || state.logRefreshing) {
    return;
  }
  state.logRefreshing = true;
  try {
    const batch = await invoke("get_logs", { cursor: state.logCursor });
    state.logCursor = batch.next_cursor;
    if (batch.entries.length) {
      state.logEntries.push(...batch.entries);
      if (state.logEntries.length > 2000) {
        state.logEntries.splice(0, state.logEntries.length - 2000);
      }
      renderLogs();
    }
  } catch (error) {
    showMessage(`Log refresh failed: ${error.message ?? error}`, true);
  } finally {
    state.logRefreshing = false;
  }
}

function renderRuntime(runtime) {
  const summary = currentSummary(runtime);
  const running = Boolean(runtime?.running);

  els.statusChip.classList.toggle("running", running);
  els.statusChip.classList.toggle("stopped", !running);
  els.statusLabel.textContent = running ? "Running" : "Stopped";
  els.titleStatus.textContent = running ? "Running" : "Stopped";
  els.listenLabel.textContent = running ? runtime.listen : summary.listen || "Not running";

  els.runtimeProvider.textContent = summary.provider || "—";
  els.runtimeModel.textContent = summary.model || "—";
  els.runtimeEndpoint.textContent = summary.endpoint || "—";
  els.runtimeBackend.textContent = summary.backend || "—";
  els.runtimeUpstream.textContent = summary.upstream || "—";
  els.runtimeListen.textContent = summary.listen || "—";

  els.requestCount.textContent = formatCount(runtime?.request_count ?? 0);
  els.completedCount.textContent = formatCount(runtime?.completed_request_count ?? 0);
  els.storeCount.textContent = formatCount(runtime?.store_size ?? 0);
  els.errorCount.textContent = formatCount(runtime?.error_count ?? 0);
  els.runtimeUptime.textContent = formatUptime(runtime?.uptime_seconds ?? 0);
  els.lastErrorField.textContent = runtime?.last_error || "None";

  drawTokenChart(runtime?.token_series);
  renderSummary();
  updateControls();
}

function currentSummary(runtime) {
  const inspection = state.inspection?.summary ?? {};
  return {
    provider: runtime?.running ? runtime.provider || inspection.provider_kind : inspection.provider_kind,
    model: runtime?.running ? runtime.model || inspection.model : inspection.model,
    endpoint: runtime?.running
      ? runtime.endpoint_mode || state.inspection?.endpoint_mode
      : state.inspection?.endpoint_mode,
    backend: runtime?.running ? runtime.state_backend || inspection.state_backend : inspection.state_backend,
    upstream: runtime?.running
      ? runtime.upstream_base_url || inspection.upstream_base_url
      : inspection.upstream_base_url,
    listen: runtime?.running ? runtime.listen || inspection.listen : inspection.listen,
  };
}

function drawTokenChart(series) {
  els.chart.innerHTML = "";
  const buckets = series?.buckets ?? [];
  const maxValue = Math.max(...buckets.map((bucket) => Number(bucket.total_tokens || 0)), 0);
  const nonZeroBuckets = buckets.filter((bucket) => Number(bucket.total_tokens || 0) > 0);

  els.chartWindowLabel.textContent = `Showing ${rangeLabel(state.rangeMinutes)}.`;
  els.partialBadge.classList.toggle("hidden", !series?.partial_usage);

  if (!buckets.length || maxValue === 0 || !nonZeroBuckets.length) {
    els.chartEmpty.classList.remove("hidden");
    els.chartLastSample.textContent = "No samples.";
    return;
  }

  els.chartEmpty.classList.add("hidden");

  const width = 560;
  const height = 228;
  const padLeft = 20;
  const padRight = 10;
  const padTop = 10;
  const padBottom = 22;
  const plotWidth = width - padLeft - padRight;
  const plotHeight = height - padTop - padBottom;
  const baselineY = height - padBottom;
  const count = buckets.length;
  const xFor = (index) => padLeft + (plotWidth * index) / Math.max(1, count - 1);
  const yFor = (value) => baselineY - (Number(value) / maxValue) * plotHeight;

  const defs = svgEl("defs");
  defs.appendChild(buildPattern("chart-input-pattern", "#ffffff", "#000080"));
  defs.appendChild(buildPattern("chart-output-pattern", "#c0c0c0", "#000000"));
  els.chart.appendChild(defs);

  els.chart.appendChild(
    svgEl("rect", {
      x: 0,
      y: 0,
      width,
      height,
      fill: "#ffffff",
    }),
  );

  for (let step = 0; step <= 4; step += 1) {
    const y = padTop + (plotHeight * step) / 4;
    els.chart.appendChild(
      svgEl("line", {
        x1: padLeft,
        y1: y,
        x2: width - padRight,
        y2: y,
        stroke: "#d0d0d0",
        "stroke-width": "1",
      }),
    );
  }

  const inputPoints = buckets.map((bucket, index) => [xFor(index), yFor(bucket.input_tokens)]);
  const totalPoints = buckets.map((bucket, index) => [xFor(index), yFor(bucket.total_tokens)]);

  els.chart.appendChild(
    svgEl("path", {
      d: areaPath(totalPoints, baselineY),
      fill: "url(#chart-output-pattern)",
      stroke: "none",
    }),
  );
  els.chart.appendChild(
    svgEl("path", {
      d: areaPath(inputPoints, baselineY),
      fill: "url(#chart-input-pattern)",
      stroke: "none",
    }),
  );
  els.chart.appendChild(
    svgEl("path", {
      d: linePath(totalPoints),
      fill: "none",
      stroke: "#000000",
      "stroke-width": "1",
    }),
  );
  els.chart.appendChild(
    svgEl("path", {
      d: linePath(inputPoints),
      fill: "none",
      stroke: "#000080",
      "stroke-width": "1",
    }),
  );
  els.chart.appendChild(
    svgEl("line", {
      x1: padLeft,
      y1: baselineY,
      x2: width - padRight,
      y2: baselineY,
      stroke: "#000000",
      "stroke-width": "1",
    }),
  );

  const firstMinute = new Date(buckets[0].minute_start_unix * 1000);
  const lastBucket = buckets[buckets.length - 1];
  const lastMinute = new Date(lastBucket.minute_start_unix * 1000);
  els.chartLastSample.textContent =
    `${formatClock(firstMinute)} -> ${formatClock(lastMinute)} | ` +
    `last ${lastBucket.total_tokens} total (${lastBucket.input_tokens} in / ${lastBucket.output_tokens} out)`;
}

function buildPattern(id, background, stroke) {
  const pattern = svgEl("pattern", {
    id,
    width: 8,
    height: 8,
    patternUnits: "userSpaceOnUse",
  });
  pattern.appendChild(svgEl("rect", { x: 0, y: 0, width: 8, height: 8, fill: background }));
  pattern.appendChild(svgEl("path", { d: "M0 0 L0 8 M4 0 L4 8", stroke, "stroke-width": "1" }));
  return pattern;
}

function svgEl(tag, attrs = {}) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, String(value)));
  return node;
}

function linePath(points) {
  return points
    .map(([x, y], index) => `${index === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`)
    .join(" ");
}

function areaPath(points, baselineY) {
  const line = linePath(points);
  const first = points[0];
  const last = points[points.length - 1];
  return `${line} L ${last[0].toFixed(2)} ${baselineY.toFixed(2)} L ${first[0].toFixed(2)} ${baselineY.toFixed(2)} Z`;
}

function renderLogs() {
  const filtered = state.logEntries.filter((entry) => {
    const enabled = state.logLevels[entry.level.toUpperCase()] ?? true;
    const searchMatch =
      !state.logSearch ||
      entry.message.toLowerCase().includes(state.logSearch) ||
      entry.level.toLowerCase().includes(state.logSearch);
    return enabled && searchMatch;
  });

  if (!filtered.length) {
    els.logStream.innerHTML = '<div class="empty-state">No matching log entries.</div>';
  } else {
    els.logStream.innerHTML = filtered
      .map(
        (entry) => `
          <div class="log-entry ${escapeHtml(entry.level)}">
            <div>[${escapeHtml(entry.timestamp)}]</div>
            <div>[${escapeHtml(entry.level)}]</div>
            <div>${escapeHtml(entry.message)}</div>
          </div>
        `,
      )
      .join("");
  }

  els.logCount.textContent = `${formatCount(filtered.length)} entries`;
  if (state.autoScroll) {
    scrollLogsToBottom();
  }
}

function renderMergedPreview(baseText, mergedText) {
  const ops = diffLines(baseText, mergedText);
  let added = 0;
  let removed = 0;
  let modified = 0;
  const lines = [];

  for (const op of ops) {
    if (op.type === "equal") {
      lines.push(
        `<div class="merged-line"><span class="merged-line-fill">${escapeHtml(op.text || " ")}</span></div>`,
      );
    } else if (op.type === "added") {
      added += 1;
      lines.push(
        `<div class="merged-line added"><span class="merged-line-fill">${escapeHtml(op.text || " ")}</span></div>`,
      );
    } else if (op.type === "modified") {
      modified += 1;
      lines.push(
        `<div class="merged-line changed"><span class="merged-line-fill">${escapeHtml(op.text || " ")}</span></div>`,
      );
    } else if (op.type === "removed") {
      removed += 1;
    }
  }

  els.mergedPreview.innerHTML =
    lines.join("") || '<div class="merged-line empty">(empty file)</div>';

  const integration = integrationState();
  if (integration.inSync) {
    els.integrationSummary.textContent =
      "Managed Codex keys already match the target TOML. Unmanaged user keys are preserved.";
    return;
  }

  els.integrationSummary.textContent =
    `${modified} changed, ${added} added, ${removed} removed. ` +
    "Only shim-managed Codex keys are rewritten; unrelated user TOML stays intact.";
}

function renderModels() {
  const models = state.catalogModels;
  const defaultModel = state.preview?.target_model || state.inspection?.summary?.model || "—";
  els.modelsCount.textContent = `${models.length} model${models.length === 1 ? "" : "s"}`;
  els.modelsDefaultLabel.textContent = `Default: ${defaultModel}`;

  if (!models.length) {
    els.modelsTableBody.innerHTML =
      '<tr><td colspan="4" class="empty-state">No catalog available.</td></tr>';
    els.modelDetailTitle.textContent = "No model selected";
    els.modelDetailList.innerHTML = "";
    els.modelDetailJson.textContent = "";
    return;
  }

  if (!models.some((model) => model.slug === state.selectedModelSlug)) {
    state.selectedModelSlug = state.preview?.target_model || models[0].slug;
  }

  els.modelsTableBody.innerHTML = models
    .map((model) => {
      const selected = model.slug === state.selectedModelSlug;
      const reasoning = formatReasoningLevels(model);
      const modalities = (model.input_modalities || []).join(", ") || "text";

      return `
        <tr data-slug="${escapeHtml(model.slug)}" class="${selected ? "selected" : ""}">
          <td>${escapeHtml(model.display_name || model.slug)}</td>
          <td>${escapeHtml(String(model.context_window))}</td>
          <td>${escapeHtml(reasoning)}</td>
          <td>${escapeHtml(modalities)}</td>
        </tr>
      `;
    })
    .join("");

  const selected = selectedCatalogModel();
  els.modelDetailTitle.textContent = selected.display_name || selected.slug;
  els.modelDetailList.innerHTML = [
    ["Slug", selected.slug],
    ["Context", selected.context_window],
    ["Modalities", (selected.input_modalities || []).join(", ") || "text"],
    ["Reasoning", formatReasoningLevels(selected)],
  ]
    .map(
      ([label, value]) =>
        `<dt>${escapeHtml(label)}</dt><dd>${escapeHtml(String(value))}</dd>`,
    )
    .join("");
  els.modelDetailJson.textContent = `${JSON.stringify(selected, null, 2)}\n`;
}

function renderDoctor() {
  els.doctorProgress.classList.toggle("hidden", !state.doctorLoading);

  if (state.doctorLoading) {
    els.doctorSummary.textContent = "Checking project trust, provider wiring, and catalog paths...";
    els.doctorTableBody.innerHTML =
      '<tr><td colspan="3" class="empty-state">Running checks...</td></tr>';
    return;
  }

  if (state.targetScope !== "project" && !state.doctorReport) {
    els.doctorSummary.textContent = "Doctor only runs against project-level Codex targets.";
    els.doctorTableBody.innerHTML =
      '<tr><td colspan="3" class="empty-state">Switch General -> Write target to project scope to run doctor.</td></tr>';
    return;
  }

  const checks = state.doctorReport?.checks ?? [];
  if (!checks.length) {
    els.doctorSummary.textContent = "Run doctor against the current Codex integration target.";
    els.doctorTableBody.innerHTML =
      '<tr><td colspan="3" class="empty-state">No checks yet.</td></tr>';
    return;
  }

  const supported = checks.filter((item) => item.status === "supported").length;
  const gated = checks.filter((item) => item.status === "gated").length;
  const unsupported = checks.filter((item) => item.status === "unsupported").length;
  els.doctorSummary.textContent =
    `${supported} supported, ${gated} gated, ${unsupported} unsupported`;

  els.doctorTableBody.innerHTML = checks
    .map(
      (check) => `
        <tr>
          <td>${escapeHtml(check.subject)}</td>
          <td class="doctor-status-${escapeHtml(check.status)}">${escapeHtml(check.status.toUpperCase())}</td>
          <td>${escapeHtml(check.detail)}</td>
        </tr>
      `,
    )
    .join("");
}

function renderSummary() {
  const summary = currentSummary(state.runtime);
  const model = selectedCatalogModel();
  const integration = integrationState();

  els.generalProvider.textContent = summary.provider || "—";
  els.generalModel.textContent = summary.model || "—";
  els.generalEndpoint.textContent = summary.endpoint || "—";
  els.generalBackend.textContent = summary.backend || "—";
  els.generalListen.value = summary.listen || "—";
  els.generalUpstream.value = summary.upstream || "—";
  els.generalWriteScope.textContent = scopeWriteLabel();
  els.generalApplyState.textContent = integration.longLabel;
  els.integrationStatus.textContent = `Codex integration: ${integration.longLabel}`;

  els.surfaceWebSearchCapability.textContent = currentWebSearchMode();
  els.surfaceSearchCapability.textContent = model
    ? model.supports_search_tool
      ? model.web_search_tool_type || "text"
      : "off"
    : "—";
  els.surfaceParallelCapability.textContent = model ? yesNo(model.supports_parallel_tool_calls) : "—";
  els.surfaceSummaryCapability.textContent = model ? yesNo(model.supports_reasoning_summaries) : "—";
  els.surfaceImageCapability.textContent = model ? yesNo(model.supports_image_detail_original) : "—";
  els.surfacePatchCapability.textContent = model ? model.apply_patch_tool_type || "off" : "—";
  els.surfaceReasoningCapability.textContent = model ? formatReasoningLevels(model) : "—";

  renderTargetScope();
}

function renderTargetScope() {
  const projectScope = state.targetScope === "project";
  els.targetScopeProject.checked = projectScope;
  els.targetScopeUser.checked = !projectScope;
  els.trustProject.disabled = !projectScope;
  if (!projectScope) {
    els.trustProject.checked = false;
  }
}

function updateControls() {
  const hasConfigText = Boolean(els.shimEditor.value.trim());
  const hasValidInspection = Boolean(state.inspection);
  const canApply = hasConfigText && hasValidInspection && Boolean(state.preview);
  const canDoctor =
    hasConfigText && hasValidInspection && Boolean(state.preview) && state.targetScope === "project";
  const yamlDirty = normalizeText(els.shimEditor.value) !== normalizeText(state.savedShimText);

  els.saveConfigBtn.disabled = !hasConfigText || !els.configPath.value.trim() || !yamlDirty;
  els.applyBtn.disabled = !canApply || state.applyPending;
  els.startBtn.disabled =
    !hasValidInspection || Boolean(state.runtime?.running) || state.runtimeActionPending;
  els.restartBtn.disabled = !hasValidInspection || state.runtimeActionPending;
  els.stopBtn.disabled = !state.runtime?.running || state.runtimeActionPending;
  els.settingsBtn.disabled = false;
  els.doctorBtn.disabled = !canDoctor || state.doctorLoading;
  els.rerunDoctorBtn.disabled = !canDoctor || state.doctorLoading;
}

function integrationState() {
  if (!state.preview) {
    return {
      inSync: false,
      pending: false,
      longLabel: "no preview",
      shortLabel: "No preview",
    };
  }

  const inSync = normalizeText(state.preview.original_toml) === normalizeText(state.preview.merged_toml);
  const scope = state.preview.mode === "project" ? "project target" : "user target";
  return {
    inSync,
    pending: !inSync,
    longLabel: inSync ? `${scope} in sync` : `${scope} pending apply`,
    shortLabel: inSync ? "In sync" : "Pending apply",
  };
}

function selectedCatalogModel() {
  return (
    state.catalogModels.find((model) => model.slug === state.selectedModelSlug) ||
    state.catalogModels.find((model) => model.slug === state.preview?.target_model) ||
    state.catalogModels[0] ||
    null
  );
}

function scopeWriteLabel() {
  return state.targetScope === "project" ? "Project (.codex)" : "User ($CODEX_HOME)";
}

function currentWebSearchMode() {
  if (!state.preview) {
    return "—";
  }
  const text = state.preview?.merged_toml || state.preview?.original_toml || "";
  const match = text.match(/^\s*web_search\s*=\s*"([^"]+)"/m);
  return match?.[1] || "disabled";
}

function handleEditorTabKey(event) {
  if (event.key !== "Tab") {
    return;
  }

  event.preventDefault();
  const textarea = event.currentTarget;
  const value = textarea.value;
  const selectionStart = textarea.selectionStart;
  const selectionEnd = textarea.selectionEnd;
  const lineStart = value.lastIndexOf("\n", selectionStart - 1) + 1;
  const lineEndIndex = value.indexOf("\n", selectionEnd);
  const blockEnd = lineEndIndex === -1 ? value.length : lineEndIndex;
  const block = value.slice(lineStart, blockEnd);
  const lines = block.split("\n");

  if (selectionStart === selectionEnd && !event.shiftKey) {
    textarea.value = `${value.slice(0, selectionStart)}  ${value.slice(selectionEnd)}`;
    textarea.selectionStart = selectionStart + 2;
    textarea.selectionEnd = selectionStart + 2;
  } else if (event.shiftKey) {
    let removedBeforeStart = 0;
    let removedTotal = 0;
    const updatedLines = lines.map((line, index) => {
      if (line.startsWith("  ")) {
        if (index === 0) {
          removedBeforeStart = 2;
        }
        removedTotal += 2;
        return line.slice(2);
      }
      if (line.startsWith("\t")) {
        if (index === 0) {
          removedBeforeStart = 1;
        }
        removedTotal += 1;
        return line.slice(1);
      }
      return line;
    });
    textarea.value =
      `${value.slice(0, lineStart)}${updatedLines.join("\n")}${value.slice(blockEnd)}`;
    textarea.selectionStart = Math.max(lineStart, selectionStart - removedBeforeStart);
    textarea.selectionEnd = Math.max(textarea.selectionStart, selectionEnd - removedTotal);
  } else {
    const updatedLines = lines.map((line) => `  ${line}`);
    textarea.value =
      `${value.slice(0, lineStart)}${updatedLines.join("\n")}${value.slice(blockEnd)}`;
    textarea.selectionStart = selectionStart + 2;
    textarea.selectionEnd = selectionEnd + lines.length * 2;
  }

  textarea.dispatchEvent(new Event("input", { bubbles: true }));
}

function syncListenToYaml() {
  const newListen = els.generalListen.value.trim();
  if (!newListen) {
    return;
  }
  let yaml = els.shimEditor.value;
  // Replace the server.listen line
  const replaced = yaml.replace(
    /^(\s*listen:\s*)(["']?)([^"'\n]*)(\2)/m,
    `$1"${newListen}"`
  );
  if (replaced !== yaml) {
    els.shimEditor.value = replaced;
    state.shimText = replaced;
  }
}

function parseCatalogModels(catalogText) {
  try {
    const parsed = JSON.parse(catalogText);
    return Array.isArray(parsed.models) ? parsed.models : [];
  } catch {
    return [];
  }
}

function diffLines(baseText, mergedText) {
  const a = baseText.replace(/\r\n/g, "\n").split("\n");
  const b = mergedText.replace(/\r\n/g, "\n").split("\n");
  const dp = Array.from({ length: a.length + 1 }, () => Array(b.length + 1).fill(0));

  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const ops = [];
  let i = 0;
  let j = 0;

  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      ops.push({ type: "equal", text: a[i] });
      i += 1;
      j += 1;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      ops.push({ type: "removed", text: a[i] });
      i += 1;
    } else {
      ops.push({ type: "added", text: b[j] });
      j += 1;
    }
  }

  while (i < a.length) {
    ops.push({ type: "removed", text: a[i] });
    i += 1;
  }

  while (j < b.length) {
    ops.push({ type: "added", text: b[j] });
    j += 1;
  }

  const normalized = [];
  for (let index = 0; index < ops.length; index += 1) {
    const current = ops[index];
    const next = ops[index + 1];
    if (current.type === "removed" && next?.type === "added") {
      normalized.push({ type: "modified", text: next.text });
      index += 1;
    } else {
      normalized.push(current);
    }
  }
  return normalized;
}

function exportLogs() {
  const text = state.logEntries
    .map((entry) => `[${entry.timestamp}] [${entry.level}] ${entry.message}`)
    .join("\n");
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "codex-shim-logs.txt";
  anchor.click();
  URL.revokeObjectURL(url);
}

function scrollLogsToBottom() {
  els.logStream.scrollTop = els.logStream.scrollHeight;
}

function isNearBottom(node) {
  return node.scrollHeight - node.scrollTop - node.clientHeight < 24;
}

function rangeLabel(minutes) {
  if (minutes === 15) {
    return "last 15 minutes";
  }
  if (minutes === 1440) {
    return "last 24 hours";
  }
  return "last 1 hour";
}

function formatClock(date) {
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatCount(value) {
  return Number(value || 0).toLocaleString();
}

function formatUptime(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0) {
    return `${hours}h ${minutes}m uptime`;
  }
  return `${minutes}m uptime`;
}

function yesNo(value) {
  return value ? "yes" : "no";
}

function formatReasoningLevels(model) {
  const levels = model?.supported_reasoning_levels?.map((item) => item.effort) || [];
  return levels.length ? levels.join(", ") : "none";
}

function normalizeText(value) {
  return String(value ?? "").replace(/\r\n/g, "\n").trimEnd();
}

function blankToNull(value) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function showMessage(message, isError = false) {
  els.statusMessage.textContent = message;
  els.statusMessage.classList.toggle("error", isError);
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

bootstrap();
