export function optional(key, value) {
    return (value === undefined ? {} : { [key]: value });
}
