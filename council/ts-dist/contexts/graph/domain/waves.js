export function projectWaveView(graph) {
    const remaining = new Map();
    for (const node of graph.nodes.values()) {
        remaining.set(node.task.id, node.task.depends_on.filter((dependency) => graph.nodes.has(dependency)));
    }
    const waves = [];
    const done = new Set();
    while (remaining.size > 0) {
        const ready = [...remaining.entries()]
            .filter(([, dependencies]) => dependencies.every((dependency) => done.has(dependency)))
            .map(([id]) => id)
            .sort();
        if (ready.length === 0) {
            throw new Error(`dependency cycle among tasks: ${[...remaining.keys()].sort().join(', ')}`);
        }
        waves.push(ready);
        ready.forEach((id) => {
            done.add(id);
            remaining.delete(id);
        });
    }
    return waves;
}
