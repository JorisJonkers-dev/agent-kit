/**
 * What the architecture-review stage reads of a repository beside its tree
 * and its decision records: the README and the build configuration at the
 * root, and which of those declare a layer rule.
 */

/** The README the packet holds, the first the repository has. */
export const READMES = ['README.md', 'readme.md', 'README'] as const

/** Layer-rule configurations: a tool that fails a build when an import points the wrong way. Each declares which layer may depend on which. */
const LAYER_CONFIGS = [
  /^\.?dependency-cruiser(\.config)?\.(c|m)?js$/,
  /^\.?dependency-cruiser(\.config)?\.json$/,
  /^\.importlinter$/,
  /^tach\.toml$/,
  /^\.?arch-?unit\.(json|ya?ml|properties)$/,
  /^\.go-arch-lint\.ya?ml$/,
  /^deptrac\.ya?ml$/,
]

/** Build and project configuration worth a critic's reading, by root file name. */
const BUILD_CONFIGS = [
  /^package\.json$/,
  /^tsconfig(\.[\w-]+)?\.json$/,
  /^Cargo\.toml$/,
  /^pyproject\.toml$/,
  /^go\.mod$/,
  /^pom\.xml$/,
  /^build\.gradle(\.kts)?$/,
  /^settings\.gradle(\.kts)?$/,
  /^Makefile$/,
  /^justfile$/,
  /^docker-compose\.ya?ml$/,
]

/** How many build configurations a packet holds, and how many bytes of each: a config is evidence, not a second tree. */
export const BUILD_CONFIG_MAX_FILES = 12
export const BUILD_CONFIG_MAX_BYTES = 8192

/** Whether a root file is a layer-rule configuration. */
export function isLayerConfig(name: string): boolean {
  return LAYER_CONFIGS.some((pattern) => pattern.test(name))
}

/** The root files to read as the build configuration, layer configs first, then by name, at most `BUILD_CONFIG_MAX_FILES`. */
export function buildConfigs(rootFiles: readonly string[]): readonly string[] {
  const layers = rootFiles.filter(isLayerConfig)
  const others = rootFiles.filter((name) => !isLayerConfig(name) && BUILD_CONFIGS.some((pattern) => pattern.test(name)))
  return [...layers, ...others].slice(0, BUILD_CONFIG_MAX_FILES)
}

/** Whether a packet path is a layer-rule configuration held as a reference. */
export function isLayerConfigPath(packetPath: string): boolean {
  return packetPath.startsWith('references/') && isLayerConfig(packetPath.split('/').at(-1) ?? '')
}

/**
 * The checks of the `dependency-direction` lens, which assume a layer rule:
 * without a declared layer list, one of them is an unasked question, not a
 * defect. Their ids are the lens file's.
 */
export const DEPENDENCY_CHECKS: readonly string[] = ['inward-import', 'port-in-adapter', 'infrastructure-type-in-domain', 'rule-in-adapter']

/** Words that judge without measuring. A finding resting on one needs a fact or a quoted decision beside it. */
export const VERDICT_WORDS = /\b(cohesive|cohesion|leaky|leak(s|ing)?|anemic|tightly[- ]coupled|loosely[- ]coupled|clean|spaghetti|god[- ](class|object|module))\b/i
