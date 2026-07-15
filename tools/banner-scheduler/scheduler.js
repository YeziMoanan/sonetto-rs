(function exposeScheduler(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.BannerScheduler = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function createScheduler() {
  "use strict";

  const WORKSPACE_STORAGE_KEY = "banner-scheduler.workspace.v1";
  const PRESET_RULES = Object.freeze({
    all: () => true,
    collaboration: (pool) => pool.bannerFlag === 5,
    rerun: (pool) => pool.bannerFlag === 4 || pool.bannerFlag === 6,
    activity: (pool) => pool.bannerFlag === 2,
    limited: (pool) => pool.bannerFlag === 3,
  });

  function parseSourceJson(text) {
    let value;
    try {
      value = JSON.parse(text);
    } catch (error) {
      throw new Error(`Invalid JSON: ${error.message}`);
    }

    if (
      !Array.isArray(value) ||
      value[0] !== "summon_pool" ||
      !Array.isArray(value[1])
    ) {
      throw new Error("Source must be a summon_pool tuple");
    }

    const seen = new Set();
    return value[1].map((pool) => {
      if (!Number.isSafeInteger(pool.id) || pool.id <= 0) {
        throw new Error("Pool ID must be a positive safe integer");
      }
      if (seen.has(pool.id)) {
        throw new Error(`Duplicate pool ID: ${pool.id}`);
      }
      seen.add(pool.id);
      return {
        id: pool.id,
        nameEn: typeof pool.nameEn === "string" ? pool.nameEn : "",
        bannerFlag: Number.isSafeInteger(pool.bannerFlag) ? pool.bannerFlag : 0,
        type: Number.isSafeInteger(pool.type) ? pool.type : 0,
        priority: Number.isSafeInteger(pool.priority) ? pool.priority : 0,
      };
    });
  }

  function selectPreset(pools, key) {
    const rule = PRESET_RULES[key];
    if (!rule) {
      throw new Error(`Unknown preset: ${key}`);
    }
    return pools.filter(rule).map((pool) => pool.id);
  }

  function filterPools(pools, criteria = {}, selectedIds = new Set()) {
    const query = String(criteria.query || "").trim().toLowerCase();
    const idPrefix = String(criteria.idPrefix || "").trim();
    const bannerFlags = new Set(criteria.bannerFlags || []);
    const types = new Set(criteria.types || []);
    const selection = criteria.selection || "all";

    if (!new Set(["all", "selected", "unselected"]).has(selection)) {
      throw new Error(`Unknown selection filter: ${selection}`);
    }

    return pools.filter((pool) => {
      const decimalId = String(pool.id);
      const matchesQuery =
        !query ||
        pool.nameEn.toLowerCase().includes(query) ||
        decimalId.includes(query);
      const matchesPrefix = !idPrefix || decimalId.startsWith(idPrefix);
      const matchesFlag =
        bannerFlags.size === 0 || bannerFlags.has(pool.bannerFlag);
      const matchesType = types.size === 0 || types.has(pool.type);
      const isSelected = selectedIds.has(pool.id);
      const matchesSelection =
        selection === "all" ||
        (selection === "selected" && isSelected) ||
        (selection === "unselected" && !isSelected);
      return (
        matchesQuery &&
        matchesPrefix &&
        matchesFlag &&
        matchesType &&
        matchesSelection
      );
    });
  }

  function moveQueueItem(queueIds, poolId, direction) {
    if (direction !== -1 && direction !== 1) {
      throw new Error("Queue direction must be -1 or 1");
    }
    const currentIndex = queueIds.indexOf(poolId);
    if (currentIndex === -1) {
      throw new Error(`Unknown queue pool ID: ${poolId}`);
    }
    const nextIndex = currentIndex + direction;
    if (nextIndex < 0 || nextIndex >= queueIds.length) {
      return [...queueIds];
    }
    const result = [...queueIds];
    [result[currentIndex], result[nextIndex]] = [
      result[nextIndex],
      result[currentIndex],
    ];
    return result;
  }

  function sortQueue(queueIds, pools, mode) {
    const sourceIndex = new Map(
      pools.map((pool, index) => [pool.id, { pool, index }]),
    );
    for (const poolId of queueIds) {
      if (!sourceIndex.has(poolId)) {
        throw new Error(`Unknown queue pool ID: ${poolId}`);
      }
    }

    const comparisons = {
      source: (left, right) => left.index - right.index,
      id: (left, right) => left.pool.id - right.pool.id,
      priority: (left, right) => left.pool.priority - right.pool.priority,
    };
    const comparison = comparisons[mode];
    if (!comparison) {
      throw new Error(`Unknown queue sort: ${mode}`);
    }

    return [...queueIds].sort((leftId, rightId) => {
      const left = sourceIndex.get(leftId);
      const right = sourceIndex.get(rightId);
      return comparison(left, right) || left.index - right.index;
    });
  }

  function shanghaiDateToUnix(dateText) {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(dateText));
    if (!match) {
      throw new Error(`Invalid Shanghai date: ${dateText}`);
    }
    const year = Number(match[1]);
    const month = Number(match[2]);
    const day = Number(match[3]);
    const calendarCheck = new Date(Date.UTC(year, month - 1, day));
    if (
      calendarCheck.getUTCFullYear() !== year ||
      calendarCheck.getUTCMonth() !== month - 1 ||
      calendarCheck.getUTCDate() !== day
    ) {
      throw new Error(`Invalid Shanghai date: ${dateText}`);
    }
    return Date.UTC(year, month - 1, day, -8, 0, 0) / 1000;
  }

  function validateQueueIds(queueIds) {
    if (!Array.isArray(queueIds) || queueIds.length === 0) {
      throw new Error("Select at least one summon pool");
    }
    const seen = new Set();
    for (const poolId of queueIds) {
      if (!Number.isSafeInteger(poolId) || poolId <= 0) {
        throw new Error("Pool ID must be a positive safe integer");
      }
      if (seen.has(poolId)) {
        throw new Error(`Duplicate pool ID: ${poolId}`);
      }
      seen.add(poolId);
    }
  }

  function assertScheduleTime(timestamp, label) {
    if (
      !Number.isSafeInteger(timestamp) ||
      timestamp < 0 ||
      timestamp > 2147483647
    ) {
      throw new Error(`${label} is outside the supported Unix time range`);
    }
  }

  function buildSchedule(queueIds, settings) {
    validateQueueIds(queueIds);
    if (!settings || typeof settings !== "object") {
      throw new Error("Schedule settings are required");
    }

    if (settings.mode === "simultaneous") {
      const onlineTime = shanghaiDateToUnix(settings.startDate);
      assertScheduleTime(onlineTime, "Online time");
      return queueIds.map((poolId, order) => ({
        poolId,
        order,
        onlineTime,
        offlineTime: 2147483647,
      }));
    }

    if (settings.mode === "batch") {
      if (
        !Number.isSafeInteger(settings.daysPerBatch) ||
        settings.daysPerBatch <= 0 ||
        !Number.isSafeInteger(settings.poolsPerBatch) ||
        settings.poolsPerBatch <= 0
      ) {
        throw new Error("Batch values must be positive integers");
      }
      const startTime = shanghaiDateToUnix(settings.startDate);
      return queueIds.map((poolId, order) => {
        const batchIndex = Math.floor(order / settings.poolsPerBatch);
        const onlineTime =
          startTime + batchIndex * settings.daysPerBatch * 86400;
        const offlineTime = onlineTime + settings.daysPerBatch * 86400;
        assertScheduleTime(onlineTime, "Online time");
        assertScheduleTime(offlineTime, "Offline time");
        return { poolId, order, onlineTime, offlineTime };
      });
    }

    if (settings.mode === "manual") {
      const manualDates = settings.manualDates || {};
      return queueIds.map((poolId, order) => {
        const dates = manualDates[poolId];
        if (!dates) {
          throw new Error(`Manual dates are required for pool ${poolId}`);
        }
        const onlineTime = shanghaiDateToUnix(dates.onlineDate);
        const offlineTime = shanghaiDateToUnix(dates.offlineDate);
        assertScheduleTime(onlineTime, "Online time");
        assertScheduleTime(offlineTime, "Offline time");
        if (offlineTime <= onlineTime) {
          throw new Error(`Offline date must be later for pool ${poolId}`);
        }
        return { poolId, order, onlineTime, offlineTime };
      });
    }

    throw new Error(`Unknown schedule mode: ${settings.mode}`);
  }

  function normalizePresetSchedule(schedule, requireTimezone) {
    if (!schedule || typeof schedule !== "object") {
      throw new Error("Preset schedule is required");
    }
    if (requireTimezone && schedule.timezone !== "Asia/Shanghai") {
      throw new Error("Preset timezone must be Asia/Shanghai");
    }
    if (!new Set(["simultaneous", "batch", "manual"]).has(schedule.mode)) {
      throw new Error(`Unknown schedule mode: ${schedule.mode}`);
    }

    if (schedule.mode === "manual") {
      return {
        mode: "manual",
        timezone: "Asia/Shanghai",
        startDate: null,
        daysPerBatch: null,
        poolsPerBatch: null,
      };
    }

    shanghaiDateToUnix(schedule.startDate);
    if (schedule.mode === "simultaneous") {
      return {
        mode: "simultaneous",
        timezone: "Asia/Shanghai",
        startDate: schedule.startDate,
        daysPerBatch: null,
        poolsPerBatch: null,
      };
    }

    if (
      !Number.isSafeInteger(schedule.daysPerBatch) ||
      schedule.daysPerBatch <= 0 ||
      !Number.isSafeInteger(schedule.poolsPerBatch) ||
      schedule.poolsPerBatch <= 0
    ) {
      throw new Error("Batch values must be positive integers");
    }
    return {
      mode: "batch",
      timezone: "Asia/Shanghai",
      startDate: schedule.startDate,
      daysPerBatch: schedule.daysPerBatch,
      poolsPerBatch: schedule.poolsPerBatch,
    };
  }

  function normalizePresetPools(pools, availablePools) {
    if (!Array.isArray(pools) || pools.length === 0) {
      throw new Error("Preset must contain at least one pool");
    }
    const availableIds = availablePools
      ? new Set(availablePools.map((pool) => pool.id))
      : null;
    const seen = new Set();
    return pools.map((pool, index) => {
      if (!pool || typeof pool !== "object") {
        throw new Error(`Invalid preset pool at order ${index}`);
      }
      if (!Number.isSafeInteger(pool.poolId) || pool.poolId <= 0) {
        throw new Error("Preset pool ID must be a positive integer");
      }
      if (seen.has(pool.poolId)) {
        throw new Error(`Duplicate preset pool ID: ${pool.poolId}`);
      }
      seen.add(pool.poolId);
      if (pool.order !== index) {
        throw new Error(`Preset pool order must be contiguous at ${index}`);
      }
      if (availableIds && !availableIds.has(pool.poolId)) {
        throw new Error(`Unknown source pool ID: ${pool.poolId}`);
      }
      if (
        !Number.isSafeInteger(pool.onlineTime) ||
        !Number.isSafeInteger(pool.offlineTime)
      ) {
        throw new Error("Preset timestamps must be integers");
      }
      assertScheduleTime(pool.onlineTime, "Online time");
      assertScheduleTime(pool.offlineTime, "Offline time");
      if (pool.offlineTime <= pool.onlineTime) {
        throw new Error(`Offline time must be later for pool ${pool.poolId}`);
      }
      return {
        poolId: pool.poolId,
        order: index,
        onlineTime: pool.onlineTime,
        offlineTime: pool.offlineTime,
      };
    });
  }

  function createPreset({
    presetName,
    sourceFileName,
    sourcePoolCount,
    schedule,
    pools,
    generatedAt,
  }) {
    const normalizedName = String(presetName || "").trim();
    const normalizedFileName = String(sourceFileName || "").trim();
    if (!normalizedName) {
      throw new Error("Preset name is required");
    }
    if (!normalizedFileName) {
      throw new Error("Source file name is required");
    }
    if (!Number.isSafeInteger(sourcePoolCount) || sourcePoolCount <= 0) {
      throw new Error("Source pool count must be a positive integer");
    }
    if (typeof generatedAt !== "string" || !Number.isFinite(Date.parse(generatedAt))) {
      throw new Error("Generated timestamp must be an ISO date");
    }

    return {
      schemaVersion: 1,
      presetName: normalizedName,
      generatedAt: new Date(generatedAt).toISOString(),
      source: {
        fileName: normalizedFileName,
        poolCount: sourcePoolCount,
      },
      schedule: normalizePresetSchedule(schedule, false),
      pools: normalizePresetPools(pools),
    };
  }

  function parsePresetJson(text, availablePools) {
    let value;
    try {
      value = JSON.parse(text);
    } catch (error) {
      throw new Error(`Invalid preset JSON: ${error.message}`);
    }
    if (!value || typeof value !== "object" || value.schemaVersion !== 1) {
      throw new Error(`Unsupported preset schema version: ${value?.schemaVersion}`);
    }
    if (!value.source || typeof value.source !== "object") {
      throw new Error("Preset source metadata is required");
    }

    const preset = createPreset({
      presetName: value.presetName,
      sourceFileName: value.source.fileName,
      sourcePoolCount: value.source.poolCount,
      schedule: normalizePresetSchedule(value.schedule, true),
      pools: normalizePresetPools(value.pools, availablePools),
      generatedAt: value.generatedAt,
    });
    return preset;
  }

  function normalizeIntegerList(values, label) {
    if (!Array.isArray(values)) {
      throw new Error(`${label} must be an array`);
    }
    const seen = new Set();
    return values.map((value) => {
      if (!Number.isSafeInteger(value)) {
        throw new Error(`${label} must contain integers`);
      }
      if (seen.has(value)) {
        throw new Error(`Duplicate ${label} value: ${value}`);
      }
      seen.add(value);
      return value;
    });
  }

  function normalizeWorkspace(workspace) {
    if (!workspace || typeof workspace !== "object") {
      throw new Error("Workspace is required");
    }
    const name = String(workspace.name || "").trim();
    if (!name) {
      throw new Error("Workspace name is required");
    }
    const selectedPoolIds = normalizeIntegerList(
      workspace.selectedPoolIds,
      "selected pool ID",
    );
    validateQueueIds(selectedPoolIds);

    const inputFilters = workspace.filters || {};
    const selection = inputFilters.selection || "all";
    if (!new Set(["all", "selected", "unselected"]).has(selection)) {
      throw new Error(`Unknown selection filter: ${selection}`);
    }
    const filters = {
      query: String(inputFilters.query || ""),
      idPrefix: String(inputFilters.idPrefix || ""),
      bannerFlags: normalizeIntegerList(inputFilters.bannerFlags || [], "bannerFlag"),
      types: normalizeIntegerList(inputFilters.types || [], "type"),
      selection,
    };

    const inputSchedule = workspace.schedule || {};
    buildSchedule(selectedPoolIds, inputSchedule);
    const manualDates = {};
    if (inputSchedule.mode === "manual") {
      for (const poolId of selectedPoolIds) {
        const dates = inputSchedule.manualDates[poolId];
        manualDates[poolId] = {
          onlineDate: dates.onlineDate,
          offlineDate: dates.offlineDate,
        };
      }
    }
    const schedule = {
      mode: inputSchedule.mode,
      startDate:
        inputSchedule.mode === "manual" ? null : inputSchedule.startDate,
      daysPerBatch:
        inputSchedule.mode === "batch" ? inputSchedule.daysPerBatch : null,
      poolsPerBatch:
        inputSchedule.mode === "batch" ? inputSchedule.poolsPerBatch : null,
      manualDates,
    };

    return {
      schemaVersion: 1,
      name,
      selectedPoolIds,
      filters,
      schedule,
    };
  }

  function restoreWorkspace(text, availablePools) {
    let value;
    try {
      value = JSON.parse(text);
    } catch (error) {
      throw new Error(`Invalid workspace JSON: ${error.message}`);
    }
    if (!value || typeof value !== "object" || value.schemaVersion !== 1) {
      throw new Error(`Unsupported workspace schema version: ${value?.schemaVersion}`);
    }
    const workspace = normalizeWorkspace(value);
    const availableIds = new Set(availablePools.map((pool) => pool.id));
    return {
      workspace,
      missingPoolIds: workspace.selectedPoolIds.filter(
        (poolId) => !availableIds.has(poolId),
      ),
    };
  }

  return Object.freeze({
    WORKSPACE_STORAGE_KEY,
    buildSchedule,
    createPreset,
    filterPools,
    moveQueueItem,
    normalizeWorkspace,
    parseSourceJson,
    restoreWorkspace,
    parsePresetJson,
    selectPreset,
    shanghaiDateToUnix,
    sortQueue,
  });
});
