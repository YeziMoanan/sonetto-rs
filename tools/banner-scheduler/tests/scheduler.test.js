const test = require("node:test");
const assert = require("node:assert/strict");
const scheduler = require("../scheduler.js");

const sourcePools = [
  { id: 1, nameEn: "Permanent", bannerFlag: 0, type: 1, priority: 99 },
  { id: 20, nameEn: "Activity", bannerFlag: 2, type: 3, priority: 20 },
  { id: 30, nameEn: "Limited", bannerFlag: 3, type: 3, priority: 30 },
  { id: 40, nameEn: "Rerun Four", bannerFlag: 4, type: 4, priority: 40 },
  { id: 50, nameEn: "Collaboration", bannerFlag: 5, type: 5, priority: 50 },
  { id: 60, nameEn: "Rerun Six", bannerFlag: 6, type: 6, priority: 60 },
];

function sourceJson(pools = sourcePools) {
  return JSON.stringify(["summon_pool", pools]);
}

test("parses and normalizes the summon_pool tuple", () => {
  assert.deepEqual(scheduler.parseSourceJson(sourceJson()), sourcePools);
});

test("rejects malformed source tuples", () => {
  assert.throws(() => scheduler.parseSourceJson("{}"), /summon_pool/i);
  assert.throws(() => scheduler.parseSourceJson("not json"), /json/i);
});

test("rejects duplicate and invalid pool IDs", () => {
  assert.throws(
    () => scheduler.parseSourceJson(sourceJson([{ id: 1 }, { id: 1 }])),
    /duplicate/i,
  );
  assert.throws(
    () => scheduler.parseSourceJson(sourceJson([{ id: 0 }])),
    /positive/i,
  );
});

test("selects every built-in bannerFlag preset", () => {
  assert.deepEqual(
    scheduler.selectPreset(sourcePools, "all"),
    [1, 20, 30, 40, 50, 60],
  );
  assert.deepEqual(scheduler.selectPreset(sourcePools, "collaboration"), [50]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "rerun"), [40, 60]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "activity"), [20]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "limited"), [30]);
  assert.throws(() => scheduler.selectPreset(sourcePools, "missing"), /unknown/i);
});

test("combines name, ID prefix, flags, types, and selection filters", () => {
  const visible = scheduler.filterPools(
    sourcePools,
    {
      query: "activity",
      idPrefix: "2",
      bannerFlags: [2],
      types: [3],
      selection: "selected",
    },
    new Set([20, 50]),
  );
  assert.deepEqual(
    visible.map((pool) => pool.id),
    [20],
  );

  assert.deepEqual(
    scheduler
      .filterPools(
        sourcePools,
        {
          query: "0",
          idPrefix: "",
          bannerFlags: [],
          types: [],
          selection: "unselected",
        },
        new Set([20, 50]),
      )
      .map((pool) => pool.id),
    [30, 40, 60],
  );
});

test("moves queue IDs without changing membership", () => {
  assert.deepEqual(
    scheduler.moveQueueItem([30, 20, 10], 20, -1),
    [20, 30, 10],
  );
  assert.deepEqual(
    scheduler.moveQueueItem([30, 20, 10], 30, -1),
    [30, 20, 10],
  );
  assert.deepEqual(
    scheduler.moveQueueItem([30, 20, 10], 10, 1),
    [30, 20, 10],
  );
  assert.throws(
    () => scheduler.moveQueueItem([30, 20, 10], 99, 1),
    /unknown/i,
  );
});

test("sorts queues deterministically", () => {
  const pools = [
    { id: 10, priority: 3 },
    { id: 20, priority: 1 },
    { id: 30, priority: 2 },
  ];
  assert.deepEqual(
    scheduler.sortQueue([30, 10, 20], pools, "priority"),
    [20, 30, 10],
  );
  assert.deepEqual(
    scheduler.sortQueue([30, 10, 20], pools, "id"),
    [10, 20, 30],
  );
  assert.deepEqual(
    scheduler.sortQueue([30, 10, 20], pools, "source"),
    [10, 20, 30],
  );
  assert.throws(
    () => scheduler.sortQueue([99], pools, "id"),
    /unknown/i,
  );
});

