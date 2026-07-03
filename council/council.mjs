#!/usr/bin/env node
var __defProp = Object.defineProperty;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __esm = (fn, res, err) => function __init() {
  if (err) throw err[0];
  try {
    return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res;
  } catch (e) {
    throw err = [e], e;
  }
};
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};

// src/domain/runstore/index.ts
var init_runstore = __esm({
  "src/domain/runstore/index.ts"() {
    "use strict";
  }
});

// src/domain/graph/index.ts
import { createHash } from "node:crypto";
function createTaskGraph(drafts, options = {}) {
  const idStrategy = options.idStrategy ?? "legacy-ordinal";
  const nodes = /* @__PURE__ */ new Map();
  let nextOrdinal = nextOrdinalAfter(drafts);
  drafts.forEach((draft, order) => {
    const contentHash = taskContentHash(draft);
    const minted = draft.id ?? mintTaskId(contentHash, idStrategy, nodes, nextOrdinal);
    nextOrdinal = nextOrdinalFor(minted, nextOrdinal);
    if (nodes.has(minted)) {
      throw new Error(`duplicate task id: ${minted}`);
    }
    nodes.set(minted, {
      blocked_by: [],
      order,
      state: "pending",
      task: normalizeTask(draft, minted, contentHash)
    });
  });
  const graph = graphFromNodes(nodes, idStrategy, nextOrdinal);
  assertKnownDependencies(graph);
  assertAcyclic(graph);
  return graph;
}
function projectWaveView(graph) {
  const remaining = /* @__PURE__ */ new Map();
  for (const node of graph.nodes.values()) {
    remaining.set(node.task.id, node.task.depends_on.filter((dependency) => graph.nodes.has(dependency)));
  }
  const waves = [];
  const done = /* @__PURE__ */ new Set();
  while (remaining.size > 0) {
    const ready = [...remaining.entries()].filter(([, dependencies]) => dependencies.every((dependency) => done.has(dependency))).map(([id]) => id).sort();
    if (ready.length === 0) {
      throw new Error(`dependency cycle among tasks: ${[...remaining.keys()].sort().join(", ")}`);
    }
    waves.push(ready);
    ready.forEach((id) => {
      done.add(id);
      remaining.delete(id);
    });
  }
  return waves;
}
function planWaves(drafts, options = {}) {
  return projectWaveView(createTaskGraph(drafts, options));
}
function graphFromNodes(nodes, idStrategy, nextOrdinal) {
  return {
    contentIndex: contentIndexFor(nodes),
    edges: edgesFor(nodes),
    idStrategy,
    nextOrdinal,
    nodes
  };
}
function normalizeTask(draft, id, contentHash) {
  return {
    ...draft,
    content_hash: contentHash,
    depends_on: draft.depends_on ?? [],
    id
  };
}
function taskContentHash(draft) {
  return draft.content_hash ?? digest(stableJson(stripIdentity(draft)));
}
function stripIdentity(draft) {
  const content = { ...draft };
  delete content.content_hash;
  delete content.id;
  return content;
}
function stableJson(value) {
  if (value === null || typeof value !== "object") {
    const serialized = JSON.stringify(value);
    return typeof serialized === "string" ? serialized : "undefined";
  }
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableJson(entry)).join(",")}]`;
  }
  const record = value;
  return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${stableJson(record[key])}`).join(",")}}`;
}
function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}
function mintTaskId(contentHash, idStrategy, nodes, nextOrdinal) {
  if (idStrategy === "content-hash") {
    let suffix = 12;
    let candidate = `ck-${contentHash.slice(0, suffix)}`;
    let collision = 2;
    while (nodes.has(candidate)) {
      if (suffix < contentHash.length) {
        suffix += 1;
        candidate = `ck-${contentHash.slice(0, suffix)}`;
      } else {
        candidate = `ck-${contentHash}-${String(collision)}`;
        collision += 1;
      }
    }
    return candidate;
  }
  return ordinalTaskId(nextOrdinal);
}
function ordinalTaskId(nextOrdinal) {
  return `T${String(nextOrdinal)}`;
}
function nextOrdinalAfter(drafts) {
  return drafts.reduce((next, draft) => {
    if (draft.id?.startsWith("T") !== true) {
      return next;
    }
    const ordinal = Number.parseInt(draft.id.slice(1), 10);
    return Number.isFinite(ordinal) ? Math.max(next, ordinal + 1) : next;
  }, 1);
}
function nextOrdinalFor(id, nextOrdinal) {
  if (!id.startsWith("T")) {
    return nextOrdinal;
  }
  const ordinal = Number.parseInt(id.slice(1), 10);
  return Number.isFinite(ordinal) ? Math.max(nextOrdinal, ordinal + 1) : nextOrdinal;
}
function edgesFor(nodes) {
  const edges = [];
  for (const node of nodes.values()) {
    node.task.depends_on.forEach((dependency) => {
      if (nodes.has(dependency)) {
        edges.push({ from: dependency, kind: "depends_on", to: node.task.id });
      }
    });
    if (isKnownTaskId(nodes, node.task.discovered_from)) {
      edges.push({ from: node.task.discovered_from, kind: "discovered_from", to: node.task.id });
    }
    node.task.supersedes?.forEach((superseded) => {
      if (nodes.has(superseded)) {
        edges.push({ from: superseded, kind: "supersedes", to: node.task.id });
      }
    });
  }
  return edges;
}
function contentIndexFor(nodes) {
  const index = /* @__PURE__ */ new Map();
  for (const node of nodes.values()) {
    if (!index.has(node.task.content_hash ?? "")) {
      index.set(node.task.content_hash ?? "", node.task.id);
    }
  }
  return index;
}
function isKnownTaskId(nodes, value) {
  return value !== void 0 && nodes.has(value);
}
function assertKnownDependencies(graph) {
  for (const node of graph.nodes.values()) {
    node.task.depends_on.forEach((dependency) => {
      if (!graph.nodes.has(dependency)) {
        throw new Error(`task ${node.task.id} depends on unknown task ${dependency}`);
      }
    });
  }
}
function assertAcyclic(graph) {
  projectWaveView(graph);
}
var init_graph = __esm({
  "src/domain/graph/index.ts"() {
    "use strict";
  }
});

// src/domain/tasks/index.ts
function renderTasksMd(tasks, specRef) {
  const featureId = specRef?.name ?? "council";
  const header = `# Tasks: ${featureId}

