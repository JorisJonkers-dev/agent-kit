import { createHash } from 'node:crypto';
import { projectWaveView } from './waves.js';
export function createTaskGraph(drafts, options = {}) {
    const idStrategy = options.idStrategy ?? 'legacy-ordinal';
    const nodes = new Map();
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
            state: 'pending',
            task: normalizeTask(draft, minted, contentHash),
        });
    });
    const graph = graphFromNodes(nodes, idStrategy, nextOrdinal);
    assertKnownDependencies(graph);
    assertAcyclic(graph);
    return graph;
}
export function ingestDiscoveredWork(graph, batch) {
    if (!graph.nodes.has(batch.sourceId)) {
        throw new Error(`unknown discovery source: ${batch.sourceId}`);
    }
    const nodes = new Map(graph.nodes);
    const deduped = [];
    const ingested = [];
    let nextOrdinal = graph.nextOrdinal;
    batch.tasks.forEach((draft) => {
        const contentHash = taskContentHash(draft);
        const existingId = firstContentOwner(nodes, contentHash);
        if (existingId !== undefined) {
            deduped.push({ content_hash: contentHash, existing_id: existingId });
            return;
        }
        const minted = draft.id ?? mintTaskId(contentHash, graph.idStrategy, nodes, nextOrdinal);
        const duplicate = nodes.get(minted);
        if (duplicate !== undefined) {
            throw new Error(`duplicate task id: ${minted}`);
        }
        nextOrdinal = nextOrdinalFor(minted, nextOrdinal);
        const discoveredDraft = { ...draft, discovered_from: batch.sourceId };
        nodes.set(minted, {
            blocked_by: [],
            order: nodes.size,
            state: 'pending',
            task: normalizeTask(discoveredDraft, minted, contentHash),
        });
        ingested.push(minted);
    });
    const nextGraph = graphFromNodes(nodes, graph.idStrategy, nextOrdinal);
    assertKnownDependencies(nextGraph);
    assertAcyclic(nextGraph);
    return { deduped, graph: nextGraph, ingested };
}
export function graphFromNodes(nodes, idStrategy, nextOrdinal) {
    return {
        contentIndex: contentIndexFor(nodes),
        edges: edgesFor(nodes),
        idStrategy,
        nextOrdinal,
        nodes,
    };
}
function normalizeTask(draft, id, contentHash) {
    return {
        ...draft,
        content_hash: contentHash,
        depends_on: draft.depends_on ?? [],
        id,
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
    if (value === null || typeof value !== 'object') {
        const serialized = JSON.stringify(value);
        return typeof serialized === 'string' ? serialized : 'undefined';
    }
    if (Array.isArray(value)) {
        return `[${value.map((entry) => stableJson(entry)).join(',')}]`;
    }
    const record = value;
    return `{${Object.keys(record)
        .sort()
        .map((key) => `${JSON.stringify(key)}:${stableJson(record[key])}`)
        .join(',')}}`;
}
function digest(value) {
    return createHash('sha256').update(value).digest('hex');
}
function mintTaskId(contentHash, idStrategy, nodes, nextOrdinal) {
    if (idStrategy === 'content-hash') {
        let suffix = 12;
        let candidate = `ck-${contentHash.slice(0, suffix)}`;
        let collision = 2;
        while (nodes.has(candidate)) {
            if (suffix < contentHash.length) {
                suffix += 1;
                candidate = `ck-${contentHash.slice(0, suffix)}`;
            }
            else {
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
        if (draft.id?.startsWith('T') !== true) {
            return next;
        }
        const ordinal = Number.parseInt(draft.id.slice(1), 10);
        return Number.isFinite(ordinal) ? Math.max(next, ordinal + 1) : next;
    }, 1);
}
function nextOrdinalFor(id, nextOrdinal) {
    if (!id.startsWith('T')) {
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
                edges.push({ from: dependency, kind: 'depends_on', to: node.task.id });
            }
        });
        if (isKnownTaskId(nodes, node.task.discovered_from)) {
            edges.push({ from: node.task.discovered_from, kind: 'discovered_from', to: node.task.id });
        }
        node.task.supersedes?.forEach((superseded) => {
            if (nodes.has(superseded)) {
                edges.push({ from: superseded, kind: 'supersedes', to: node.task.id });
            }
        });
    }
    return edges;
}
function contentIndexFor(nodes) {
    const index = new Map();
    for (const node of nodes.values()) {
        if (!index.has(node.task.content_hash ?? '')) {
            index.set(node.task.content_hash ?? '', node.task.id);
        }
    }
    return index;
}
function isKnownTaskId(nodes, value) {
    return value !== undefined && nodes.has(value);
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
function firstContentOwner(nodes, contentHash) {
    for (const node of nodes.values()) {
        if (node.task.content_hash === contentHash) {
            return node.task.id;
        }
    }
    return undefined;
}
export function planWaves(drafts, options = {}) {
    return projectWaveView(createTaskGraph(drafts, options));
}
