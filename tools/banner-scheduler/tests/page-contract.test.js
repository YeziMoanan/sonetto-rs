const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const pagePath = path.join(__dirname, "..", "index.html");

test("provides the complete local page contract", () => {
  const html = fs.readFileSync(pagePath, "utf8");
  const requiredIds = [
    "source-file",
    "status",
    "filter-query",
    "filter-id-prefix",
    "filter-flags",
    "filter-types",
    "filter-selection",
    "pool-list",
    "selected-queue",
    "queue-sort",
    "schedule-mode",
    "start-date",
    "days-per-batch",
    "pools-per-batch",
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

  assert.match(html, /<main\b/i);
  for (const id of requiredIds) {
    assert.match(html, new RegExp(`id=["']${id}["']`, "i"));
  }
  for (const preset of ["all", "collaboration", "rerun", "activity", "limited"]) {
    assert.match(html, new RegExp(`data-preset=["']${preset}["']`, "i"));
  }

  assert.match(html, /href=["']styles\.css["']/i);
  assert.match(html, /src=["']scheduler\.js["'][^>]*defer/i);
  assert.match(html, /src=["']app\.js["'][^>]*defer/i);
  assert.doesNotMatch(html, /(?:src|href)=["']https?:/i);
});