test("converts Shanghai calendar dates independently of host timezone", () => {
  assert.equal(scheduler.shanghaiDateToUnix("2026-07-15"), 1784044800);
  assert.throws(() => scheduler.shanghaiDateToUnix("2026-02-30"), /date/i);
  assert.throws(() => scheduler.shanghaiDateToUnix("15-07-2026"), /date/i);
});

test("generates simultaneous schedules", () => {
  assert.deepEqual(
    scheduler.buildSchedule([50, 40], {
      mode: "simultaneous",
      startDate: "2026-07-15",
    }),
    [
      {
        poolId: 50,
        order: 0,
        onlineTime: 1784044800,
        offlineTime: 2147483647,
      },
      {
        poolId: 40,
        order: 1,
        onlineTime: 1784044800,
        offlineTime: 2147483647,
      },
    ],
  );
});

test("generates rotating batch schedules", () => {
  const batch = scheduler.buildSchedule([50, 40, 30], {
    mode: "batch",
    startDate: "2026-07-15",
    daysPerBatch: 7,
    poolsPerBatch: 2,
  });
  assert.deepEqual(
    batch.map(({ onlineTime, offlineTime }) => [onlineTime, offlineTime]),
    [
      [1784044800, 1784649600],
      [1784044800, 1784649600],
      [1784649600, 1785254400],
    ],
  );
});

test("keeps manual dates attached to pool IDs after reordering", () => {
  const pools = scheduler.buildSchedule([40, 50], {
    mode: "manual",
    manualDates: {
      50: { onlineDate: "2026-07-15", offlineDate: "2026-07-16" },
      40: { onlineDate: "2026-08-01", offlineDate: "2026-08-03" },
    },
  });
  assert.equal(pools[0].poolId, 40);
  assert.equal(
    pools[0].onlineTime,
    scheduler.shanghaiDateToUnix("2026-08-01"),
  );
});

test("rejects invalid schedule inputs", () => {
  assert.throws(
    () => scheduler.buildSchedule([], { mode: "simultaneous", startDate: "2026-07-15" }),
    /select/i,
  );
  assert.throws(
    () => scheduler.buildSchedule([50, 50], { mode: "simultaneous", startDate: "2026-07-15" }),
    /duplicate/i,
  );
  assert.throws(
    () => scheduler.buildSchedule([50], {
      mode: "batch",
      startDate: "2026-07-15",
      daysPerBatch: 0,
      poolsPerBatch: 2,
    }),
    /positive/i,
  );
  assert.throws(
    () => scheduler.buildSchedule([50], {
      mode: "manual",
      manualDates: {
        50: { onlineDate: "2026-07-15", offlineDate: "2026-07-15" },
      },
    }),
    /later/i,
  );
});

function makeExportedPreset() {
  const schedule = {
    mode: "batch",
    startDate: "2026-07-15",
    daysPerBatch: 7,
    poolsPerBatch: 2,
  };
  return scheduler.createPreset({
    presetName: "Collaboration rotation",
    sourceFileName: "summon_pool.json",
    sourcePoolCount: sourcePools.length,
    schedule,
    pools: scheduler.buildSchedule([50, 40], schedule),
    generatedAt: "2026-07-15T03:00:00.000Z",
  });
}

test("creates the documented version 1 preset", () => {
  assert.deepEqual(makeExportedPreset(), {
    schemaVersion: 1,
    presetName: "Collaboration rotation",
    generatedAt: "2026-07-15T03:00:00.000Z",
    source: {
      fileName: "summon_pool.json",
      poolCount: 6,
    },
    schedule: {
      mode: "batch",
      timezone: "Asia/Shanghai",
      startDate: "2026-07-15",
      daysPerBatch: 7,
      poolsPerBatch: 2,
    },
    pools: [
      {
        poolId: 50,
        order: 0,
        onlineTime: 1784044800,
        offlineTime: 1784649600,
      },
      {
        poolId: 40,
        order: 1,
        onlineTime: 1784044800,
        offlineTime: 1784649600,
      },
    ],
  });
});

