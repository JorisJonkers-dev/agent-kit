import { graphFromNodes } from './graph-factory.js';
export function compactClosedTasks(graph) {
    const nodes = new Map();
    for (const node of graph.nodes.values()) {
        if (node.state !== 'closed') {
            const task = {
                ...node.task,
                depends_on: node.task.depends_on.filter((dependency) => {
                    const dependencyNode = graph.nodes.get(dependency);
                    return dependencyNode?.state !== 'closed';
                }),
            };
            nodes.set(node.task.id, { ...node, task });
        }
    }
    return graphFromNodes(nodes, graph.idStrategy, graph.nextOrdinal);
}
