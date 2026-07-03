export function applyBoundsGate(input) {
    const outOfBounds = findOutOfBoundsFiles(input.filesChanged, input.allowedPaths, input.taskId);
    return {
        files_changed: input.filesChanged,
        out_of_bounds: outOfBounds,
        status: outOfBounds.length > 0 ? 'out-of-bounds' : (input.status ?? 'ok'),
    };
}
export function findOutOfBoundsFiles(filesChanged, allowedPaths, taskId) {
    const allowed = new Set(allowedPaths);
    const storyPath = `workers/${taskId}/story.md`;
    return filesChanged.filter((file) => !allowed.has(file) && file !== storyPath);
}
