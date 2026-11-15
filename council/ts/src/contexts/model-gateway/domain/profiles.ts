/**
 * Delegation profiles: one named profile owns the models a delegation may use
 * and the tool scope it runs with. A request may name a profile or narrow it,
 * never widen it. The repository data policy always wins: it is intersected
 * with the profile, and a profile left with no allowed model is an error,
 * never a silent fallback.
 *
 * The profiles are rendered into the shipped defaults from
 * registry/delegation-profiles.yaml (scripts/render_registry.py).
 */
import {
  COST_CLASSES,
  ModelGatewayError,
  TIERS,
  TOOL_KINDS,
  type CostClass,
  type DelegationProfile,
  type GatewayConfig,
  type Tier,
  type ToolKind,
  type ToolList,
  type ToolScope,
} from './config.js'
import type { Narrowing } from './narrowing.js'
import type { PolicyDecision } from './policy.js'

const modelKey = (client: string, model: string): string => `${client}:${model}`

/** One model a profile leaves a request, with what the delegate picks by. */
export interface ProfileModel {
  readonly client: string
  readonly model: string
  readonly family: string
  readonly tier: Tier | null
  readonly cost_class: CostClass
}

export interface Resolution {
  /** The allowed models, cheapest first (cost class, then tier, then the profile's own order). */
  readonly models: readonly ProfileModel[]
  /** Why no model is allowed, or why the profile cannot serve the request; set exactly when `models` is empty. */
  readonly error?: string
}

function refused(message: string): ModelGatewayError {
  return new ModelGatewayError('profile-refused', message)
}

/** Refuses a narrowing that widens the profile: a model, a tool or a spend the profile does not give. */
export function requireNarrowing(profile: DelegationProfile, narrow: Narrowing | undefined): void {
  if (narrow === undefined) return
  for (const key of narrow.models ?? []) {
    if (!profile.models.includes(key)) throw refused(`narrow.models names ${key}, which profile ${profile.name} does not allow: a request may narrow a profile, never widen it`)
  }
  for (const kind of TOOL_KINDS) {
    const asked = narrow[kind]
    const given = profile.tools[kind]
    if (asked === undefined || given === 'any') continue
    const extra = asked.find((item) => !given.includes(item))
    if (extra !== undefined) {
      throw refused(`narrow.${kind} names ${extra}, which profile ${profile.name} does not grant (its ${kind} scope is ${given.length === 0 ? 'deny-all' : given.join(', ')}): a request may narrow a profile, never widen it`)
    }
  }
  if (profile.max_spend_usd !== undefined && narrow.max_spend_usd !== undefined && narrow.max_spend_usd > profile.max_spend_usd) {
    throw refused(`narrow.max_spend_usd ${String(narrow.max_spend_usd)} is above profile ${profile.name}'s cap of ${String(profile.max_spend_usd)}: a request may narrow a profile, never widen it`)
  }
}

/** The tool scope a run holds: the profile's, narrowed. */
export function effectiveTools(profile: DelegationProfile, narrow: Narrowing | undefined): ToolScope {
  const pick = (kind: ToolKind): ToolList => narrow?.[kind] ?? profile.tools[kind]
  return { mcp: pick('mcp'), plugins: pick('plugins'), skills: pick('skills') }
}

/** The spend cap a run holds: the narrower of the profile's and the request's. */
export function effectiveSpend(profile: DelegationProfile, narrow: Narrowing | undefined): number | undefined {
  const caps = [profile.max_spend_usd, narrow?.max_spend_usd].filter((cap): cap is number => cap !== undefined)
  return caps.length === 0 ? undefined : Math.min(...caps)
}

/** Whether a skill or plugin entry is one plain name: the folder or file it names in Pi's home, never a path that reaches out of it. */
export function plainName(name: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name)
}

/**
 * The limit the chosen client cannot enforce, as a reason, or undefined when
 * it can hold the scope. Only Pi enforces: its per-run home is built from the
 * scope, so it holds a list of skills and plugins (extensions) by name, and
 * holds mcp at none. It cannot hold `any` (its home is built only from named
 * items) or an MCP server (the confined launch reaches only the route's proxy
 * port and starts no other program, so a server could never connect). Every
 * other client is unable to hold a restricted scope.
 */