<!-- council-tasks-format: v1 -->`;
  const lines = [header.trim(), ""];
  for (const task of tasks) {
    const id = stringifyForDisplay(task.id);
    const taskTitle = stringifyForDisplay(task.title ?? id).replaceAll("\n", " ").trim() || id;
    lines.push(
      `## ${id}: ${taskTitle}`,
      `<!-- council-task-id: ${id} -->`,
      "```json",
      stableJsonStringify(task),
      "```",
      ""
    );
  }
  return `${lines.join("\n").trimEnd()}
`;
}
function parseTasksMd(text) {
  const tasks = [];
  TASK_BLOCK_RE.lastIndex = 0;
  for (const match of text.matchAll(TASK_BLOCK_RE)) {
    const groups = match.groups;
    const headerId = groups.headerId?.trim() ?? "";
    const markerId = groups.markerId?.trim() ?? "";
    if (headerId !== markerId) {
      throw new Error(`task marker mismatch: header ${pythonRepr(headerId)}, marker ${pythonRepr(markerId)}`);
    }
    let task;
    try {
      task = JSON.parse(groups.body ?? "");
    } catch (error) {
      throw new Error(`task ${pythonRepr(markerId)} JSON block is invalid: ${jsonErrorMessage(error)}`);
    }
    if (!isJsonRecord(task)) {
      throw new Error(`task ${pythonRepr(markerId)} JSON block must be an object`);
    }
    if (stringifyForDisplay(task.id).trim() !== markerId) {
      throw new Error(`task ${pythonRepr(markerId)} JSON id does not match marker`);
    }
    tasks.push(task);
  }
  if (tasks.length === 0) {
    throw new Error("no council task JSON blocks found in tasks.md");
  }
  const seen = /* @__PURE__ */ new Set();
  for (const task of tasks) {
    const taskId = stringifyForDisplay(task.id);
    if (seen.has(taskId)) {
      throw new Error(`duplicate task id in tasks.md: ${taskId}`);
    }
    seen.add(taskId);
  }
  return tasks;
}
function assertTasksBijection(tasks, tasksMdText) {
  const parsed = parseTasksMd(tasksMdText);
  validateTasks(parsed);
  if (normaliseTasks(parsed) !== normaliseTasks(tasks)) {
    throw new Error("tasks.md does not match tasks.json");
  }
}
function validateTasks(tasks, options = {}) {
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error("consolidator returned no tasks");
  }
  const seen = /* @__PURE__ */ new Set();
  for (const task of tasks) {
    if (!isJsonRecord(task)) {
      throw new Error("task ? must be an object");
    }
    const missing = REQUIRED_VALIDATE_FIELDS.filter((field) => !(field in task));
    if (missing.length > 0) {
      throw new Error(`task ${stringifyForDisplay(task.id ?? "?")} missing fields: ${formatPythonList(missing)}`);
    }
    const taskIdKey = comparableKey(task.id);
    if (seen.has(taskIdKey)) {
      throw new Error(`duplicate task id: ${stringifyForDisplay(task.id)}`);
    }
    seen.add(taskIdKey);
    if (!stringifyForDisplay(task.verify).trim()) {
      options.onWarning?.(
        `warning: task ${stringifyForDisplay(task.id)} has no verify command - its result is unchecked except by the adversarial verifier`
      );
    }
  }
  assertTaskDag(tasks);
}
function validateTasksJsonSchema(tasks) {
  const errors = [];
  if (!Array.isArray(tasks)) {
    return { valid: false, errors: ["tasks must be an array"] };
  }
  for (const [index, task] of tasks.entries()) {
    const path = `$[${String(index)}]`;
    if (!isJsonRecord(task)) {
      errors.push(`${path} must be an object`);
      continue;
    }
    for (const field of REQUIRED_SCHEMA_FIELDS) {
      if (!(field in task)) {
        errors.push(`${path}.${field} is required`);
      }
    }
    for (const field of Object.keys(task)) {
      if (!SCHEMA_ALLOWED_FIELDS.has(field)) {
        errors.push(`${path}.${field} is not allowed by schema`);
      }
    }
    validateSchemaFieldTypes(task, path, errors);
  }
  return { valid: errors.length === 0, errors };
}
function assertTasksJsonSchema(tasks) {
  const result = validateTasksJsonSchema(tasks);
  if (!result.valid) {
    throw new Error(`tasks JSON Schema validation failed: ${result.errors.join("; ")}`);
  }
}
function assertTaskDag(tasks) {
  const ids = new Set(tasks.map((task) => comparableKey(task.id)));
  const deps = /* @__PURE__ */ new Map();
  const idLabels = /* @__PURE__ */ new Map();
  for (const task of tasks) {
    const taskIdKey = comparableKey(task.id);
    if (!isJsonArray(task.depends_on)) {
      throw new Error(`task ${pythonRepr(stringifyForDisplay(task.id))} depends_on must be an array`);
    }
    deps.set(taskIdKey, task.depends_on);
    idLabels.set(taskIdKey, stringifyForDisplay(task.id));
    for (const dep of task.depends_on) {
      const depKey = comparableKey(dep);
      if (!ids.has(depKey)) {
        throw new Error(
          `task ${pythonRepr(stringifyForDisplay(task.id))} depends on unknown task ${pythonRepr(stringifyForDisplay(dep))}`
        );
      }
    }
  }
  const remaining = new Map(deps);
  const done = /* @__PURE__ */ new Set();
  while (remaining.size > 0) {
    const ready = [...remaining.entries()].filter(([, depList]) => depList.every((dep) => done.has(comparableKey(dep)))).map(([taskId]) => taskId).sort((left, right) => (idLabels.get(left) ?? left).localeCompare(idLabels.get(right) ?? right));
    if (ready.length === 0) {
      const remainingLabels = [...remaining.keys()].map((taskId) => idLabels.get(taskId) ?? taskId).sort();
      throw new Error(`dependency cycle among tasks: ${formatPythonList(remainingLabels)}`);
    }
    for (const taskId of ready) {
      done.add(taskId);
      remaining.delete(taskId);
    }
  }
}
function validateSchemaFieldTypes(task, path, errors) {
  for (const field of SCHEMA_STRING_FIELDS) {
    if (field in task && typeof task[field] !== "string") {
      errors.push(`${path}.${field} must be a string`);
    }
  }
  for (const field of SCHEMA_STRING_ARRAY_FIELDS) {
    if (field in task && !isStringArray(task[field])) {
      errors.push(`${path}.${field} must be an array of strings`);
    }
  }
  if (typeof task.id === "string" && !TASK_ID_RE.test(task.id)) {
    errors.push(`${path}.id must match a council task id`);
  }
  if (isJsonArray(task.depends_on)) {
    for (const [index, dep] of task.depends_on.entries()) {
      if (typeof dep === "string" && !TASK_ID_RE.test(dep)) {
        errors.push(`${path}.depends_on[${String(index)}] must match a council task id`);
      }
    }
  }
  if (isJsonArray(task.supersedes)) {
    for (const [index, dep] of task.supersedes.entries()) {
      if (typeof dep === "string" && !TASK_ID_RE.test(dep)) {
        errors.push(`${path}.supersedes[${String(index)}] must match a council task id`);
      }
    }
  }
  if (typeof task.difficulty === "string" && !["trivial", "moderate", "hard"].includes(task.difficulty)) {
    errors.push(`${path}.difficulty must be trivial, moderate, or hard`);
  }
  if (typeof task.model === "string" && !["haiku", "sonnet", "opus"].includes(task.model)) {
    errors.push(`${path}.model must be haiku, sonnet, or opus`);
  }
}
function normaliseTasks(tasks) {
  return stableJsonStringify(tasks);
}
function stableJsonStringify(value, level = 0) {
  if (value === null) {
    return "null";
  }
  if (typeof value === "string") {
    return quoteJsonString(value);
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return primitiveJsonString(value);
  }
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return "[]";
    }
    const currentIndent = " ".repeat(level * 2);
    const nextIndent = " ".repeat((level + 1) * 2);
    return `[
${value.map((item) => `${nextIndent}${stableJsonStringify(item, level + 1)}`).join(",\n")}
${currentIndent}]`;
  }
  if (isJsonRecord(value)) {
    const keys = Object.keys(value).sort();
    if (keys.length === 0) {
      return "{}";
    }
    const currentIndent = " ".repeat(level * 2);
    const nextIndent = " ".repeat((level + 1) * 2);
    return `{
${keys.map((key) => `${nextIndent}${quoteJsonString(key)}: ${stableJsonStringify(value[key], level + 1)}`).join(",\n")}
${currentIndent}}`;
  }
  throw new Error(`cannot serialize non-JSON value: ${describeNonJsonValue(value)}`);
}
function quoteJsonString(value) {
  const quoted = JSON.stringify(value);
  let escaped = "";
  for (let index = 0; index < quoted.length; index += 1) {
    const code = quoted.charCodeAt(index);
    escaped += code > 127 ? `\\u${code.toString(16).padStart(4, "0")}` : quoted.charAt(index);
  }
  return escaped;
}
function isJsonRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function isJsonArray(value) {
  return Array.isArray(value);
}
function isStringArray(value) {
  return isJsonArray(value) && value.every((item) => typeof item === "string");
}
function comparableKey(value) {
  return stableJsonStringify(value);
}
function stringifyForDisplay(value) {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean" || value === null) {
    return primitiveJsonString(value);
  }
  if (value === void 0) {
    return "";
  }
  return stableJsonStringify(value);
}
function primitiveJsonString(value) {
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (value === null) {
    return "null";
  }
  return Number.isFinite(value) ? String(Object.is(value, -0) ? 0 : value) : "null";
}
function describeNonJsonValue(value) {
  return value === void 0 ? "undefined" : typeof value;
}
function formatPythonList(values) {
  return `[${values.map((value) => pythonRepr(value)).join(", ")}]`;
}
function pythonRepr(value) {
  return `'${value.replaceAll("\\", "\\\\").replaceAll("'", "\\'")}'`;
}
function jsonErrorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
var TASK_BLOCK_RE, REQUIRED_VALIDATE_FIELDS, REQUIRED_SCHEMA_FIELDS, TASK_ID_RE, SCHEMA_STRING_FIELDS, SCHEMA_STRING_ARRAY_FIELDS, SCHEMA_ALLOWED_FIELDS;
var init_tasks = __esm({
  "src/domain/tasks/index.ts"() {
    "use strict";
    TASK_BLOCK_RE = /^## (?<headerId>[^\n:]+)(?::[^\n]*)?\n<!-- council-task-id: (?<markerId>[^>]+) -->\n```json\n(?<body>.*?)\n```/gms;
    REQUIRED_VALIDATE_FIELDS = ["id", "objective", "depends_on", "paths", "model", "verify"];
    REQUIRED_SCHEMA_FIELDS = [
      "id",
      "title",
      "objective",
      "output_format",
      "paths",
      "depends_on",
      "difficulty",
      "model",
      "verify",
      "boundaries"
    ];
    TASK_ID_RE = /^(?:T[0-9]+|ck-[0-9a-f]{4,})$/;
    SCHEMA_STRING_FIELDS = /* @__PURE__ */ new Set([
      "id",
      "title",
      "objective",
      "output_format",
      "difficulty",
      "model",
      "dev_notes",
      "spec_ref",
      "archetype",
      "context_profile",
      "discovered_from",
      "content_hash",
      "model_tier",
      "verify",
      "boundaries"
    ]);
    SCHEMA_STRING_ARRAY_FIELDS = /* @__PURE__ */ new Set([
      "paths",
      "depends_on",
      "acceptance_criteria",
      "context_refs",
      "supersedes"
    ]);
    SCHEMA_ALLOWED_FIELDS = /* @__PURE__ */ new Set([
      ...SCHEMA_STRING_FIELDS,
      ...SCHEMA_STRING_ARRAY_FIELDS,
      "engine"
    ]);
  }
});

