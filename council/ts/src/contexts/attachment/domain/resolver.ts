import { resolveContextProfile } from '../../context/index.js'
import { recommendLenses } from '../../triage/index.js'
import type { TaskResolvedAttachment } from '../../../shared-kernel/index.js'
import type { ResolvedContextProfile } from '../../context/index.js'
import type { LensProblemProfile } from '../../triage/index.js'

export interface AttachmentResolutionInput {
  readonly archetype?: string
  readonly clarity?: LensProblemProfile['clarity']
  readonly contextProfile?: string
  readonly landscape?: LensProblemProfile['landscape']
  readonly parallelism?: LensProblemProfile['parallelism']
  readonly paths?: readonly string[]
  readonly risk?: LensProblemProfile['risk']
  readonly signals?: readonly string[]
  readonly size?: LensProblemProfile['size']
  readonly taskKind?: string
}

export interface AttachmentResolverCatalog {
  readonly profiles: readonly AttachmentResolverProfile[]
}

export interface AttachmentResolverProfile {
  readonly fullSkills: readonly string[]
  readonly mcpProfile: string
  readonly name: string
  readonly skillCards: readonly AttachmentResolverSkillCard[]
}

export interface AttachmentResolverSkillCard {
  readonly name: string
  readonly negativeTriggers: readonly string[]
  readonly positiveTriggers: readonly string[]
  readonly requiredMcpProfile: string
}

export interface SelectedAttachmentSkillCard {
  readonly name: string
  readonly requiredMcpProfile: string
  readonly score: number
}

export interface RejectedAttachmentSkillCard {
  readonly name: string
  readonly reason: 'low-fit' | 'negative-trigger'
  readonly score: number
}

export interface ResolvedAttachments extends TaskResolvedAttachment {
  readonly contextProfile: ResolvedContextProfile
  readonly lensIds: readonly string[]
  readonly rejectedSkillCards: readonly RejectedAttachmentSkillCard[]
  readonly selectedProfiles: readonly string[]
  readonly selectedSkillCards: readonly SelectedAttachmentSkillCard[]
}

interface ScoredSkillCard {
  readonly negativeMatches: number
  readonly profileName: string
  readonly skillCard: AttachmentResolverSkillCard
  readonly score: number
}

const ACTIVE_SKILL_CAP = 12
const POSITIVE_TRIGGER_SCORE = 14
const NEGATIVE_TRIGGER_SCORE = 28

const EMPTY_ATTACHMENT_PROFILE: AttachmentResolverProfile = {
  fullSkills: [],
  mcpProfile: 'minimal',
  name: 'minimal',
  skillCards: [],
}

const MCP_PROFILE_RANKS = new Map<string, number>([
  ['minimal', 0],
  ['frontend', 1],
  ['cluster', 2],
  ['code-intel', 3],
  ['full-diagnostic', 4],
])

const PATH_SIGNAL_RULES: readonly {
  readonly pattern: RegExp
  readonly signals: readonly string[]
}[] = [
  { pattern: /(^|\/)(app-ui|frontend|components)(\/|$)|\.(css|scss|vue|jsx|tsx)$/u, signals: ['frontend', 'ui', 'component', 'app-ui'] },
  { pattern: /fleet\.ya?ml|kubernetes|k8s|helm|ingress|traefik|platform\/(inventory|rendered)/u, signals: ['cluster', 'fleet.yaml', 'kubernetes', 'traefik'] },
  { pattern: /dependency-cruiser|tsconfig|eslint|src\/contexts|\/domain\/|barrel/u, signals: ['code intel', 'strict typescript', 'cross-context', 'barrel'] },
  { pattern: /(^|\/)(ci|workflows?)(\/|\.|$)|\.github|test-logs?|trace/u, signals: ['diagnostic', 'failing ci', 'test logs', 'trace'] },
]

export function resolveAttachments(
  input: AttachmentResolutionInput = {},
  catalog: AttachmentResolverCatalog,
): ResolvedAttachments {
  const contextProfile = resolveContextProfile({
    ...(input.archetype === undefined ? {} : { archetype: input.archetype }),
    ...(input.contextProfile === undefined ? {} : { context_profile: input.contextProfile }),
  })
  const recommendation = recommendLenses(toLensProblemProfile(input))
  const evidence = buildEvidence(input, contextProfile)
  const scoredCards = catalog.profiles.flatMap((profile) =>
    profile.skillCards.map((skillCard) => scoreSkillCard(profile.name, skillCard, evidence)),
  )
  const selectedCards = scoredCards.filter((card) => card.score > 0 && card.negativeMatches === 0).sort(compareScoredCards)
  const rejectedCards = scoredCards
    .filter((card) => card.score <= 0 || card.negativeMatches > 0)
    .map(toRejectedSkillCard)
    .sort(compareRejectedCards)
  const selectedProfile = selectProfile(catalog.profiles, selectedCards)
  const selectedMcpProfile = maxMcpProfile(
    selectedProfile.mcpProfile,
    selectedCards.map((card) => card.skillCard.requiredMcpProfile),
  )

  return {
    activeSkills: uniqueInOrder([
      ...selectedCards.map((card) => card.skillCard.name),
      ...selectedProfile.fullSkills,
    ]).slice(0, ACTIVE_SKILL_CAP),
    contextProfile,
    lensIds: recommendation.lenses.map((lens) => lens.id),
    mcpProfile: selectedMcpProfile,
    rejectedSkillCards: rejectedCards,
    selectedProfiles: [selectedProfile.name],
    selectedSkillCards: selectedCards.map((card) => ({
      name: card.skillCard.name,
      requiredMcpProfile: card.skillCard.requiredMcpProfile,
      score: card.score,
    })),
  }
}