export function unenforceable(profileName: string, client: string, isPi: boolean, tools: ToolScope): string | undefined {
  const restricted = TOOL_KINDS.filter((kind) => tools[kind] !== 'any')
  if (!isPi) {
    if (restricted.length === 0) return undefined
    return `profile ${profileName} restricts ${restricted.join(', ')} (a scope is deny-all unless the profile lists it), and ${client} cannot enforce a tool scope: only Pi does`
  }
  const open = TOOL_KINDS.filter((kind) => tools[kind] === 'any')
  if (open.length > 0) {
    return `profile ${profileName} leaves ${open.join(', ')} open with any, and ${client} builds its per-run home only from named items: list the ${open.join(', ')} it needs, or leave them out for deny-all`
  }
  if (tools.mcp !== 'any' && tools.mcp.length > 0) {
    return `profile ${profileName} grants mcp ${tools.mcp.join(', ')}, and ${client} cannot enforce it: Pi has a built-in MCP client, but a confined launch reaches only the route's proxy port and starts no other program, so a listed server could never connect; mcp stays deny-all on Pi`
  }
  for (const kind of ['skills', 'plugins'] as const) {
    const list = tools[kind]
    const bad = list === 'any' ? undefined : list.find((item) => !plainName(item))
    if (bad !== undefined) {
      return `profile ${profileName} lists ${kind} entry ${bad}, which is not a plain name: a ${kind === 'skills' ? 'skill' : 'plugin'} is named by the folder or file it is in Pi's home, never a path`
    }
  }
  return undefined
}

/** The tool scope a run holds, or undefined when it names no profile (or one the defaults do not state). */
export function runScope(config: GatewayConfig, profile: string | undefined, narrow: Narrowing | undefined): ToolScope | undefined {
  const found = profile === undefined ? undefined : config.profiles[profile]
  return found === undefined ? undefined : effectiveTools(found, narrow)
}

/** A tool scope as the ledger writes it: each kind a list of names, or `any`, in a fixed order. */
export function toolsJson(scope: ToolScope): ToolScope {
  return { mcp: scope.mcp, plugins: scope.plugins, skills: scope.skills }
}

/**
 * The models of the profile a request may use. `available` is what the data
 * policy permits (and, for a listing, what is usable now), so the policy is
 * the intersection; a profile with nothing left is an error with its reason.
 * With `forRun`, a profile that leaves out the writer's family needs the
 * writer's family named.
 */
export function resolveProfile(
  profile: DelegationProfile,
  available: readonly ProfileModel[],
  policyReason: string,
  narrow: Narrowing | undefined,
  forRun: boolean,
): Resolution {
  const asked = narrow?.models ?? profile.models
  const pinned = profile.pin === undefined ? '' : ` (pinned to ${profile.pin})`
  const wanted = profile.models.filter((key) => asked.includes(key) && (profile.pin === undefined || key === profile.pin))
  const permitted = wanted.flatMap((key) => {
    const found = available.find((model) => `${model.client}:${model.model}` === key)
    return found === undefined ? [] : [found]
  })
  if (permitted.length === 0) {
    return {
      error: `profile ${profile.name} has no allowed model${pinned}: none of its ${String(wanted.length)} models is both usable now and permitted by the data policy (${policyReason})`,
      models: [],
    }
  }
  let left = permitted
  if (profile.exclude_writer_family === true) {
    const writer = narrow?.writer_family
    if (writer === undefined && forRun) {
      return { error: `profile ${profile.name} leaves out the writer's family, so the request must name it as narrow.writer_family`, models: [] }
    }
    if (writer !== undefined) {
      left = permitted.filter((model) => model.family !== writer)
      if (left.length === 0) {
        return { error: `profile ${profile.name} has no allowed model once the writer's family ${writer} is left out`, models: [] }
      }
    }
  }
  const families = [...new Set(left.map((model) => model.family))].sort()
  if (profile.distinct_families !== undefined && families.length < profile.distinct_families) {
    return {
      error: `profile ${profile.name} needs ${String(profile.distinct_families)} distinct families but only ${String(families.length)} remain (${families.join(', ')})`,
      models: [],
    }
  }
  const COST_RANK: readonly CostClass[] = COST_CLASSES
  const TIER_RANK: readonly Tier[] = TIERS
  const order = (model: ProfileModel): number => profile.models.indexOf(`${model.client}:${model.model}`)
  const sorted = [...left].sort(
    (a, b) =>
      COST_RANK.indexOf(a.cost_class) - COST_RANK.indexOf(b.cost_class) ||
      (a.tier === null ? TIER_RANK.length : TIER_RANK.indexOf(a.tier)) - (b.tier === null ? TIER_RANK.length : TIER_RANK.indexOf(b.tier)) ||
      order(a) - order(b),
  )
  return { models: sorted }
}