test("round-trips exported presets without sharing mutable input", () => {
  const preset = makeExportedPreset();
  const parsed = scheduler.parsePresetJson(JSON.stringify(preset), sourcePools);
  assert.deepEqual(parsed, preset);
  parsed.pools[0].poolId = 1;
  assert.equal(preset.pools[0].poolId, 50);
});

test("rejects invalid exported preset contracts", () => {
  const preset = makeExportedPreset();
  const parseChanged = (change) => {
    const changed = JSON.parse(JSON.stringify(preset));
    change(changed);
    return () => scheduler.parsePresetJson(JSON.stringify(changed), sourcePools);
  };

  assert.throws(parseChanged((value) => { value.schemaVersion = 2; }), /version/i);
  assert.throws(parseChanged((value) => { value.schedule.timezone = "UTC"; }), /timezone/i);
  assert.throws(parseChanged((value) => { value.pools[1].poolId = 50; }), /duplicate/i);
  assert.throws(parseChanged((value) => { value.pools[1].order = 3; }), /order/i);
  assert.throws(parseChanged((value) => { value.pools[1].poolId = 999; }), /unknown/i);
  assert.throws(parseChanged((value) => { value.pools[0].onlineTime = 1.5; }), /integer/i);
  assert.throws(parseChanged((value) => { value.pools[0].offlineTime = value.pools[0].onlineTime; }), /later/i);
  assert.throws(parseChanged((value) => { value.pools = []; }), /pool/i);
});

test("normalizes browser workspaces without embedding source rows", () => {
  const workspace = scheduler.normalizeWorkspace({
    name: "My rotation",
    selectedPoolIds: [50, 40],
    filters: {
      query: "collab",
      idPrefix: "",
      bannerFlags: [5],
      types: [],
      selection: "all",
    },
    schedule: {
      mode: "batch",
      startDate: "2026-07-15",
      daysPerBatch: 7,
      poolsPerBatch: 2,
      manualDates: {},
    },
  });

  assert.deepEqual(workspace, {
    schemaVersion: 1,
    name: "My rotation",
    selectedPoolIds: [50, 40],
    filters: {
      query: "collab",
      idPrefix: "",
      bannerFlags: [5],
      types: [],
      selection: "all",
    },
    schedule: {
      mode: "batch",
      startDate: "2026-07-15",
      daysPerBatch: 7,
      poolsPerBatch: 2,
      manualDates: {},
    },
  });
  assert.doesNotMatch(JSON.stringify(workspace), /nameEn|priority/);
  assert.equal(scheduler.WORKSPACE_STORAGE_KEY, "banner-scheduler.workspace.v1");
});

test("reports missing source IDs when restoring a workspace", () => {
  const saved = {
    schemaVersion: 1,
    name: "Old source",
    selectedPoolIds: [50, 999],
    filters: {
      query: "",
      idPrefix: "",
      bannerFlags: [],
      types: [],
      selection: "all",
    },
    schedule: {
      mode: "simultaneous",
      startDate: "2026-07-15",
      daysPerBatch: null,
      poolsPerBatch: null,
      manualDates: {},
    },
  };
  const restored = scheduler.restoreWorkspace(JSON.stringify(saved), sourcePools);
  assert.deepEqual(restored.missingPoolIds, [999]);
  assert.deepEqual(restored.workspace.selectedPoolIds, [50, 999]);
});

test("rejects invalid browser workspaces", () => {
  assert.throws(
    () => scheduler.normalizeWorkspace({
      name: "Duplicate",
      selectedPoolIds: [50, 50],
      filters: {},
      schedule: { mode: "simultaneous", startDate: "2026-07-15" },
    }),
    /duplicate/i,
  );
  assert.throws(
    () => scheduler.restoreWorkspace("{}", sourcePools),
    /version/i,
  );
});
