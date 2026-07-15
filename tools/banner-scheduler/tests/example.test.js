const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const scheduler = require("../scheduler.js");

test("ships a valid collaboration preset example", () => {
  const examplePath = path.join(
    __dirname,
    "..",
    "examples",
    "all-collaboration.json",
  );
  const preset = scheduler.parsePresetJson(
    fs.readFileSync(examplePath, "utf8"),
    [{ id: 305121 }, { id: 305111 }],
  );
  assert.equal(preset.schemaVersion, 1);
  assert.equal(preset.schedule.timezone, "Asia/Shanghai");
  assert.equal(preset.source.poolCount, 210);
  assert.deepEqual(
    preset.pools.map((pool) => pool.poolId),
    [305121, 305111],
  );
});
