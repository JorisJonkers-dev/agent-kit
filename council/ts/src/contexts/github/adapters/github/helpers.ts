import type {
  ExistingGithubLabel,
  ExistingGithubMilestone,
} from '../../../github/index.js'
import type { GhPr } from '../../../../ports/index.js'
import type { GithubAdapterContext, MethodOptions } from './types.js'

export function addJoinedOption(
  args: string[],
  option: string,
  values: readonly string[],
): void {
  if (values.length > 0) {
    args.push(option, values.join(','))
  }
}

export function optionalTrimmed(value: string): string | undefined {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : undefined
}

export function parseJsonArray(value: string): readonly unknown[] {
  const parsed: unknown = JSON.parse(value)
  return Array.isArray(parsed) ? parsed : []
}

export function parseJsonObject<T>(
  value: string,
  guard: (candidate: unknown) => candidate is T,
): T | undefined {
  const parsed: unknown = JSON.parse(value)
  return guard(parsed) ? parsed : undefined
}

export function hasName(candidate: unknown): candidate is ExistingGithubLabel {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubLabel>).name === 'string'
  )
}

export function hasMilestoneShape(
  candidate: unknown,
): candidate is ExistingGithubMilestone {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubMilestone>).number === 'number' &&
    typeof (candidate as Partial<ExistingGithubMilestone>).title === 'string'
  )
}

export function hasPullRequestShape(candidate: unknown): candidate is GhPr {
  if (!candidate || typeof candidate !== 'object') {
    return false
  }

  const pullRequest = candidate as Partial<GhPr>
  return typeof pullRequest.number === 'number' && typeof pullRequest.url === 'string'
}

export function existingLabelName(label: ExistingGithubLabel | string): string {
  return typeof label === 'string' ? label : label.name
}

export function uniqueStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)]
}

export function isDryRun(context: GithubAdapterContext, options: MethodOptions): boolean {
  return options.dryRun ?? context.dryRun
}

export function isBootstrapEnabled(
  context: GithubAdapterContext,
  options: MethodOptions,
): boolean {
  return options.githubBootstrap ?? context.githubBootstrap
}

export function logFailure(
  context: GithubAdapterContext,
  method: string,
  error: unknown,
): void {
  context.log(`github ${method} failed`, error)
}
