(function initializeBannerScheduler() {
  "use strict";

  const scheduler = window.BannerScheduler;
  if (!scheduler) {
    throw new Error("BannerScheduler core is not loaded");
  }

  const elementIds = [
    "source-file",
    "source-drop-zone",
    "status",
    "source-count",
    "visible-count",
    "selected-count",
    "filter-query",
    "filter-id-prefix",
    "filter-flags",
    "filter-types",
    "filter-selection",
    "select-visible",
    "clear-visible",
    "pool-list",
    "selected-queue",
    "queue-sort",
    "clear-queue",
    "schedule-mode",
    "start-date",
    "start-date-field",
    "days-per-batch",
    "days-per-batch-field",
    "pools-per-batch",
    "pools-per-batch-field",
    "schedule-preview",
    "preset-name",
    "preset-file",
    "export-preset",
    "workspace-name",
    "workspace-select",
    "save-workspace",
    "load-workspace",
    "delete-workspace",
  ];
  const elements = Object.fromEntries(
    elementIds.map((id) => [id, document.getElementById(id)]),
  );

  const state = {
    pools: [],
    sourceFileName: "",
    selectedPoolIds: [],
    filters: {
      query: "",
      idPrefix: "",
      bannerFlags: [],
      types: [],
      selection: "all",
    },
    schedule: {
      mode: "simultaneous",
      startDate: shanghaiToday(),
      daysPerBatch: 7,
      poolsPerBatch: 2,
      manualDates: {},
    },
    draggedPoolId: null,
    activeWorkspaceName: "",
  };

  elements["start-date"].value = state.schedule.startDate;
  renderWorkspaceOptions();
  bindEvents();
  renderAll();

  function shanghaiToday() {
    return new Date(Date.now() + 8 * 60 * 60 * 1000)
      .toISOString()
      .slice(0, 10);
  }

  function addCalendarDays(dateText, days) {
    const [year, month, day] = dateText.split("-").map(Number);
    return new Date(Date.UTC(year, month - 1, day + days))
      .toISOString()
      .slice(0, 10);
  }

  function unixToShanghaiDate(timestamp) {
    const parts = new Intl.DateTimeFormat("en-CA", {
      timeZone: "Asia/Shanghai",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).formatToParts(new Date(timestamp * 1000));
    const values = Object.fromEntries(parts.map((part) => [part.type, part.value]));
    return `${values.year}-${values.month}-${values.day}`;
  }

  function formatUnix(timestamp) {
    if (timestamp === 2147483647) {
      return "持续开放（2038 上限）";
    }
    return new Intl.DateTimeFormat("zh-CN", {
      timeZone: "Asia/Shanghai",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date(timestamp * 1000));
  }

  function setStatus(message, kind = "info") {
    elements.status.textContent = message;
    elements.status.dataset.kind = kind;
  }

  function clearElement(element) {
    while (element.firstChild) {
      element.removeChild(element.firstChild);
    }
  }

  function appendCell(row, text, className = "") {
    const cell = document.createElement("td");
    cell.textContent = text;
    if (className) {
      cell.className = className;
    }
    row.appendChild(cell);
    return cell;
  }

  function appendEmptyRow(container, columnCount, text) {
    const row = document.createElement("tr");
    const cell = appendCell(row, text, "empty");
    cell.colSpan = columnCount;
    container.appendChild(row);
  }

  function poolById(poolId) {
    return state.pools.find((pool) => pool.id === poolId);
  }

  function selectedIdSet() {
    return new Set(state.selectedPoolIds);
  }

  function currentVisiblePools() {
    return scheduler.filterPools(state.pools, state.filters, selectedIdSet());
  }

  function ensureManualDates(poolId) {
    if (!state.schedule.manualDates[poolId]) {
      const onlineDate = state.schedule.startDate || shanghaiToday();
      state.schedule.manualDates[poolId] = {
        onlineDate,
        offlineDate: addCalendarDays(onlineDate, 1),
      };
    }
  }

  function selectPool(poolId) {
    if (!state.selectedPoolIds.includes(poolId)) {
      state.selectedPoolIds.push(poolId);
      ensureManualDates(poolId);
    }
  }

  function removePool(poolId) {
    state.selectedPoolIds = state.selectedPoolIds.filter(
      (selectedPoolId) => selectedPoolId !== poolId,
    );
  }

  function renderFilterOptions(container, values, selectedValues, fieldName) {
    clearElement(container);
    if (values.length === 0) {
      const empty = document.createElement("span");
      empty.className = "muted";
      empty.textContent = "载入后生成";
      container.appendChild(empty);
      return;
    }
    for (const value of values) {
      const label = document.createElement("label");
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.value = String(value);
      checkbox.dataset.filterField = fieldName;
      checkbox.checked = selectedValues.includes(value);
      label.append(checkbox, document.createTextNode(String(value)));
      container.appendChild(label);
    }
  }

  function populateDynamicFilters() {
    const flags = [...new Set(state.pools.map((pool) => pool.bannerFlag))].sort(
      (left, right) => left - right,
    );
    const types = [...new Set(state.pools.map((pool) => pool.type))].sort(
      (left, right) => left - right,
    );
    renderFilterOptions(
      elements["filter-flags"],
      flags,
      state.filters.bannerFlags,
      "bannerFlags",
    );
    renderFilterOptions(
      elements["filter-types"],
      types,
      state.filters.types,
      "types",
    );
  }

  function renderPoolList() {
    const container = elements["pool-list"];
    clearElement(container);
    const visiblePools = currentVisiblePools();
    const selectedIds = selectedIdSet();
    if (visiblePools.length === 0) {
      appendEmptyRow(
        container,
        6,
        state.pools.length ? "没有符合当前筛选条件的卡池" : "尚未载入数据",
      );
    }

    for (const pool of visiblePools) {
      const row = document.createElement("tr");
      if (selectedIds.has(pool.id)) {
        row.className = "is-selected";
      }
      const checkCell = document.createElement("td");
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = selectedIds.has(pool.id);
      checkbox.dataset.poolId = String(pool.id);
      checkbox.setAttribute("aria-label", `选择卡池 ${pool.id} ${pool.nameEn}`);
      checkCell.appendChild(checkbox);
      row.appendChild(checkCell);
      appendCell(row, String(pool.id));
      appendCell(row, pool.nameEn || "（无英文名称）");
      appendCell(row, String(pool.bannerFlag));
      appendCell(row, String(pool.type));
      appendCell(row, String(pool.priority));
      container.appendChild(row);
    }

    elements["source-count"].textContent = String(state.pools.length);
    elements["visible-count"].textContent = String(visiblePools.length);
    elements["selected-count"].textContent = String(state.selectedPoolIds.length);
  }

  function makeRowButton(label, action, poolId, disabled = false) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.dataset.action = action;
    button.dataset.poolId = String(poolId);
    button.disabled = disabled;
    return button;
  }

  function renderSelectedQueue() {
    const container = elements["selected-queue"];
    clearElement(container);
    if (state.selectedPoolIds.length === 0) {
      appendEmptyRow(container, 6, "从上方选择卡池后在此调整顺序");
      return;
    }

    for (const [index, poolId] of state.selectedPoolIds.entries()) {
      const pool = poolById(poolId);
      const row = document.createElement("tr");
      row.draggable = true;
      row.dataset.poolId = String(poolId);
      appendCell(row, String(index + 1));
      appendCell(row, String(poolId));
      appendCell(row, pool?.nameEn || "未知卡池");

      ensureManualDates(poolId);
      for (const role of ["online", "offline"]) {
        const cell = document.createElement("td");
        const input = document.createElement("input");
        input.type = "date";
        input.max = "2038-01-18";
        input.className = "manual-date";
        input.dataset.manualRole = role;
        input.dataset.poolId = String(poolId);
        input.value =
          role === "online"
            ? state.schedule.manualDates[poolId].onlineDate
            : state.schedule.manualDates[poolId].offlineDate;
        input.disabled = state.schedule.mode !== "manual";
        cell.appendChild(input);
        row.appendChild(cell);
      }

      const actionCell = document.createElement("td");
      actionCell.className = "row-actions";
      actionCell.append(
        makeRowButton("↑", "up", poolId, index === 0),
        makeRowButton(
          "↓",
          "down",
          poolId,
          index === state.selectedPoolIds.length - 1,
        ),
        makeRowButton("移除", "remove", poolId),
      );
      row.appendChild(actionCell);
      container.appendChild(row);
    }
  }

  function readScheduleControls() {
    state.schedule.mode = elements["schedule-mode"].value;
    state.schedule.startDate = elements["start-date"].value;
    state.schedule.daysPerBatch = Number(elements["days-per-batch"].value);
    state.schedule.poolsPerBatch = Number(elements["pools-per-batch"].value);
    return {
      mode: state.schedule.mode,
      startDate: state.schedule.startDate,
      daysPerBatch: state.schedule.daysPerBatch,
      poolsPerBatch: state.schedule.poolsPerBatch,
      manualDates: state.schedule.manualDates,
    };
  }

  function calculateCurrentSchedule() {
    return scheduler.buildSchedule(
      state.selectedPoolIds,
      readScheduleControls(),
    );
  }

  function renderScheduleControls() {
    const isBatch = state.schedule.mode === "batch";
    const isManual = state.schedule.mode === "manual";
    elements["start-date-field"].hidden = isManual;
    elements["days-per-batch-field"].hidden = !isBatch;
    elements["pools-per-batch-field"].hidden = !isBatch;
  }

  function renderSchedulePreview() {
    const container = elements["schedule-preview"];
    clearElement(container);
    if (state.selectedPoolIds.length === 0) {
      appendEmptyRow(container, 4, "选择卡池后生成预览");
      elements["export-preset"].disabled = true;
      return;
    }

    try {
      const scheduledPools = calculateCurrentSchedule();
      for (const scheduled of scheduledPools) {
        const row = document.createElement("tr");
        let position = String(scheduled.order + 1);
        if (state.schedule.mode === "batch") {
          position = `批次 ${Math.floor(scheduled.order / state.schedule.poolsPerBatch) + 1}`;
        } else if (state.schedule.mode === "simultaneous") {
          position = "同时";
        }
        appendCell(row, position);
        const pool = poolById(scheduled.poolId);
        appendCell(row, `${scheduled.poolId} · ${pool?.nameEn || "未知卡池"}`);
        appendCell(row, formatUnix(scheduled.onlineTime));
        appendCell(row, formatUnix(scheduled.offlineTime));
        container.appendChild(row);
      }
      elements["export-preset"].disabled = false;
    } catch (error) {
      appendEmptyRow(container, 4, error.message);
      elements["export-preset"].disabled = true;
    }
  }

  function renderAll() {
    renderPoolList();
    renderSelectedQueue();
    renderScheduleControls();
    renderSchedulePreview();
  }

  async function loadSourceFile(file) {
    if (!file) {
      return;
    }
    try {
      const parsedPools = scheduler.parseSourceJson(await file.text());
      state.pools = parsedPools;
      state.sourceFileName = file.name;
      state.selectedPoolIds = [];
      state.schedule.manualDates = {};
      state.activeWorkspaceName = "";
      populateDynamicFilters();
      renderAll();
      setStatus(`已载入 ${parsedPools.length} 个唯一卡池：${file.name}`, "success");
    } catch (error) {
      setStatus(`载入失败：${error.message}`, "error");
    } finally {
      elements["source-file"].value = "";
    }
  }

  function applyPreset(presetKey) {
    if (state.pools.length === 0) {
      setStatus("请先载入 summon_pool.json", "error");
      return;
    }
    const selectedPoolIds = scheduler.selectPreset(state.pools, presetKey);
    state.selectedPoolIds = selectedPoolIds;
    for (const poolId of selectedPoolIds) {
      ensureManualDates(poolId);
    }
    renderAll();
    const kind = selectedPoolIds.length > 0 ? "success" : "info";
    setStatus(`预设已选择 ${selectedPoolIds.length} 个卡池`, kind);
  }

  function updateFilterFromCheckboxes(fieldName) {
    const selector = `input[data-filter-field="${fieldName}"]:checked`;
    state.filters[fieldName] = [...document.querySelectorAll(selector)].map(
      (checkbox) => Number(checkbox.value),
    );
    renderPoolList();
  }

  function renderWorkspaceOptions(selectedName = state.activeWorkspaceName) {
    const container = elements["workspace-select"];
    clearElement(container);
    const workspaces = readWorkspaceStore();
    const emptyOption = document.createElement("option");
    emptyOption.value = "";
    emptyOption.textContent = workspaces.length
      ? "选择已保存预设"
      : "暂无已保存预设";
    container.appendChild(emptyOption);
    for (const workspace of workspaces) {
      const option = document.createElement("option");
      option.value = workspace.name;
      option.textContent = workspace.name;
      option.selected = workspace.name === selectedName;
      container.appendChild(option);
    }
  }

  function readWorkspaceStore() {
    const text = localStorage.getItem(scheduler.WORKSPACE_STORAGE_KEY);
    if (!text) {
      return [];
    }
    const value = JSON.parse(text);
    if (!Array.isArray(value)) {
      throw new Error("浏览器预设存储格式无效");
    }
    return value;
  }

  function writeWorkspaceStore(workspaces) {
    localStorage.setItem(
      scheduler.WORKSPACE_STORAGE_KEY,
      JSON.stringify(workspaces),
    );
  }

  function currentWorkspaceInput() {
    return {
      name: elements["workspace-name"].value,
      selectedPoolIds: state.selectedPoolIds,
      filters: state.filters,
      schedule: readScheduleControls(),
    };
  }

  function saveWorkspace() {
    try {
      const workspace = scheduler.normalizeWorkspace(currentWorkspaceInput());
      const workspaces = readWorkspaceStore().filter(
        (saved) =>
          saved.name !== workspace.name &&
          saved.name !== state.activeWorkspaceName,
      );
      workspaces.push(workspace);
      workspaces.sort((left, right) => left.name.localeCompare(right.name, "zh-CN"));
      writeWorkspaceStore(workspaces);
      state.activeWorkspaceName = workspace.name;
      renderWorkspaceOptions(workspace.name);
      setStatus(`浏览器预设已保存：${workspace.name}`, "success");
    } catch (error) {
      setStatus(`保存失败：${error.message}`, "error");
    }
  }

  function applyWorkspace(workspace) {
    state.selectedPoolIds = [...workspace.selectedPoolIds];
    state.filters = {
      query: workspace.filters.query,
      idPrefix: workspace.filters.idPrefix,
      bannerFlags: [...workspace.filters.bannerFlags],
      types: [...workspace.filters.types],
      selection: workspace.filters.selection,
    };
    state.schedule = {
      mode: workspace.schedule.mode,
      startDate: workspace.schedule.startDate || shanghaiToday(),
      daysPerBatch: workspace.schedule.daysPerBatch || 7,
      poolsPerBatch: workspace.schedule.poolsPerBatch || 2,
      manualDates: JSON.parse(JSON.stringify(workspace.schedule.manualDates || {})),
    };
    elements["filter-query"].value = state.filters.query;
    elements["filter-id-prefix"].value = state.filters.idPrefix;
    elements["filter-selection"].value = state.filters.selection;
    elements["schedule-mode"].value = state.schedule.mode;
    elements["start-date"].value = state.schedule.startDate;
    elements["days-per-batch"].value = String(state.schedule.daysPerBatch);
    elements["pools-per-batch"].value = String(state.schedule.poolsPerBatch);
    elements["workspace-name"].value = workspace.name;
    populateDynamicFilters();
    renderAll();
  }

  function loadWorkspace() {
    try {
      if (state.pools.length === 0) {
        throw new Error("请先载入 summon_pool.json");
      }
      const name = elements["workspace-select"].value;
      const saved = readWorkspaceStore().find((workspace) => workspace.name === name);
      if (!saved) {
        throw new Error("请选择一个已保存预设");
      }
      const restored = scheduler.restoreWorkspace(JSON.stringify(saved), state.pools);
      if (restored.missingPoolIds.length > 0) {
        throw new Error(`当前源文件缺少池 ID：${restored.missingPoolIds.join(", ")}`);
      }
      state.activeWorkspaceName = restored.workspace.name;
      applyWorkspace(restored.workspace);
      setStatus(`已载入浏览器预设：${restored.workspace.name}`, "success");
    } catch (error) {
      setStatus(`载入预设失败：${error.message}`, "error");
    }
  }

  function deleteWorkspace() {
    try {
      const name = elements["workspace-select"].value;
      if (!name) {
        throw new Error("请选择要删除的预设");
      }
      writeWorkspaceStore(
        readWorkspaceStore().filter((workspace) => workspace.name !== name),
      );
      if (state.activeWorkspaceName === name) {
        state.activeWorkspaceName = "";
      }
      renderWorkspaceOptions();
      setStatus(`已删除浏览器预设：${name}`, "success");
    } catch (error) {
      setStatus(`删除失败：${error.message}`, "error");
    }
  }

  async function importPresetFile(file) {
    if (!file) {
      return;
    }
    try {
      if (state.pools.length === 0) {
        throw new Error("请先载入 summon_pool.json");
      }
      const preset = scheduler.parsePresetJson(await file.text(), state.pools);
      state.selectedPoolIds = preset.pools.map((pool) => pool.poolId);
      state.schedule.mode = preset.schedule.mode;
      state.schedule.startDate = preset.schedule.startDate || shanghaiToday();
      state.schedule.daysPerBatch = preset.schedule.daysPerBatch || 7;
      state.schedule.poolsPerBatch = preset.schedule.poolsPerBatch || 2;
      state.schedule.manualDates = {};
      for (const pool of preset.pools) {
        state.schedule.manualDates[pool.poolId] = {
          onlineDate: unixToShanghaiDate(pool.onlineTime),
          offlineDate: unixToShanghaiDate(pool.offlineTime),
        };
      }
      elements["preset-name"].value = preset.presetName;
      elements["schedule-mode"].value = state.schedule.mode;
      elements["start-date"].value = state.schedule.startDate;
      elements["days-per-batch"].value = String(state.schedule.daysPerBatch);
      elements["pools-per-batch"].value = String(state.schedule.poolsPerBatch);
      renderAll();
      setStatus(`已导入 ${preset.pools.length} 个卡池：${file.name}`, "success");
    } catch (error) {
      setStatus(`导入失败：${error.message}`, "error");
    } finally {
      elements["preset-file"].value = "";
    }
  }

  function exportPreset() {
    try {
      if (state.pools.length === 0) {
        throw new Error("请先载入 summon_pool.json");
      }
      const preset = scheduler.createPreset({
        presetName: elements["preset-name"].value,
        sourceFileName: state.sourceFileName,
        sourcePoolCount: state.pools.length,
        schedule: readScheduleControls(),
        pools: calculateCurrentSchedule(),
        generatedAt: new Date().toISOString(),
      });
      const blob = new Blob([`${JSON.stringify(preset, null, 2)}\n`], {
        type: "application/json;charset=utf-8",
      });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      const safeName = preset.presetName
        .trim()
        .replace(/[^\p{L}\p{N}._-]+/gu, "-")
        .replace(/^-+|-+$/g, "") || "banner-schedule";
      anchor.href = url;
      anchor.download = `${safeName}.json`;
      anchor.click();
      URL.revokeObjectURL(url);
      setStatus(`已导出 ${preset.pools.length} 个卡池：${anchor.download}`, "success");
    } catch (error) {
      setStatus(`导出失败：${error.message}`, "error");
    }
  }

  function handleQueueAction(button) {
    const poolId = Number(button.dataset.poolId);
    const action = button.dataset.action;
    if (action === "remove") {
      removePool(poolId);
    } else if (action === "up") {
      state.selectedPoolIds = scheduler.moveQueueItem(
        state.selectedPoolIds,
        poolId,
        -1,
      );
    } else if (action === "down") {
      state.selectedPoolIds = scheduler.moveQueueItem(
        state.selectedPoolIds,
        poolId,
        1,
      );
    }
    renderAll();
  }

  function bindEvents() {
    elements["source-file"].addEventListener("change", (event) => {
      loadSourceFile(event.target.files[0]);
    });
    for (const eventName of ["dragenter", "dragover"]) {
      elements["source-drop-zone"].addEventListener(eventName, (event) => {
        event.preventDefault();
        elements["source-drop-zone"].classList.add("is-dragging");
      });
    }
    for (const eventName of ["dragleave", "drop"]) {
      elements["source-drop-zone"].addEventListener(eventName, (event) => {
        event.preventDefault();
        elements["source-drop-zone"].classList.remove("is-dragging");
      });
    }
    elements["source-drop-zone"].addEventListener("drop", (event) => {
      loadSourceFile(event.dataTransfer.files[0]);
    });

    for (const button of document.querySelectorAll("button[data-preset]")) {
      button.addEventListener("click", () => applyPreset(button.dataset.preset));
    }

    elements["filter-query"].addEventListener("input", (event) => {
      state.filters.query = event.target.value;
      renderPoolList();
    });
    elements["filter-id-prefix"].addEventListener("input", (event) => {
      state.filters.idPrefix = event.target.value;
      renderPoolList();
    });
    elements["filter-selection"].addEventListener("change", (event) => {
      state.filters.selection = event.target.value;
      renderPoolList();
    });
    elements["filter-flags"].addEventListener("change", () => {
      updateFilterFromCheckboxes("bannerFlags");
    });
    elements["filter-types"].addEventListener("change", () => {
      updateFilterFromCheckboxes("types");
    });

    elements["pool-list"].addEventListener("change", (event) => {
      const checkbox = event.target.closest("input[data-pool-id]");
      if (!checkbox) {
        return;
      }
      const poolId = Number(checkbox.dataset.poolId);
      if (checkbox.checked) {
        selectPool(poolId);
      } else {
        removePool(poolId);
      }
      renderAll();
    });

    elements["select-visible"].addEventListener("click", () => {
      for (const pool of currentVisiblePools()) {
        selectPool(pool.id);
      }
      renderAll();
      setStatus(`当前已选择 ${state.selectedPoolIds.length} 个卡池`, "success");
    });
    elements["clear-visible"].addEventListener("click", () => {
      const visibleIds = new Set(currentVisiblePools().map((pool) => pool.id));
      state.selectedPoolIds = state.selectedPoolIds.filter(
        (poolId) => !visibleIds.has(poolId),
      );
      renderAll();
    });

    elements["selected-queue"].addEventListener("click", (event) => {
      const button = event.target.closest("button[data-action]");
      if (button) {
        handleQueueAction(button);
      }
    });
    elements["selected-queue"].addEventListener("change", (event) => {
      const input = event.target.closest("input[data-manual-role]");
      if (!input) {
        return;
      }
      const poolId = Number(input.dataset.poolId);
      ensureManualDates(poolId);
      const field =
        input.dataset.manualRole === "online" ? "onlineDate" : "offlineDate";
      state.schedule.manualDates[poolId][field] = input.value;
      renderSchedulePreview();
    });
    elements["selected-queue"].addEventListener("dragstart", (event) => {
      const row = event.target.closest("tr[data-pool-id]");
      if (!row) {
        return;
      }
      state.draggedPoolId = Number(row.dataset.poolId);
      row.classList.add("is-dragging");
      event.dataTransfer.effectAllowed = "move";
    });
    elements["selected-queue"].addEventListener("dragend", (event) => {
      event.target.closest("tr")?.classList.remove("is-dragging");
      state.draggedPoolId = null;
    });
    elements["selected-queue"].addEventListener("dragover", (event) => {
      if (event.target.closest("tr[data-pool-id]")) {
        event.preventDefault();
      }
    });
    elements["selected-queue"].addEventListener("drop", (event) => {
      event.preventDefault();
      const targetRow = event.target.closest("tr[data-pool-id]");
      const targetPoolId = Number(targetRow?.dataset.poolId);
      if (!state.draggedPoolId || !targetPoolId || state.draggedPoolId === targetPoolId) {
        return;
      }
      const reordered = state.selectedPoolIds.filter(
        (poolId) => poolId !== state.draggedPoolId,
      );
      const targetIndex = reordered.indexOf(targetPoolId);
      reordered.splice(targetIndex, 0, state.draggedPoolId);
      state.selectedPoolIds = reordered;
      renderAll();
    });

    elements["queue-sort"].addEventListener("change", (event) => {
      state.selectedPoolIds = scheduler.sortQueue(
        state.selectedPoolIds,
        state.pools,
        event.target.value,
      );
      renderAll();
    });
    elements["clear-queue"].addEventListener("click", () => {
      state.selectedPoolIds = [];
      renderAll();
    });

    for (const id of [
      "schedule-mode",
      "start-date",
      "days-per-batch",
      "pools-per-batch",
    ]) {
      elements[id].addEventListener("change", () => {
        readScheduleControls();
        for (const poolId of state.selectedPoolIds) {
          ensureManualDates(poolId);
        }
        renderAll();
      });
    }

    elements["save-workspace"].addEventListener("click", saveWorkspace);
    elements["load-workspace"].addEventListener("click", loadWorkspace);
    elements["delete-workspace"].addEventListener("click", deleteWorkspace);
    elements["preset-file"].addEventListener("change", (event) => {
      importPresetFile(event.target.files[0]);
    });
    elements["export-preset"].addEventListener("click", exportPreset);
  }
})();