function toLensProblemProfile(input: AttachmentResolutionInput): LensProblemProfile {
  return {
    ...(input.size === undefined ? {} : { size: input.size }),
    ...(input.taskKind === undefined ? {} : { kind: input.taskKind }),
    ...(input.risk === undefined ? {} : { risk: input.risk }),
    ...(input.landscape === undefined ? {} : { landscape: input.landscape }),
    ...(input.clarity === undefined ? {} : { clarity: input.clarity }),
    ...(input.parallelism === undefined ? {} : { parallelism: input.parallelism }),
    signals: [...(input.signals ?? []), ...pathSignals(input.paths ?? [])],
  }
}

function buildEvidence(
  input: AttachmentResolutionInput,
  contextProfile: ResolvedContextProfile,
): readonly string[] {
  return [
    input.taskKind,
    contextProfile.profile,
    contextProfile.archetype,
    contextProfile.repo_context,
    contextProfile.skills,
    contextProfile.mcp,
    ...(input.signals ?? []),
    ...(input.paths ?? []),
    ...pathSignals(input.paths ?? []),
  ].filter((value): value is string => value !== undefined && value.trim().length > 0)
}

function pathSignals(paths: readonly string[]): readonly string[] {
  return paths.flatMap((path) => [
    path,
    ...PATH_SIGNAL_RULES.filter((rule) => rule.pattern.test(path.toLowerCase())).flatMap((rule) => rule.signals),
  ])
}

function scoreSkillCard(
  profileName: string,
  skillCard: AttachmentResolverSkillCard,
  evidence: readonly string[],
): ScoredSkillCard {
  const positiveMatches = skillCard.positiveTriggers.filter((trigger) => evidenceMatches(trigger, evidence)).length
  const negativeMatches = skillCard.negativeTriggers.filter((trigger) => evidenceMatches(trigger, evidence)).length
  return {
    negativeMatches,
    profileName,
    score: positiveMatches * POSITIVE_TRIGGER_SCORE - negativeMatches * NEGATIVE_TRIGGER_SCORE,
    skillCard,
  }
}

function selectProfile(
  profiles: readonly AttachmentResolverProfile[],
  selectedCards: readonly ScoredSkillCard[],
): AttachmentResolverProfile {
  const profileScores = profiles
    .map((profile) => ({
      profile,
      score: selectedCards
        .filter((card) => card.profileName === profile.name)
        .reduce((score, card) => score + card.score, 0),
    }))
    .sort((left, right) => right.score - left.score || left.profile.name.localeCompare(right.profile.name))
  const bestProfile = profileScores.find((profile) => profile.score > 0)?.profile
  const fallbackProfile = profiles.reduce<AttachmentResolverProfile>(
    (selected, profile) =>
      selected === EMPTY_ATTACHMENT_PROFILE || profile.name.localeCompare(selected.name) < 0 ? profile : selected,
    EMPTY_ATTACHMENT_PROFILE,
  )
  return bestProfile ?? profileScores.find((profile) => profile.profile.mcpProfile === 'minimal')?.profile ?? fallbackProfile
}

function maxMcpProfile(baseProfile: string, requiredProfiles: readonly string[]): string {
  return [baseProfile, ...requiredProfiles].reduce((selected, candidate) =>
    mcpRank(candidate) > mcpRank(selected) ? candidate : selected,
  )
}

function mcpRank(profile: string): number {
  return MCP_PROFILE_RANKS.get(profile) ?? 0
}

function evidenceMatches(trigger: string, evidence: readonly string[]): boolean {
  const normalizedTrigger = normalizePhrase(trigger)
  const triggerTokens = tokenize(normalizedTrigger)
  const normalizedEvidence = evidence.map(normalizePhrase)
  const evidenceText = normalizedEvidence.join(' ')
  const evidenceTokens = new Set(normalizedEvidence.flatMap(tokenize))
  return triggerTokens.length === 1
    ? evidenceTokens.has(triggerTokens[0] ?? normalizedTrigger)
    : evidenceText.includes(normalizedTrigger) || triggerTokens.every((token) => evidenceTokens.has(token))
}

function normalizePhrase(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/gu, ' ')
}

function tokenize(value: string): readonly string[] {
  return value.split(/[^a-z0-9]+/u).filter((token) => token.length > 0)
}

function toRejectedSkillCard(card: ScoredSkillCard): RejectedAttachmentSkillCard {
  return {
    name: card.skillCard.name,
    reason: card.negativeMatches > 0 ? 'negative-trigger' : 'low-fit',
    score: card.score,
  }
}

function compareScoredCards(left: ScoredSkillCard, right: ScoredSkillCard): number {
  return right.score - left.score || left.skillCard.name.localeCompare(right.skillCard.name)
}

function compareRejectedCards(
  left: RejectedAttachmentSkillCard,
  right: RejectedAttachmentSkillCard,
): number {
  return left.name.localeCompare(right.name)
}

function uniqueInOrder(values: readonly string[]): readonly string[] {
  return [...new Set(values)]
}
