export function addJoinedOption(args, option, values) {
    if (values.length > 0) {
        args.push(option, values.join(','));
    }
}
export function optionalTrimmed(value) {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : undefined;
}
export function parseJsonArray(value) {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
}
export function parseJsonObject(value, guard) {
    const parsed = JSON.parse(value);
    return guard(parsed) ? parsed : undefined;
}
export function hasName(candidate) {
    return (candidate !== null &&
        typeof candidate === 'object' &&
        typeof candidate.name === 'string');
}
export function hasMilestoneShape(candidate) {
    return (candidate !== null &&
        typeof candidate === 'object' &&
        typeof candidate.number === 'number' &&
        typeof candidate.title === 'string');
}
export function hasPullRequestShape(candidate) {
    if (!candidate || typeof candidate !== 'object') {
        return false;
    }
    const pullRequest = candidate;
    return typeof pullRequest.number === 'number' && typeof pullRequest.url === 'string';
}
export function existingLabelName(label) {
    return typeof label === 'string' ? label : label.name;
}
export function uniqueStrings(values) {
    return [...new Set(values)];
}
export function isDryRun(context, options) {
    return options.dryRun ?? context.dryRun;
}
export function isBootstrapEnabled(context, options) {
    return options.githubBootstrap ?? context.githubBootstrap;
}
export function logFailure(context, method, error) {
    context.log(`github ${method} failed`, error);
}