/** What a run asks of its profile: the profile's name, the narrowing, and the client model it was sent to. */
export interface ProfiledRun {
  readonly profile?: string
  readonly narrow?: Narrowing
  readonly client: string
  readonly model: string
}

/** The named profile, or the refusal that says the defaults state no such profile. */
export function requireProfile(config: GatewayConfig, name: string): DelegationProfile {
  const profile = config.profiles[name]
  if (profile === undefined) {
    const known = Object.keys(config.profiles).sort()
    throw new ModelGatewayError('unknown-profile', `no delegation profile ${name}: the defaults state ${known.length === 0 ? 'none' : known.join(', ')}`)
  }
  return profile
}

/**
 * The models a run's profile leaves it for the repository the decision was
 * made about: the catalogue's models the data policy permits, cut to the
 * profile's (narrowed) models.
 */
export function profileModels(config: GatewayConfig, decision: PolicyDecision, run: ProfiledRun & { readonly profile: string }): Resolution {
  const profile = requireProfile(config, run.profile)
  requireNarrowing(profile, run.narrow)
  const permitted = new Set(
    decision.permissions.filter((permission) => permission.permitted).map((permission) => modelKey(permission.client, permission.model)),
  )
  const available = config.models.flatMap((entry): ProfileModel[] => {
    const client = config.clients[entry.client]
    if (client === undefined || !permitted.has(modelKey(entry.client, entry.model))) return []
    return [{ client: entry.client, cost_class: client.cost_class, family: entry.family, model: entry.model, tier: entry.tier }]
  })
  return resolveProfile(profile, available, decision.reason, run.narrow, true)
}

/**
 * Holds a run to its profile, or refuses it with the reason: the profile
 * leaves no allowed model for the repository, the client model is not one of
 * them, the request widens the profile, or the profile's tool scope names a
 * limit the client cannot enforce. A run with no profile is held to nothing.
 */
export function holdToProfile(config: GatewayConfig, decision: PolicyDecision, run: ProfiledRun, isPi: boolean): void {
  if (run.profile === undefined) return
  const resolution = profileModels(config, decision, { ...run, profile: run.profile })
  if (resolution.error !== undefined) throw refused(resolution.error)
  if (!resolution.models.some((model) => model.client === run.client && model.model === run.model)) {
    const allowed = resolution.models.map((model) => modelKey(model.client, model.model)).join(', ')
    throw refused(`${modelKey(run.client, run.model)} is not allowed by profile ${run.profile} here: the profile and the data policy leave ${allowed}`)
  }
  const profile = requireProfile(config, run.profile)
  const reason = unenforceable(run.profile, run.client, isPi, effectiveTools(profile, run.narrow))
  if (reason !== undefined) throw refused(reason)
}

/** One profile as `models` lists it: what it allows for the repository the caller works in, or why it allows nothing. */
export interface ProfileListing {
  readonly name: string
  readonly description: string
  readonly tools: ToolScope
  /** The profile's models this machine can use now and the data policy permits, cheapest first. */
  readonly models: readonly ProfileModel[]
  readonly pin?: string
  readonly ladder?: readonly string[]
  readonly distinct_families?: number
  readonly exclude_writer_family?: boolean
  readonly max_spend_usd?: number
  /** Why no model is allowed; set exactly when `models` is empty. */
  readonly error?: string
}

/** Every profile, by name, with the models it leaves among the ones `models` lists as permitted. */
export function listProfiles(config: GatewayConfig, permitted: readonly ProfileModel[], policyReason: string): readonly ProfileListing[] {
  return Object.values(config.profiles)
    .sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0))
    .map((profile): ProfileListing => {
      const resolution = resolveProfile(profile, permitted, policyReason, undefined, false)
      return {
        description: profile.description,
        models: resolution.models.map((model) => ({
          client: model.client,
          cost_class: model.cost_class,
          family: model.family,
          model: model.model,
          tier: model.tier,
        })),
        name: profile.name,
        tools: profile.tools,
        ...(profile.pin === undefined ? {} : { pin: profile.pin }),
        ...(profile.ladder === undefined ? {} : { ladder: profile.ladder }),
        ...(profile.distinct_families === undefined ? {} : { distinct_families: profile.distinct_families }),
        ...(profile.exclude_writer_family === undefined ? {} : { exclude_writer_family: profile.exclude_writer_family }),
        ...(profile.max_spend_usd === undefined ? {} : { max_spend_usd: profile.max_spend_usd }),
        ...(resolution.error === undefined ? {} : { error: resolution.error }),
      }
    })
}