// src/adapters/fs/index.ts
import { appendFile, mkdir, open, readFile, readdir, rename, unlink } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
async function normalizeLegacyRunDir(runDir) {
  const rawState = await readStandaloneJson(join(runDir, "state.json"));
  const tasks = assertTasks(await readStandaloneJson(join(runDir, "tasks.json")));
  const report = await readOptionalLegacyReport(join(runDir, "report.json"));
  const workerResults = await readWorkerResults(runDir);
  const graph = createTaskGraph(tasks, {
    idStrategy: tasks.some((task) => task.id.startsWith("T")) ? "legacy-ordinal" : "content-hash"
  });
  return {
    graph,
    report,
    runId: basename(runDir),
    state: normalizeLegacyState(rawState),
    tasks,
    workerResults
  };
}
function normalizeLegacyState(value) {
  const record = assertRecord(value, "state");
  const state = {};
  copyOptionalString(record, state, "stage");
  copyOptionalString(record, state, "intensity");
  copyOptionalInteger(record, state, "rounds");
  copyOptionalInteger(record, state, "task_count");
  copyOptionalString(record, state, "spec_id");
  copyOptionalString(record, state, "spec_slug");
  copyOptionalString(record, state, "spec_relpath");
  copyOptionalString(record, state, "integration_branch");
  return assertRunState(state);
}
async function readWorkerResults(runDir) {
  const workersDir = join(runDir, WORKERS_DIR);
  let taskIds;
  try {
    taskIds = await readdir(workersDir);
  } catch (error) {
    if (isErrno(error, "ENOENT")) return /* @__PURE__ */ new Map();
    throw error;
  }
  const results = /* @__PURE__ */ new Map();
  for (const taskId of taskIds.sort()) {
    results.set(
      taskId,
      assertWorkerResult(await readStandaloneJson(join(workersDir, taskId, RESULT_FILE)), taskId)
    );
  }
  return results;
}
async function readOptionalLegacyReport(path) {
  try {
    return assertLegacyReport(await readStandaloneJson(path));
  } catch (error) {
    if (isErrno(error, "ENOENT")) return void 0;
    throw error;
  }
}
async function readStandaloneJson(path) {
  return parseJson(await readFile(path, "utf8"));
}
function parseJson(text) {
  return JSON.parse(text);
}
function assertRunState(value) {
  const record = assertRecord(value, "state");
  assertAllowed(record, "state", [
    "stage",
    "intensity",
    "rounds",
    "task_count",
    "spec_id",
    "spec_slug",
    "spec_relpath",
    "agents",
    "integration_branch",
    "engine",
    "model_tier",
    "content_hash"
  ]);
  optionalString(record, "state", "stage");
  optionalString(record, "state", "intensity");
  optionalInteger(record, "state", "rounds");
  optionalInteger(record, "state", "task_count");
  optionalString(record, "state", "spec_id");
  optionalString(record, "state", "spec_slug");
  optionalString(record, "state", "spec_relpath");
  optionalStringArray(record, "state", "agents");
  optionalString(record, "state", "integration_branch");
  optionalString(record, "state", "model_tier");
  optionalString(record, "state", "content_hash");
  return record;
}
function assertTasks(value) {
  validateTasks(value);
  assertTasksJsonSchema(value);
  return value;
}
function assertReviewVerdict(value) {
  const record = assertRecord(value, "review verdict");
  assertAllowed(record, "review verdict", [
    "satisfied",
    "reasons",
    "issues",
    "task_id",
    "reviewer",
    "engine",
    "model_tier",
    "content_hash"
  ]);
  requiredBoolean(record, "review verdict", "satisfied");
  requiredString(record, "review verdict", "reasons");
  requiredStringArray(record, "review verdict", "issues");
  optionalString(record, "review verdict", "task_id");
  optionalString(record, "review verdict", "reviewer");
  optionalString(record, "review verdict", "model_tier");
  optionalString(record, "review verdict", "content_hash");
  return record;
}
function assertWorkerResult(value, taskId) {
  const record = assertRecord(value, "worker result");
  assertAllowed(record, "worker result", [
    "task_id",
    "title",
    "model",
    "suggested_model",
    "engine",
    "model_tier",
    "branch",
    "worktree",
    "committed",
    "summary",
    "files_changed",
    "out_of_bounds",
    "verify_rc",
    "verify_output",
    "verdict",
    "merge",
    "status",
    "error",
    "content_hash"
  ]);
  requiredString(record, "worker result", "task_id");
  requiredString(record, "worker result", "status");
  if (taskId !== void 0 && record.task_id !== taskId) {
    fail(`worker result task_id must match path task id: ${taskId}`);
  }
  optionalString(record, "worker result", "title");
  optionalString(record, "worker result", "model");
  optionalEnum(record, "worker result", "suggested_model", ["haiku", "sonnet", "opus"]);
  optionalString(record, "worker result", "model_tier");
  optionalString(record, "worker result", "branch");
  optionalString(record, "worker result", "worktree");
  optionalBoolean(record, "worker result", "committed");
  optionalString(record, "worker result", "summary");
  optionalStringArray(record, "worker result", "files_changed");
  optionalStringArray(record, "worker result", "out_of_bounds");
  optionalIntegerOrNull(record, "worker result", "verify_rc");
  optionalString(record, "worker result", "verify_output");
  if (record.verdict !== void 0 && record.verdict !== null) assertReviewVerdict(record.verdict);
  optionalString(record, "worker result", "merge");
  optionalString(record, "worker result", "error");
  optionalString(record, "worker result", "content_hash");
  return record;
}
function assertLegacyReport(value) {
  const record = assertRecord(value, "legacy report");
  requiredString(record, "legacy report", "run");
  optionalString(record, "legacy report", "integration_branch");
  optionalString(record, "legacy report", "integration_worktree");
  requiredArray(record, "legacy report", "waves").forEach(
    (wave) => {
      assertStringArray(wave, "legacy report wave");
    }
  );
  requiredArray(record, "legacy report", "tasks").forEach(assertLegacyTaskReport);
  return record;
}
function assertLegacyTaskReport(value) {
  const record = assertRecord(value, "legacy task report");
  requiredString(record, "legacy task report", "task_id");
  optionalString(record, "legacy task report", "status");
  optionalString(record, "legacy task report", "merge");
  optionalString(record, "legacy task report", "model");
  optionalStringArray(record, "legacy task report", "files_changed");
  optionalIntegerOrNull(record, "legacy task report", "verify_rc");
  optionalBoolean(record, "legacy task report", "verifier_satisfied");
  optionalStringArray(record, "legacy task report", "out_of_bounds");
  optionalString(record, "legacy task report", "branch");
  optionalBoolean(record, "legacy task report", "good");
}
function assertRecord(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`);
  return value;
}
function assertAllowed(record, label, allowed) {
  const allowedSet = new Set(allowed);
  Object.keys(record).forEach((key) => {
    if (!allowedSet.has(key)) fail(`${label}.${key} is not allowed`);
  });
}
function requiredString(record, label, field) {
  const value = record[field];
  if (typeof value !== "string") fail(`${label}.${field} must be a string`);
  return value;
}
function optionalString(record, label, field) {
  if (record[field] !== void 0 && typeof record[field] !== "string") fail(`${label}.${field} must be a string`);
}
function requiredBoolean(record, label, field) {
  if (typeof record[field] !== "boolean") fail(`${label}.${field} must be a boolean`);
}
function optionalBoolean(record, label, field) {
  if (record[field] !== void 0 && typeof record[field] !== "boolean") fail(`${label}.${field} must be a boolean`);
}
function optionalEnum(record, label, field, values) {
  const value = record[field];
  if (value !== void 0 && (typeof value !== "string" || !values.includes(value))) {
    fail(`${label}.${field} must be one of: ${values.join(", ")}`);
  }
}
function optionalInteger(record, label, field) {
  if (record[field] !== void 0 && !Number.isInteger(record[field])) fail(`${label}.${field} must be an integer`);
}
function optionalIntegerOrNull(record, label, field) {
  if (record[field] !== void 0 && record[field] !== null && !Number.isInteger(record[field])) {
    fail(`${label}.${field} must be an integer or null`);
  }
}
function requiredArray(record, label, field) {
  const value = record[field];
  if (!Array.isArray(value)) fail(`${label}.${field} must be an array`);
  return value;
}
function requiredStringArray(record, label, field) {
  assertStringArray(requiredArray(record, label, field), `${label}.${field}`);
}
function optionalStringArray(record, label, field) {
  if (record[field] !== void 0) assertStringArray(record[field], `${label}.${field}`);
}
function assertStringArray(value, label) {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    fail(`${label} must be an array of strings`);
  }
}
function copyOptionalString(from, to, field) {
  if (typeof from[field] === "string") to[field] = from[field];
}
function copyOptionalInteger(from, to, field) {
  if (Number.isInteger(from[field])) to[field] = from[field];
}
function isErrno(error, code) {
  return error instanceof Error && "code" in error && error.code === code;
}
function fail(message) {
  throw new Error(message);
}
var WORKERS_DIR, RESULT_FILE;
var init_fs = __esm({
  "src/adapters/fs/index.ts"() {
    "use strict";
    init_runstore();
    init_graph();
    init_tasks();
    WORKERS_DIR = "workers";
    RESULT_FILE = "result.json";
  }
});

// src/domain/config/index.ts
function parseToml(source) {
  const finalNewline = source.endsWith("\n");
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  if (finalNewline) {
    lines.pop();
  }
  const data = {};
  const assignments = [];
  const tables = [];
  let currentPath = [];
  let currentTable = data;
  lines.forEach((line, lineIndex) => {
    const body = stripInlineComment(line).trim();
    if (body === "") {
      return;
    }
    const arrayHeader = /^\[\[(.+)\]\]$/.exec(body);
    const tableHeader = /^\[(.+)\]$/.exec(body);
    if (arrayHeader) {
      currentPath = parseKeyPath(arrayHeader[1] ?? "");
      currentTable = appendArrayTable(data, currentPath);
      tables.push({ lineIndex, path: currentPath, array: true });
      return;
    }
    if (tableHeader) {
      currentPath = parseKeyPath(tableHeader[1] ?? "");
      currentTable = ensureTable(data, currentPath);
      tables.push({ lineIndex, path: currentPath, array: false });
      return;
    }
    const equalIndex = findTopLevelChar(body, "=");
    if (equalIndex < 1) {
      throw new Error(`invalid TOML assignment on line ${String(lineIndex + 1)}`);
    }
    const rawKey = body.slice(0, equalIndex).trim();
    const keyPath = parseKeyPath(rawKey);
    const value = parseTomlValue(body.slice(equalIndex + 1).trim());
    setNestedValue(currentTable, keyPath, value);
    assignments.push({ lineIndex, tablePath: currentPath, keyPath, sourceKey: rawKey });
  });
  return { source, lines, finalNewline, data, assignments, tables };
}
function parseCouncilConfig(source) {
  return normalizeCouncilConfig(parseToml(source).data);
}
function writeTomlValue(value) {
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new Error(`TOML number must be finite, got ${String(value)}`);
    }
    return String(value);
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (isTomlArray(value)) {
    return `[${value.map((item) => writeTomlValue(item)).join(", ")}]`;
  }
  throw new Error("inline TOML tables are not supported by the council writer");
}
function isTomlArray(value) {
  return Array.isArray(value);
}
function writeCouncilConfig(source, config) {
  return writeTomlUpdates(parseToml(source), flattenCouncilConfig(config));
}
function writeTomlUpdates(document, updates) {
  const lines = [...document.lines];
  const written = /* @__PURE__ */ new Set();
  const assignments = [...document.assignments].sort((a, b) => b.lineIndex - a.lineIndex);
  assignments.forEach((assignment) => {
    const updateKey = pathKey([...assignment.tablePath, ...assignment.keyPath]);
    const value = updates.get(updateKey);
    if (value === void 0 || written.has(updateKey)) {
      return;
    }
    lines[assignment.lineIndex] = replaceAssignmentValue(
      lines[assignment.lineIndex] ?? "",
      assignment.sourceKey,
      value
    );
    written.add(updateKey);
  });
  const existingInserts = /* @__PURE__ */ new Map();
  const newTableInserts = /* @__PURE__ */ new Map();
  updates.forEach((value, updateKey) => {
    if (written.has(updateKey)) {
      return;
    }
    queueMissingAssignment(document, existingInserts, newTableInserts, updateKey.split("."), value);
  });
  const orderedExistingInserts = [...existingInserts.entries()].sort(
    ([left], [right]) => right - left
  );
  orderedExistingInserts.forEach(([index, insertLines]) => lines.splice(index, 0, ...insertLines));
  newTableInserts.forEach((insert) => {
    if (lines.length > 0 && lines.at(-1) !== "") {
      lines.push("");
    }
    lines.push(`[${insert.path.join(".")}]`, ...insert.lines);
  });
  return `${lines.join("\n")}
`;
}
function resolveCouncilConfig(input = {}) {
  const mergedBeforeFlags = mergeCouncilConfigs(input.user, input.project);
  const intensity = resolveIntensity(input.preset, mergedBeforeFlags, input.flags);
  const preset = PRESETS[intensity];
  const resolved = mergeCouncilConfigs(
    { ...BASE_ROLES, ...preset },
    input.user,
    input.project,
    input.flags,
    { intensity }
  );
  return {
    ...resolved,
    intensity,
    planner_a: requireString(resolved.planner_a, "planner_a"),
    planner_b: requireString(resolved.planner_b, "planner_b"),
    consolidator: requireString(resolved.consolidator, "consolidator"),
    worker: requireString(resolved.worker, "worker"),
    verifier: requireString(resolved.verifier, "verifier"),
    codex_effort: requireCodexEffort(resolved.codex_effort),
    rounds: requireNumber(resolved.rounds, "rounds"),
    max_workers: requireNumber(resolved.max_workers, "max_workers"),
    runtime: resolveRuntime(input.env, resolved.codex_effort)
  };
}
function coerceConfigValue(key, raw) {
  if (!isConfigKey(key)) {
    throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(", ")}`);
  }
  switch (key) {
    case "intensity":
      return requireIntensity(raw);
    case "rounds":
    case "max_workers": {
      const parsed = Number.parseInt(raw, 10);
      if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
        throw new Error(`${key} must be an integer, got ${raw}`);
      }
      return parsed;
    }
    case "codex_effort":
      return requireCodexEffort(raw);
    case "planner_a":
    case "planner_b":
    case "consolidator":
    case "worker":
    case "verifier":
      if (!/^(claude|codex):.+/.test(raw)) {
        throw new Error(`${key} must be claude:<model> or codex:<model>, got ${raw}`);
      }
      return raw;
  }
}
function parseTomlValue(raw) {
  if (raw.startsWith('"')) {
    return parseString(raw);
  }
  if (raw.startsWith("[")) {
    return parseArray(raw);
  }
  if (raw === "true") {
    return true;
  }
  if (raw === "false") {
    return false;
  }
  if (/^[+-]?\d+$/.test(raw)) {
    return Number.parseInt(raw, 10);
  }
  throw new Error(`unsupported TOML value ${raw}`);
}
function parseString(raw) {
  const commentIndex = findValueEnd(raw);
  const candidate = raw.slice(0, commentIndex);
  try {
    return JSON.parse(candidate);
  } catch {
    throw new Error(`invalid TOML string ${raw}`);
  }
}
function parseArray(raw) {
  const end = findMatchingBracket(raw);
  if (end < 0) {
    throw new Error(`unterminated TOML array ${raw}`);
  }
  const inner = raw.slice(1, end).trim();
  if (inner === "") {
    return [];
  }
  return splitTopLevel(inner, ",").map((part) => parseTomlValue(part.trim()));
}
function parseKeyPath(raw) {
  const parts = splitTopLevel(raw.trim(), ".").map((part) => part.trim());
  if (parts.some((part) => !/^[A-Za-z0-9_-]+$/.test(part))) {
    throw new Error(`unsupported TOML key path ${raw}`);
  }
  return parts;
}
function stripInlineComment(line) {
  const commentIndex = findCommentIndex(line);
  return commentIndex < 0 ? line : line.slice(0, commentIndex);
}
function findCommentIndex(line) {
  let inString = false;
  let escaped = false;
  let depth = 0;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\" && inString) {
      escaped = true;
      continue;
    }
    if (char === '"') {
      inString = !inString;
      continue;
    }
    if (!inString && char === "[") {
      depth += 1;
      continue;
    }
    if (!inString && char === "]") {
      depth -= 1;
      continue;
    }
    if (!inString && depth === 0 && char === "#") {
      return index;
    }
  }
  return -1;
}
function findTopLevelChar(line, target) {
  let inString = false;
  let escaped = false;
  let depth = 0;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\" && inString) {
      escaped = true;
      continue;
    }
    if (char === '"') {
      inString = !inString;
      continue;
    }
    if (!inString && char === "[") {
      depth += 1;
      continue;
    }
    if (!inString && char === "]") {
      depth -= 1;
      continue;
    }
    if (!inString && depth === 0 && char === target) {
      return index;
    }
  }
  return -1;
}
function splitTopLevel(raw, delimiter) {
  const parts = [];
  let start = 0;
  let inString = false;
  let escaped = false;
  let depth = 0;
  for (let index = 0; index < raw.length; index += 1) {
    const char = raw[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\" && inString) {
      escaped = true;
      continue;
    }
    if (char === '"') {
      inString = !inString;
      continue;
    }
    if (!inString && char === "[") {
      depth += 1;
      continue;
    }
    if (!inString && char === "]") {
      depth -= 1;
      continue;
    }
    if (!inString && depth === 0 && char === delimiter) {
      parts.push(raw.slice(start, index));
      start = index + 1;
    }
  }
  parts.push(raw.slice(start));
  return parts;
}
function findMatchingBracket(raw) {
  let inString = false;
  let escaped = false;
  let depth = 0;
  for (let index = 0; index < raw.length; index += 1) {
    const char = raw[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\" && inString) {
      escaped = true;
      continue;
    }
    if (char === '"') {
      inString = !inString;
      continue;
    }
    if (!inString && char === "[") {
      depth += 1;
      continue;
    }
    if (!inString && char === "]") {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}
function findValueEnd(raw) {
  const commentIndex = findCommentIndex(raw);
  return commentIndex < 0 ? raw.length : commentIndex;
}
function setNestedValue(table, keyPath, value) {
  const head = keyPath[0] ?? "";
  const tail = keyPath.slice(1);
  if (tail.length === 0) {
    table[head] = value;
    return;
  }
  const child = table[head];
  if (!isTomlTable(child)) {
    table[head] = {};
  }
  setNestedValue(table[head], tail, value);
}
function ensureTable(root, path) {
  let table = root;
  path.forEach((part) => {
    const next = table[part];
    if (Array.isArray(next)) {
      throw new Error(`TOML table conflicts with existing array table ${path.join(".")}`);
    }
    if (!isTomlTable(next)) {
      table[part] = {};
    }
    table = table[part];
  });
  return table;
}
function appendArrayTable(root, path) {
  const parent = ensureTable(root, path.slice(0, -1));
  const name = path.at(-1) ?? "";
  const current = parent[name];
  const next = {};
  if (current === void 0) {
    parent[name] = [next];
    return next;
  }
  if (Array.isArray(current)) {
    const tables = current;
    tables.push(next);
    return next;
  }
  throw new Error(`TOML array table conflicts with existing table ${path.join(".")}`);
}
function isTomlTable(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function replaceAssignmentValue(line, sourceKey, value) {
  const commentIndex = findCommentIndex(line);
  const suffix = commentIndex < 0 ? "" : line.slice(commentIndex);
  const prefixMatch = /^(\s*)/.exec(line);
  const prefix = prefixMatch?.[1] ?? "";
  const spacing = suffix === "" ? "" : " ";
  return `${prefix}${sourceKey} = ${writeTomlValue(value)}${spacing}${suffix}`.trimEnd();
}
function queueMissingAssignment(document, existingInserts, newTableInserts, fullPath, value) {
  const key = fullPath.at(-1);
  if (!key) {
    throw new Error("cannot write empty TOML path");
  }
  const tablePath = fullPath.slice(0, -1);
  const insertLine = `${key} = ${writeTomlValue(value)}`;
  const table = findTable(document, tablePath);
  if (!table && tablePath.length > 0) {
    const tableKey = pathKey(tablePath);
    const pending2 = newTableInserts.get(tableKey);
    if (pending2) {
      pending2.lines.push(insertLine);
    } else {
      newTableInserts.set(tableKey, { path: tablePath, lines: [insertLine] });
    }
    return;
  }
  const index = table ? findTableInsertIndex(document, table) : findRootInsertIndex(document);
  const pending = existingInserts.get(index);
  if (pending) {
    pending.push(insertLine);
  } else {
    existingInserts.set(index, [insertLine]);
  }
}
function findTable(document, tablePath) {
  const tables = [...document.tables].reverse();
  return tables.find((table) => !table.array && samePath(table.path, tablePath)) ?? tables.find((table) => table.array && samePath(table.path, tablePath));
}
function findTableInsertIndex(document, table) {
  const next = document.tables.find((candidate) => candidate.lineIndex > table.lineIndex);
  return next?.lineIndex ?? document.lines.length;
}
function findRootInsertIndex(document) {
  return document.tables[0]?.lineIndex ?? document.lines.length;
}
function samePath(left, right) {
  return left.length === right.length && left.every((part, index) => part === right[index]);
}
function pathKey(path) {
  return path.join(".");
}
function normalizeCouncilConfig(data) {
  return omitUndefined({
    intensity: optionalIntensity(data.intensity),
    planner_a: optionalString2(data.planner_a),
    planner_b: optionalString2(data.planner_b),
    consolidator: optionalString2(data.consolidator),
    worker: optionalString2(data.worker),
    verifier: optionalString2(data.verifier),
    codex_effort: optionalCodexEffort(data.codex_effort),
    rounds: optionalNumber(data.rounds),
    max_workers: optionalNumber(data.max_workers),
    watchdog: optionalObject(
      data.watchdog,
      (watchdog) => omitUndefined({
        stall_after_s: optionalNumber(watchdog.stall_after_s),
        window: optionalNumber(watchdog.window),
        repeat_limit: optionalNumber(watchdog.repeat_limit),
        max_restarts: optionalNumber(watchdog.max_restarts),
        escalate_model: optionalString2(watchdog.escalate_model),
        disk_cap_gib: optionalNumber(watchdog.disk_cap_gib)
      })
    ),
    design: optionalObject(
      data.design,
      (design) => omitUndefined({
        lenses: optionalStringArray2(design.lenses),
        rounds: optionalNumber(design.rounds),
        stages: optionalStringRecordTable(
          design.stages,
          (stage) => omitUndefined({
            engine: optionalString2(stage.engine),
            effort: optionalString2(stage.effort)
          })
        )
      })
    ),
    review: optionalObject(
      data.review,
      (review) => omitUndefined({
        council: optionalBoolean2(review.council),
        max_fix_rounds: optionalNumber(review.max_fix_rounds),
        difficulty: optionalStringRecord(review.difficulty)
      })
    ),
    github: optionalObject(
      data.github,
      (github) => omitUndefined({
        enabled: optionalBoolean2(github.enabled),
        assignee: optionalString2(github.assignee)
      })
    ),
    engines: optionalStringRecordTable(
      data.engines,
      (engine) => omitUndefined({
        argv: optionalStringArray2(engine.argv),
        stream_format: optionalString2(engine.stream_format),
        result_extraction: optionalString2(engine.result_extraction)
      })
    ),
    triage: optionalObject(
      data.triage,
      (triage) => omitUndefined({
        matrix_overrides: optionalStringRecord(triage.matrix_overrides)
      })
    ),
    context: optionalObject(
      data.context,
      (context) => omitUndefined({
        pack_stale_after_s: optionalNumber(context.pack_stale_after_s)
      })
    ),
    model_matrix: optionalObject(
      data.model_matrix,
      (modelMatrix) => omitUndefined({
        roles: optionalRoleRecord(modelMatrix.roles),
        intensity: optionalStringRecordTable(
          modelMatrix.intensity,
          (preset) => omitUndefined({
            rounds: optionalNumber(preset.rounds),
            codex_effort: optionalCodexEffort(preset.codex_effort),
            worker: optionalString2(preset.worker),
            max_workers: optionalNumber(preset.max_workers)
          })
        )
      })
    )
  });
}
function flattenCouncilConfig(config) {
  const updates = /* @__PURE__ */ new Map();
  addScalars(updates, [], config, CONFIG_KEYS);
  addScalars(updates, ["watchdog"], config.watchdog, [
    "stall_after_s",
    "window",
    "repeat_limit",
    "max_restarts",
    "escalate_model",
    "disk_cap_gib"
  ]);
  addScalars(updates, ["design"], config.design, ["lenses", "rounds"]);
  addNestedScalars(updates, ["design", "stages"], config.design?.stages, ["engine", "effort"]);
  addScalars(updates, ["review"], config.review, ["council", "max_fix_rounds"]);
  addRecord(updates, ["review", "difficulty"], config.review?.difficulty);
  addScalars(updates, ["github"], config.github, ["enabled", "assignee"]);
  addNestedScalars(updates, ["engines"], config.engines, ["argv", "stream_format", "result_extraction"]);
  addRecord(updates, ["triage", "matrix_overrides"], config.triage?.matrix_overrides);
  addScalars(updates, ["context"], config.context, ["pack_stale_after_s"]);
  addRecord(updates, ["model_matrix", "roles"], config.model_matrix?.roles);
  addNestedScalars(updates, ["model_matrix", "intensity"], config.model_matrix?.intensity, [
    "rounds",
    "codex_effort",
    "worker",
    "max_workers"
  ]);
  return updates;
}
function addScalars(updates, prefix, source, keys) {
  if (!source) {
    return;
  }
  keys.forEach((key) => {
    const value = source[key];
    if (value !== void 0) {
      updates.set(pathKey([...prefix, key]), value);
    }
  });
}
function addRecord(updates, prefix, source) {
  if (!source) {
    return;
  }
  Object.entries(source).forEach(([key, value]) => {
    if (value !== void 0) {
      updates.set(pathKey([...prefix, key]), value);
    }
  });
}
function addNestedScalars(updates, prefix, source, keys) {
  if (!source) {
    return;
  }
  Object.entries(source).forEach(([name, value]) => {
    addScalars(updates, [...prefix, name], value, keys);
  });
}
function mergeCouncilConfigs(...configs) {
  return configs.reduce((merged, config) => deepMerge(merged, config), {});
}
function deepMerge(left, right) {
  if (!right) {
    return left;
  }
  const merged = { ...left };
  Object.entries(right).forEach(([key, value]) => {
    if (value === void 0) {
      return;
    }
    const existing = merged[key];
    merged[key] = isPlainObject(existing) && isPlainObject(value) ? deepMerge(existing, value) : value;
  });
  return merged;
}
function resolveIntensity(preset, config, flags) {
  return requireIntensity(flags?.intensity ?? config.intensity ?? preset ?? DEFAULT_INTENSITY);
}
function resolveRuntime(env, codexEffort) {
  const codexReasoning = env?.get("COUNCIL_CODEX_REASONING") ?? codexEffort ?? ENV_DEFAULTS.COUNCIL_CODEX_REASONING;
  return {
    codex_reasoning: codexReasoning,
    plan_timeout_s: envInt(env, "COUNCIL_PLAN_TIMEOUT_S", ENV_DEFAULTS.COUNCIL_PLAN_TIMEOUT_S),
    worker_timeout_s: envInt(env, "COUNCIL_WORKER_TIMEOUT_S", ENV_DEFAULTS.COUNCIL_WORKER_TIMEOUT_S),
    verify_timeout_s: envInt(env, "COUNCIL_VERIFY_TIMEOUT_S", ENV_DEFAULTS.COUNCIL_VERIFY_TIMEOUT_S)
  };
}
function envInt(env, name, fallback) {
  const raw = env?.get(name);
  if (raw === void 0) {
    return fallback;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
    throw new Error(`${name} must be an integer, got ${raw}`);
  }
  return parsed;
}
function requireString(value, key) {
  const parsed = optionalString2(value);
  if (parsed === void 0) {
    throw new Error(`${key} must be a string`);
  }
  return parsed;
}
function requireNumber(value, key) {
  const parsed = optionalNumber(value);
  if (parsed === void 0) {
    throw new Error(`${key} must be a number`);
  }
  return parsed;
}
function requireIntensity(value) {
  const parsed = optionalIntensity(value);
  if (parsed === void 0) {
    throw new Error(`unknown intensity ${String(value)}; choose from ${Object.keys(PRESETS).join(", ")}`);
  }
  return parsed;
}
function requireCodexEffort(value) {
  const parsed = optionalCodexEffort(value);
  if (parsed === void 0) {
    throw new Error(`codex_effort must be one of ${CODEX_EFFORTS.join(", ")}`);
  }
  return parsed;
}
function optionalIntensity(value) {
  return typeof value === "string" && value in PRESETS ? value : void 0;
}
function optionalCodexEffort(value) {
  return typeof value === "string" && CODEX_EFFORTS.includes(value) ? value : void 0;
}
function optionalString2(value) {
  return typeof value === "string" ? value : void 0;
}
function optionalNumber(value) {
  return typeof value === "number" ? value : void 0;
}
function optionalBoolean2(value) {
  return typeof value === "boolean" ? value : void 0;
}
function optionalStringArray2(value) {
  return Array.isArray(value) && value.every((item) => typeof item === "string") ? value : void 0;
}
function optionalObject(value, map) {
  const table = asTomlTable(value);
  if (!table) {
    return void 0;
  }
  const mapped = map(table);
  return Object.keys(mapped).length > 0 ? mapped : void 0;
}
function optionalStringRecord(value) {
  const table = asTomlTable(value);
  if (!table) {
    return void 0;
  }
  const entries = Object.entries(table).filter((entry) => typeof entry[1] === "string");
  return entries.length > 0 ? Object.fromEntries(entries) : void 0;
}
function optionalRoleRecord(value) {
  const record = optionalStringRecord(value);
  if (!record) {
    return void 0;
  }
  return Object.fromEntries(Object.entries(record).filter(([key]) => ROLE_KEYS.includes(key)));
}
function optionalStringRecordTable(value, map) {
  const table = asTomlTable(value);
  if (!table) {
    return void 0;
  }
  const entries = Object.entries(table).map(([key, entry]) => [key, latestTable(entry)]).filter((entry) => entry[1] !== void 0).map(([key, table2]) => [key, map(table2)]).filter((entry) => Object.keys(entry[1]).length > 0);
  return entries.length > 0 ? Object.fromEntries(entries) : void 0;
}
function latestTable(value) {
  const table = asTomlTable(value);
  if (table) {
    return table;
  }
  if (Array.isArray(value)) {
    return value.map((item) => asTomlTable(item)).filter((item) => item !== void 0).at(-1);
  }
  return void 0;
}
function asTomlTable(value) {
  return isTomlTable(value) ? value : void 0;
}
function omitUndefined(value) {
  return Object.fromEntries(Object.entries(value).filter((entry) => entry[1] !== void 0));
}
function isPlainObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function isConfigKey(key) {
  return CONFIG_KEYS.includes(key);
}
var DEFAULT_INTENSITY, ROLE_KEYS, INT_KEYS, CODEX_EFFORTS, CONFIG_KEYS, BASE_ROLES, PRESETS, ENV_DEFAULTS;
var init_config = __esm({
  "src/domain/config/index.ts"() {
    "use strict";
    DEFAULT_INTENSITY = "standard";
    ROLE_KEYS = ["planner_a", "planner_b", "consolidator", "worker", "verifier"];
    INT_KEYS = ["rounds", "max_workers"];
    CODEX_EFFORTS = ["low", "medium", "high", "xhigh"];
    CONFIG_KEYS = [
      "intensity",
      ...ROLE_KEYS,
      "codex_effort",
      ...INT_KEYS
    ];
    BASE_ROLES = {
      planner_a: "claude:opus",
      planner_b: "codex:gpt-5.5",
      consolidator: "claude:opus",
      verifier: "claude:sonnet"
    };
    PRESETS = {
      quick: { rounds: 1, codex_effort: "low", worker: "claude:haiku", max_workers: 4 },
      standard: { rounds: 2, codex_effort: "high", worker: "claude:haiku", max_workers: 6 },
      thorough: { rounds: 3, codex_effort: "high", worker: "claude:sonnet", max_workers: 6 },
      max: { rounds: 3, codex_effort: "xhigh", worker: "claude:sonnet", max_workers: 8 }
    };
    ENV_DEFAULTS = {
      COUNCIL_CODEX_REASONING: "high",
      COUNCIL_PLAN_TIMEOUT_S: 1200,
      COUNCIL_WORKER_TIMEOUT_S: 1800,
      COUNCIL_VERIFY_TIMEOUT_S: 600
    };
  }
});

// src/domain/triage/routing-matrix.json
var routing_matrix_default;
var init_routing_matrix = __esm({
  "src/domain/triage/routing-matrix.json"() {
    routing_matrix_default = {
      $schema: "./routing-matrix.schema.json",
      schemaVersion: 1,
      source: {
        repository: "https://github.com/cameronsjo/spec-compare",
        path: "docs/use-case-scoring.md",
        ref: "main",
        transcribedAt: "2026-07-03",
        notes: [
          "Decision tree: modification prefers OpenSpec-style deltas; small greenfield prefers Spec-Kit-style full planning; large parallel work prefers Spec Kitty-style orchestration; emergencies and unclear experiments skip SDD.",
          "Council routes adapt those choices to direct, delta, full, and program orchestration while preserving plan-only behavior."
        ]
      },
      dimensions: {
        size: ["trivial", "small", "medium", "large", "program"],
        landscape: ["brownfield", "greenfield"],
        kind: [
          "ui-tweak",
          "bugfix",
          "hotfix",
          "refactor",
          "api",
          "feature",
          "cross-cutting",
          "maintenance",
          "design-system",
          "prototype"
        ],
        risk: ["low", "medium", "high", "critical"],
        clarity: ["clear", "needs-questions", "unclear"],
        parallelism: ["none", "some", "high"]
      },
      useCaseScores: [
        {
          id: "trivial-modification",
          category: "trivial",
          best: ["OpenSpec"],
          avoid: ["BMad", "Kiro"],
          scores: { OpenSpec: 5, "Spec-Kit": 3, "Spec Kitty": 2, BMad: 1, Kiro: 2, Tessl: 4 }
        },
        {
          id: "greenfield-feature",
          category: "large",
          best: ["Spec-Kit", "BMad"],
          avoid: ["OpenSpec"],
          scores: { OpenSpec: 3, "Spec-Kit": 5, "Spec Kitty": 4, BMad: 5, Kiro: 4, Tessl: 3 }
        },
        {
          id: "medium-refactor",
          category: "medium",
          best: ["OpenSpec"],
          avoid: ["BMad"],
          scores: { OpenSpec: 5, "Spec-Kit": 4, "Spec Kitty": 3, BMad: 2, Kiro: 3, Tessl: 4 }
        },
        {
          id: "production-bug",
          category: "emergency",
          best: ["OpenSpec", "None"],
          avoid: ["BMad", "Kiro"],
          scores: { OpenSpec: 4, "Spec-Kit": 2, "Spec Kitty": 2, BMad: 1, Kiro: 1, Tessl: 3, None: 4 }
        },
        {
          id: "api-endpoint",
          category: "large",
          best: ["BMad"],
          avoid: ["OpenSpec"],
          scores: { OpenSpec: 3, "Spec-Kit": 4, "Spec Kitty": 3, BMad: 5, Kiro: 3, Tessl: 4 }
        },
        {
          id: "parallel-features",
          category: "parallel",
          best: ["Spec Kitty"],
          avoid: ["Spec-Kit"],
          scores: { OpenSpec: 3, "Spec-Kit": 2, "Spec Kitty": 5, BMad: 2, Kiro: 2, Tessl: 2 }
        },
        {
          id: "cross-cutting-change",
          category: "medium",
          best: ["OpenSpec"],
          avoid: ["Kiro"],
          scores: { OpenSpec: 5, "Spec-Kit": 3, "Spec Kitty": 3, BMad: 2, Kiro: 2, Tessl: 3 }
        },
        {
          id: "emergency-hotfix",
          category: "emergency",
          best: ["None"],
          avoid: ["OpenSpec", "Spec-Kit", "Spec Kitty", "BMad", "Kiro", "Tessl"],
          scores: { OpenSpec: 2, "Spec-Kit": 1, "Spec Kitty": 1, BMad: 1, Kiro: 1, Tessl: 1, None: 5 }
        },
        {
          id: "design-system",
          category: "large",
          best: ["Spec-Kit", "BMad"],
          avoid: ["Tessl"],
          scores: { OpenSpec: 3, "Spec-Kit": 5, "Spec Kitty": 4, BMad: 5, Kiro: 3, Tessl: 2 }
        },
        {
          id: "experimental-prototype",
          category: "unclear",
          best: ["None"],
          avoid: ["BMad"],
          scores: { OpenSpec: 3, "Spec-Kit": 2, "Spec Kitty": 2, BMad: 1, Kiro: 3, Tessl: 2, None: 5 }
        },
        {
          id: "dependency-update",
          category: "medium",
          best: ["OpenSpec"],
          avoid: ["BMad"],
          scores: { OpenSpec: 4, "Spec-Kit": 3, "Spec Kitty": 2, BMad: 2, Kiro: 2, Tessl: 3 }
        },
        {
          id: "solo-side-project",
          category: "small",
          best: ["OpenSpec", "None"],
          avoid: ["BMad"],
          scores: { OpenSpec: 4, "Spec-Kit": 3, "Spec Kitty": 2, BMad: 1, Kiro: 3, Tessl: 2, None: 4 }
        }
      ],
      routeProfiles: [
        {
          route: "direct",
          basis: "No SDD/direct-code path adapted to council as a single minimal task DAG.",
          dagShape: "single-minimal-task",
          planExecutesWorkers: false,
          stageTiers: {
            grill: "haiku",
            survey: "skip",
            plan: "haiku",
            critique: "skip",
            consolidate: "haiku",
            tasking: "haiku",
            verify: "haiku"
          }
        },
        {
          route: "delta",
          basis: "OpenSpec-style modification path for brownfield deltas.",
          dagShape: "delta-task-dag",
          planExecutesWorkers: false,
          stageTiers: {
            grill: "haiku",
            survey: "haiku",
            plan: "sonnet",
            critique: "haiku",
            consolidate: "sonnet",
            tasking: "haiku",
            verify: "sonnet"
          }
        },
        {
          route: "full",
          basis: "Spec-Kit/BMad-style full planning for greenfield or architecture-heavy work.",
          dagShape: "full-task-dag",
          planExecutesWorkers: false,
          stageTiers: {
            grill: "sonnet",
            survey: "sonnet",
            plan: "opus",
            critique: "sonnet",
            consolidate: "opus",
            tasking: "sonnet",
            verify: "sonnet"
          }
        },
        {
          route: "program",
          basis: "Spec Kitty-style parallel orchestration for large multi-agent programs.",
          dagShape: "parallel-program-dag",
          planExecutesWorkers: false,
          stageTiers: {
            grill: "sonnet",
            survey: "opus",
            plan: "opus",
            critique: "opus",
            consolidate: "opus",
            tasking: "sonnet",
            verify: "sonnet"
          }
        }
      ],
      routeRules: [
        {
          id: "critical-hotfix-direct",
          route: "direct",
          reason: "Emergency hotfixes should skip SDD and document after the immediate repair.",
          useCaseRefs: ["emergency-hotfix", "production-bug"],
          when: { kind: ["hotfix"], risk: ["critical"] }
        },
        {
          id: "trivial-clear-direct",
          route: "direct",
          reason: "A trivial, clear, low-risk change should not fan out planning overhead.",
          useCaseRefs: ["trivial-modification", "solo-side-project"],
          when: {
            size: ["trivial"],
            kind: ["ui-tweak", "bugfix", "maintenance"],
            risk: ["low"],
            clarity: ["clear"],
            parallelism: ["none"]
          }
        },
        {
          id: "prototype-direct",
          route: "direct",
          reason: "Unclear prototypes should explore cheaply before a heavier SDD route exists.",
          useCaseRefs: ["experimental-prototype"],
          when: {
            size: ["trivial", "small"],
            kind: ["prototype"],
            risk: ["low", "medium"],
            clarity: ["unclear"]
          }
        },
        {
          id: "high-parallel-program",
          route: "program",
          reason: "High parallelism needs explicit program-level orchestration and collision control.",
          useCaseRefs: ["parallel-features"],
          when: { parallelism: ["high"] }
        },
        {
          id: "program-scale",
          route: "program",
          reason: "Program-sized work needs staged coordination even without explicit parallel demand.",
          useCaseRefs: ["parallel-features", "design-system"],
          when: { size: ["program"] }
        },
        {
          id: "large-high-risk-program",
          route: "program",
          reason: "Large high-risk work should be decomposed and governed as a program.",
          useCaseRefs: ["design-system", "greenfield-feature"],
          when: { size: ["large"], risk: ["high", "critical"] }
        },
        {
          id: "brownfield-delta",
          route: "delta",
          reason: "Brownfield modifications fit an OpenSpec-style delta path.",
          useCaseRefs: [
            "trivial-modification",
            "medium-refactor",
            "production-bug",
            "cross-cutting-change",
            "dependency-update"
          ],
          when: {
            landscape: ["brownfield"],
            kind: ["ui-tweak", "bugfix", "refactor", "cross-cutting", "maintenance"]
          }
        },
        {
          id: "greenfield-full",
          route: "full",
          reason: "Greenfield features need full specification, planning, and task generation.",
          useCaseRefs: ["greenfield-feature", "api-endpoint", "design-system"],
          when: { landscape: ["greenfield"] }
        },
        {
          id: "architecture-full",
          route: "full",
          reason: "API, feature, and design-system work usually needs architecture-level planning.",
          useCaseRefs: ["api-endpoint", "design-system", "greenfield-feature"],
          when: { kind: ["api", "feature", "design-system"] }
        },
        {
          id: "fallback-delta",
          route: "delta",
          reason: "Unknown brownfield work defaults to a delta route.",
          useCaseRefs: ["medium-refactor"],
          when: { landscape: ["brownfield"] }
        },
        {
          id: "fallback-full",
          route: "full",
          reason: "Unknown work defaults to full planning.",
          useCaseRefs: ["greenfield-feature"],
          when: {}
        }
      ],
      stageAdjustments: [
        {
          id: "clarity-needs-grill",
          reason: "Unresolved questions should spend more tier on grill before planning.",
          when: { clarity: ["needs-questions"] },
          minTiers: { grill: "sonnet" }
        },
        {
          id: "unclear-grill",
          reason: "Unclear requirements require a stronger exploration pass.",
          when: { clarity: ["unclear"] },
          minTiers: { grill: "opus", survey: "sonnet" }
        },
        {
          id: "high-risk-review",
          reason: "High risk raises planning and verification tiers.",
          when: { risk: ["high", "critical"] },
          minTiers: { plan: "opus", consolidate: "opus", verify: "sonnet" }
        },
        {
          id: "some-parallelism",
          reason: "Parallel execution needs stronger decomposition and consolidation.",
          when: { parallelism: ["some"] },
          minTiers: { survey: "sonnet", consolidate: "opus", tasking: "sonnet" }
        }
      ]
    };
  }
});

// src/domain/triage/index.ts
function loadRoutingMatrix(value = routing_matrix_default) {
  return parseRoutingMatrix(value);
}
function parseRoutingMatrix(value) {
  if (!isRecord(value) || value.$schema !== "./routing-matrix.schema.json" || value.schemaVersion !== 1) {
    throw new TypeError("Routing matrix must declare schema version 1");
  }
  if (!Array.isArray(value.routeProfiles) || !Array.isArray(value.routeRules) || !Array.isArray(value.stageAdjustments)) {
    throw new TypeError("Routing matrix must include profiles, rules, and stage adjustments");
  }
  return value;
}
function classifyTriage(input, matrix = routingMatrix) {
  const matches = matrix.routeRules.filter((rule) => matchesCondition(input, rule.when));
  const selectedRule = matches[0];
  if (selectedRule === void 0) {
    throw new Error("Routing matrix has no fallback rule");
  }
  const profile = matrix.routeProfiles.find((routeProfile) => routeProfile.route === selectedRule.route);
  if (profile === void 0) {
    throw new Error(`Routing matrix has no profile for ${selectedRule.route}`);
  }
  const candidateRules = matches.length > 1 ? matches.filter((rule) => hasSpecificCondition(rule.when)) : matches;
  const appliedAdjustments = matrix.stageAdjustments.filter((adjustment) => matchesCondition(input, adjustment.when));
  return {
    route: selectedRule.route,
    matchedRuleId: selectedRule.id,
    candidateRoutes: uniqueRoutes(candidateRules.map((rule) => rule.route)),
    reasons: [selectedRule.reason, ...appliedAdjustments.map((adjustment) => adjustment.reason)],
    useCaseRefs: selectedRule.useCaseRefs,
    stageTiers: applyStageAdjustments(profile.stageTiers, appliedAdjustments),
    plan: {
      dagShape: profile.dagShape,
      executesWorkers: profile.planExecutesWorkers,
      directWorkerPolicy: "never-during-plan"
    }
  };
}
function matchesCondition(input, condition) {
  return includesOrAny(condition.size, input.size) && includesOrAny(condition.landscape, input.landscape) && includesOrAny(condition.kind, input.kind) && includesOrAny(condition.risk, input.risk) && includesOrAny(condition.clarity, input.clarity) && includesOrAny(condition.parallelism, input.parallelism);
}
function applyStageAdjustments(base, adjustments) {
  return adjustments.reduce(
    (stageTiers, adjustment) => mergeMinTiers(stageTiers, adjustment.minTiers),
    { ...base }
  );
}
function mergeMinTiers(stageTiers, minTiers) {
  return {
    grill: maxTier(stageTiers.grill, minTiers.grill),
    survey: maxTier(stageTiers.survey, minTiers.survey),
    plan: maxTier(stageTiers.plan, minTiers.plan),
    critique: maxTier(stageTiers.critique, minTiers.critique),
    consolidate: maxTier(stageTiers.consolidate, minTiers.consolidate),
    tasking: maxTier(stageTiers.tasking, minTiers.tasking),
    verify: maxTier(stageTiers.verify, minTiers.verify)
  };
}
function maxTier(current, minimum) {
  return minimum === void 0 || tierRank[current] >= tierRank[minimum] ? current : minimum;
}
function includesOrAny(allowed, value) {
  return allowed === void 0 || allowed.includes(value);
}
function hasSpecificCondition(condition) {
  return Object.keys(condition).length > 0;
}
function uniqueRoutes(routes) {
  return [...new Set(routes)];
}
function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
var tierRank, routingMatrix;
var init_triage = __esm({
  "src/domain/triage/index.ts"() {
    "use strict";
    init_routing_matrix();
    tierRank = {
      skip: 0,
      haiku: 1,
      sonnet: 2,
      opus: 3
    };
    routingMatrix = loadRoutingMatrix();
  }
});

// src/app/index.ts
var app_exports = {};
__export(app_exports, {
  CouncilApp: () => CouncilApp,
  assignAgents: () => assignAgents,
  extractJson: () => extractJson,
  localizeVerify: () => localizeVerify,
  parseAgentsPool: () => parseAgentsPool,
  parseEngineSpec: () => parseEngineSpec,
  pythonSelfTestGolden: () => pythonSelfTestGolden,
  renderTemplate: () => renderTemplate,
  splitDestUrl: () => splitDestUrl
});
import { mkdir as mkdir2, readFile as readFile2, writeFile } from "node:fs/promises";
import { basename as basename2, dirname as dirname2 } from "node:path";
function parseEngineSpec(spec) {
  const [cli, ...rest] = spec.split(":");
  const model = rest.join(":");
  if (cli !== "claude" && cli !== "codex" || model.trim().length === 0) {
    throw new Error(`engine must be claude:<model> or codex:<model>, got ${JSON.stringify(spec)}`);
  }
  return { cli, label: `${cli}:${model}`, model };
}
function parseAgentsPool(spec) {
  if (spec.trim().length === 0) throw new Error("agents pool must not be empty");
  return spec.split(",").flatMap((part) => {
    const pieces = part.trim().split("*");
    if (pieces.length > 2) throw new Error(`malformed agent spec ${JSON.stringify(part)}`);
    const [engineRaw, countRaw] = pieces;
    if (!engineRaw) throw new Error(`malformed agent spec ${JSON.stringify(part)}`);
    const engine = parseEngineSpec(engineRaw);
    const count = countRaw === void 0 ? 1 : Number.parseInt(countRaw, 10);
    if (!Number.isInteger(count) || count < 1 || String(count) !== String(countRaw ?? 1)) {
      throw new Error(`agent count must be a positive integer in ${JSON.stringify(part)}`);
    }
    return Array.from({ length: count }, () => engine);
  });
}
function assignAgents(taskIds, agents) {
  const [head, ...tail] = agents;
  if (head === void 0) throw new Error("agents pool must not be empty");
  const pool = [head, ...tail];
  return new Map(taskIds.map((taskId, index) => [taskId, pool[index % pool.length] ?? head]));
}
function extractJson(text) {
  const fenced = /```json\s*([\s\S]*?)\s*```/u.exec(text);
  if (fenced?.[1]) return JSON.parse(fenced[1]);
  const start = text.indexOf("{");
  if (start < 0) throw new Error("no JSON object found");
  for (let end = text.length; end > start; end -= 1) {
    const candidate = text.slice(start, end).trim();
    if (!candidate.endsWith("}")) continue;
    try {
      return JSON.parse(candidate);
    } catch {
      continue;
    }
  }
  throw new Error("no JSON object found");
}
function renderTemplate(template, values) {
  return Object.entries(values).reduce(
    (rendered, [key, value]) => rendered.replaceAll(`{{${key}}}`, value),
    template
  );
}
function splitDestUrl(owner, name) {
  return `git@github.com:${owner}/${name}.git`;
}
function localizeVerify(command, repoRoot, worktree) {
  return command.replaceAll(repoRoot, worktree);
}
function pythonSelfTestGolden() {
  const tasks = [
    taskForSelfTest("T1", []),
    taskForSelfTest("T2", ["T1"]),
    taskForSelfTest("T3", ["T1"]),
    taskForSelfTest("T4", ["T2", "T3"])
  ];
  const config = resolveCouncilConfig({ flags: { intensity: "quick", rounds: 5 } });
  const agents = assignAgents(["t1", "t2", "t3"], parseAgentsPool("claude:haiku,codex:gpt-5.5"));
  return {
    agents: stringifyAssignments(agents),
    config: {
      defaultIntensity: resolveCouncilConfig().intensity,
      quickRoundOverride: config.rounds,
      thoroughWorker: resolveCouncilConfig({ flags: { intensity: "thorough" } }).worker
    },
    splitDestUrl: splitDestUrl("o", "n"),
    verify: {
      localized: localizeVerify("cd /workspace/services/foo && npm test", "/workspace", "/tmp/wt/T1"),
      relative: localizeVerify("npm test", "/workspace", "/tmp/wt/T1")
    },
    waves: planWaves(tasks)
  };
}
function taskForSelfTest(id, dependsOn) {
  return {
    boundaries: "Stay in scope",
    depends_on: dependsOn,
    difficulty: "moderate",
    id,
    model: "haiku",
    objective: `Task ${id}`,
    output_format: "Code edits",
    paths: [`${id}.txt`],
    title: id,
    verify: "npm test"
  };
}
function stringifyAssignments(assignments) {
  return Object.fromEntries(
    [...assignments.entries()].map(([taskId, engine]) => [taskId, `${engine.cli}:${engine.model}`])
  );
}
function requireConfigKey(key) {
  if (key === void 0) throw new Error("config action requires a key");
  if (!CONFIG_KEYS.includes(key)) {
    throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(", ")}`);
  }
  return key;
}
function omitKey(object, key) {
  return Object.fromEntries(Object.entries(object).filter(([k]) => k !== key));
}
function removeRootAssignment(source, key) {
  const lines = source.replace(/\r\n/gu, "\n").split("\n");
  let inTable = false;
  const kept = lines.filter((line) => {
    if (/^\s*\[/.test(line)) inTable = true;
    return inTable || !new RegExp(`^\\s*${escapeRegExp(key)}\\s*=`).test(line);
  });
  return kept.join("\n");
}
function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
async function writeTextFile(path, text) {
  await mkdir2(dirname2(path), { recursive: true });
  await writeFile(path, text, "utf8");
}
function isErrno2(error, code) {
  return error instanceof Error && "code" in error && error.code === code;
}
var CouncilApp;
var init_app = __esm({
  "src/app/index.ts"() {
    "use strict";
    init_fs();
    init_config();
    init_graph();
    init_tasks();
    init_triage();
    CouncilApp = class {
      gh;
      readText;
      writeText;
      constructor(deps = {}) {
        this.gh = deps.gh;
        this.readText = deps.readText ?? ((path) => readFile2(path, "utf8"));
        this.writeText = deps.writeText ?? writeTextFile;
      }
      plan(input = {}) {
        const config = resolveCouncilConfig(input.config === void 0 ? {} : { flags: input.config });
        const triage = input.triage ? classifyTriage(input.triage) : void 0;
        const taskLimit = triage?.route === "direct" ? 1 : void 0;
        return Promise.resolve({
          command: "plan",
          config,
          designRequested: input.design ?? false,
          directTierPolicy: "shrink-dag-only",
          executesWorkers: false,
          estimatedModelCalls: 2 + config.rounds * 4 + 1,
          ...input.runDir ? { runDir: input.runDir } : {},
          ...taskLimit === void 0 ? {} : { taskLimit },
          ...triage ? { triage } : {}
        });
      }
      async fanout(input) {
        const summary = await this.status({ runDir: input.runDir });
        const github = await this.maybeCreatePullRequest(input.github, input.dryRun, summary.run);
        return {
          github: github.kind,
          ...github.url ? { prUrl: github.url } : {},
          run: summary.run,
          tasks: summary.tasks,
          waves: summary.waves
        };
      }
      async fleet(input) {
        const tasks = await this.readTasksJson(input.tasksPath);
        const waves = planWaves(tasks);
        const ids = tasks.map((task) => task.id);
        const agents = assignAgents(ids, parseAgentsPool(input.agents));
        const github = await this.maybeCreatePullRequest(input.github, input.dryRun, basename2(input.tasksPath, ".json"));
        return {
          agents: stringifyAssignments(agents),
          github: github.kind,
          ...github.url ? { prUrl: github.url } : {},
          run: basename2(input.tasksPath, ".json"),
          tasks,
          waves
        };
      }
      async status(input) {
        const normalized = await normalizeLegacyRunDir(input.runDir);
        return {
          ...normalized.report ? { report: normalized.report } : {},
          run: normalized.runId,
          state: normalized.state,
          tasks: normalized.tasks,
          waves: normalized.report?.waves ?? planWaves(normalized.tasks),
          workerResults: [...normalized.workerResults.values()]
        };
      }
      async readReviewPack(input) {
        const summary = await this.status({ runDir: input.runDir });
        return {
          gate: input.gate,
          run: summary.run,
          task_count: summary.tasks.length,
          waves: summary.waves,
          worker_results: summary.workerResults.length
        };
      }
      async config(input) {
        if (input.action === "path") {
          return { paths: input.paths };
        }
        const user = await this.readOptionalConfig(input.paths.user);
        const project = await this.readOptionalConfig(input.paths.project);
        const target = input.project ? input.paths.project : input.paths.user;
        const current = input.project ? project : user;
        if (input.action === "show") {
          return {
            config: current,
            paths: input.paths,
            resolved: resolveCouncilConfig({ project, user }),
            target
          };
        }
        if (input.action === "get") {
          const key2 = requireConfigKey(input.key);
          const resolved = resolveCouncilConfig({ project, user });
          return { key: key2, paths: input.paths, resolved, value: resolved[key2], target };
        }
        if (input.action === "set") {
          const key2 = requireConfigKey(input.key);
          if (input.value === void 0) throw new Error("config set requires <key> <value>");
          const next2 = { ...current, [key2]: coerceConfigValue(key2, input.value) };
          await this.writeConfig(target, next2);
          return { config: next2, key: key2, paths: input.paths, target, value: next2[key2] };
        }
        const key = requireConfigKey(input.key);
        const next = omitKey(current, key);
        await this.writeConfig(target, next, key);
        return { config: next, key, paths: input.paths, target };
      }
      async roundTripTasksMarkdown(tasksPath) {
        const tasks = await this.readTasksJson(tasksPath);
        const records = tasks;
        const markdown = renderTasksMd(records);
        assertTasksBijection(records, markdown);
        return parseTasksMd(markdown);
      }
      async readTasksJson(path) {
        const parsed = JSON.parse(await this.readText(path));
        validateTasks(parsed);
        return parsed;
      }
      async maybeCreatePullRequest(github, dryRun, run) {
        if (!github) return { kind: "disabled" };
        if (dryRun) return { kind: "dry-run" };
        if (!this.gh) throw new Error("--github requires a gh adapter");
        const pr = await this.gh.createPullRequest({
          body: `Council run ${run}`,
          cwd: ".",
          draft: true,
          title: `Council ${run}`
        });
        return { kind: "created", url: pr.url };
      }
      async readOptionalConfig(path) {
        try {
          return parseCouncilConfig(await this.readText(path));
        } catch (error) {
          if (isErrno2(error, "ENOENT")) return {};
          throw error;
        }
      }
      async writeConfig(path, next, unsetKey) {
        let source = "";
        try {
          source = await this.readText(path);
        } catch (error) {
          if (!isErrno2(error, "ENOENT")) throw error;
        }
        const writableSource = unsetKey === void 0 ? source : removeRootAssignment(source, unsetKey);
        await this.writeText(path, writeCouncilConfig(writableSource, next));
      }
    };
  }
});

// src/cli/index.ts
init_app();
var COMMANDS = [
  { help: "validate amendment payloads and append them to a run", name: "amend" },
  { help: "show or change council.toml while preserving unrelated lines", name: "config" },
  { help: "assemble context packs for downstream stages", name: "context" },
  { help: "run design stages D0-D5", name: "design" },
  { help: "execute a planned task DAG", name: "fanout" },
  { help: "round-robin a task DAG across an explicit agent pool", name: "fleet" },
  { help: "adversarially question task readiness", name: "grill" },
  { help: "inject operator guidance into a supervised worker", name: "inject" },
  { help: "compose planning stages without auto-executing workers", name: "plan" },
  { help: "assemble checkpoint review packs", name: "review-pack" },
  { help: "run TS parity checks for Python self-test cases", name: "self-test" },
  { help: "extract a subtree into a destination repo", name: "split" },
  { help: "summarize a run directory", name: "status" },
  { help: "supervise a worker process with watchdog controls", name: "supervise" },
  { help: "survey repository context", name: "survey" },
  { help: "synchronize BMAD assets", name: "sync-bmad" },
  { help: "synchronize council skills", name: "sync-skills" },
  { help: "tail one task log", name: "tail" },
  { help: "classify request routing before planning", name: "triage" }
];
function commandRegistry() {
  return COMMANDS;
}
async function runCli(argv, runtime = {}) {
  const app = runtime.app ?? new CouncilApp();
  const [command, ...rest] = argv;
  try {
    if (command === void 0 || command === "--help" || command === "-h") {
      return ok(renderHelp());
    }
    if (command === "--self-test" || command === "self-test") {
      return ok(JSON.stringify(await appSelfTest(), null, 2));
    }
    if (!isCommand(command)) {
      return fail2(`unknown command: ${command}`);
    }
    switch (command) {
      case "plan":
        return okJson(await app.plan(parsePlan(rest)));
      case "fanout":
        return okJson(await app.fanout(parseFanout(rest)));
      case "fleet":
        return okJson(await app.fleet(parseFleet(rest)));
      case "config":
        return okJson(
          await app.config({
            ...parseConfig(rest),
            paths: runtime.configPaths ?? defaultConfigPaths()
          })
        );
      case "status":
        return okJson(await app.status({ runDir: requireFlag(parseFlags(rest), "run") }));
      case "review-pack":
        return okJson(await app.readReviewPack(parseReviewPack(rest)));
      case "triage":
        return okJson((await app.plan({ triage: parseTriage(rest) })).triage ?? {});
      case "design":
      case "amend":
      case "context":
      case "grill":
      case "inject":
      case "split":
      case "supervise":
      case "survey":
      case "sync-bmad":
      case "sync-skills":
      case "tail":
        return okJson({ command, compiled: true });
    }
    return fail2(`unknown command: ${command}`);
  } catch (error) {
    return fail2(error instanceof Error ? error.message : String(error));
  }
}
async function appSelfTest() {
  const { pythonSelfTestGolden: pythonSelfTestGolden2 } = await Promise.resolve().then(() => (init_app(), app_exports));
  return pythonSelfTestGolden2();
}
function parsePlan(argv) {
  const flags = parseFlags(argv);
  return {
    config: configOverrides(flags),
    design: flags.has("design"),
    ...flags.get("brief") ? { brief: requireFlag(flags, "brief") } : {},
    ...flags.get("run") ? { runDir: requireFlag(flags, "run") } : {},
    ...flags.has("triage") ? { triage: parseTriageFlag(requireFlag(flags, "triage")) } : {}
  };
}
function parseFanout(argv) {
  const flags = parseFlags(argv);
  return {
    dryRun: flags.has("dry-run"),
    github: flags.has("github"),
    runDir: requireFlag(flags, "run")
  };
}
function parseFleet(argv) {
  const flags = parseFlags(argv);
  return {
    agents: requireFlag(flags, "agents"),
    dryRun: flags.has("dry-run"),
    github: flags.has("github"),
    tasksPath: requireFlag(flags, "tasks")
  };
}
function parseConfig(argv) {
  const positional = argv.filter((arg) => !arg.startsWith("--"));
  const flags = parseFlags(argv);
  const action = positional[0];
  if (!isConfigAction(action)) throw new Error("config requires action show|get|set|unset|path");
  return {
    action,
    ...positional[1] ? { key: positional[1] } : {},
    project: flags.has("project"),
    ...positional[2] ? { value: positional[2] } : {}
  };
}
function parseReviewPack(argv) {
  const flags = parseFlags(argv);
  const gate = requireFlag(flags, "gate");
  if (gate !== "1" && gate !== "design" && gate !== "2") throw new Error("--gate must be 1, design, or 2");
  return { gate, runDir: requireFlag(flags, "run") };
}
function parseTriage(argv) {
  return parseTriageFlag(requireFlag(parseFlags(argv), "input"));
}
function parseTriageFlag(raw) {
  const parsed = JSON.parse(raw);
  return parsed;
}
function parseFlags(argv) {
  const flags = /* @__PURE__ */ new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg?.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[index + 1];
    if (next === void 0 || next.startsWith("--")) {
      flags.set(key, "true");
    } else {
      flags.set(key, next);
      index += 1;
    }
  }
  return flags;
}
function configOverrides(flags) {
  const config = {};
  const intensity = flags.get("intensity");
  const rounds = flags.get("rounds");
  const plannerA = flags.get("planner-a");
  const plannerB = flags.get("planner-b");
  const consolidator = flags.get("consolidator");
  const codexEffort = flags.get("codex-effort");
  if (intensity !== void 0) config.intensity = intensity;
  if (rounds !== void 0) config.rounds = Number.parseInt(rounds, 10);
  if (plannerA !== void 0) config.planner_a = plannerA;
  if (plannerB !== void 0) config.planner_b = plannerB;
  if (consolidator !== void 0) config.consolidator = consolidator;
  if (codexEffort !== void 0) config.codex_effort = codexEffort;
  return config;
}
function requireFlag(flags, name) {
  const value = flags.get(name);
  if (value === void 0 || value === "true") throw new Error(`--${name} is required`);
  return value;
}
function isCommand(value) {
  return COMMANDS.some((command) => command.name === value);
}
function isConfigAction(value) {
  return value === "show" || value === "get" || value === "set" || value === "unset" || value === "path";
}
function renderHelp() {
  return COMMANDS.map((command) => `${command.name}	${command.help}`).join("\n");
}
function defaultConfigPaths() {
  return {
    project: ".council.toml",
    user: `${process.env.HOME ?? "."}/.config/council/council.toml`
  };
}
function ok(stdout) {
  return { exitCode: 0, stderr: "", stdout: `${stdout.trimEnd()}
` };
}
function okJson(value) {
  return ok(JSON.stringify(value, null, 2));
}
function fail2(stderr) {
  return { exitCode: 2, stderr: `${stderr.trimEnd()}
`, stdout: "" };
}
if (import.meta.url === `file://${process.argv[1] ?? ""}`) {
  void runCli(process.argv.slice(2)).then((result) => {
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
    process.exitCode = result.exitCode;
  });
}
export {
  commandRegistry,
  runCli
};
