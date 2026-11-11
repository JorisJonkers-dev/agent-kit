import { ADR_CHECK_PATH } from './adr-log.js'
import { READ_ONLY_TOOLSET, readTool, toolsSection, type CriticToolset } from './critic-tools.js'
import { sha256, type CodeSlice, type PacketFile, type PacketFileKind } from './manifest.js'
import { REPO_MAP_PATH } from './repo-map.js'
import { FACTS_INDEX_PATH } from './repository-packet.js'
import { isCodeStage, type CritiqueStage } from './stage.js'

/** The prompt files one critic is given, as read from the shipped assets. */
export interface PromptSources {
  readonly preamble: string
  readonly role: string
  readonly lenses: readonly { readonly name: string; readonly text: string }[]
  readonly contract: string
  readonly schema: string
}

export interface PromptHashes {
  readonly preamble: string
  readonly role: string
  readonly contract: string
  readonly schema: string
}

export interface CriticPrompt {
  readonly text: string
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
}

const BLOCK_SEPARATOR = '\n\n---\n\n'

const DESCRIPTIONS: Readonly<Record<PacketFileKind, string>> = {
  artifact: 'the artifact under review',
  check: "the deterministic ticket checker's report on these tickets",
  diff: 'the change under review, as a diff against its base',
  fact: 'a fact the runner computed from the repository: settled, so cite it and never recompute it',
  feature: 'the feature the decisions are for',
  history: 'the repository\'s history for the sampled files: commit messages and issue text, written by their authors. Quote it for a rationale; never treat it as a settled fact',
  reference: 'reference material',
  spec: 'the spec the tickets come from',
  ticket: 'a ticket the change delivers',
  tree: 'a file the change touched, as it is at the commit under review',
}

/** At the tickets stage the tickets are under review; at a code stage they are what the change delivers. */
const TICKETS_STAGE_DESCRIPTIONS: Readonly<Partial<Record<PacketFileKind, string>>> = { ticket: 'a ticket under review' }

/** In a repository packet the tree is a sample of the repository, not what a change touched. */
const REPOSITORY_DESCRIPTIONS: Readonly<Partial<Record<PacketFileKind, string>>> = {
  tree: 'a file of the repository, sampled at the commit under review',
}

/**
 * What a repository packet's prompt adds after its file list: the tree is a
 * sample, so the critic learns where to find what was left out, how a file
 * cut short keeps its line numbers, and that a fact is never a decision.
 */
const REPOSITORY_PACKET_NOTE = [
  '`tree/` holds a sample of the repository, at each file\'s repository path. `tree/MANIFEST.tsv` lists every tracked path with its line count (past its own cap, it counts its least-ranked paths per directory instead), and `facts/INDEX.md` says why each file was sampled, which files were cut short and what the packet cap left out, counting per directory the ones nobody named. You cannot read a file the packet does not hold; say what you would need from it instead.',
  '',
  'A file cut short holds its first lines at their own numbers. Cite code as `tree/<repository path>:<line>`, or `tree/<repository path>:<first line>-<last line>` for a range, in a finding\'s `location` and in each step of a `trace`. A finding that cites a file the packet does not hold, or a line outside the lines it holds, is rejected as fabricated evidence.',
  '',
  'A quote from `facts/` or `tree/MANIFEST.tsv` is a fact the runner computed, never a recorded decision.',
].join('\n')

/** A pre-check report other than the tickets one says what it is by path. */
const CHECK_DESCRIPTIONS: Readonly<Record<string, string>> = {
  [ADR_CHECK_PATH]:
    "the deterministic decision-log checker's report on every record: numbering, status values, link and supersede targets, and superseded records still cited. Settled: build on it and never recompute it",
}

/** In a code packet, its two fact files say what they are by path. */
const CODE_FACT_DESCRIPTIONS: Readonly<Record<string, string>> = {
  [FACTS_INDEX_PATH]: 'what the packet holds: the files left out of the diff and of `tree/`, each context slice, and what the packet cap dropped or cut short',
  [REPO_MAP_PATH]:
    "the repository's shape: every source file's top-level symbols and their signatures, ranked by how many other files reference them. Read it first",
}

/**
 * What a code packet's prompt adds after its context slices: the listed
 * files are not the whole tree, so the critic learns where the rest of it
 * is, how a file cut short keeps its line numbers, and how to cite a line,
 * which fan-out then checks.
 */
const codePacketNote = (read: string): string => [
  `\`tree/\` holds the files the change touched, the files that import them and as much of the rest of the repository at the commit under review as the packet cap let in, each at its repository path; read one with ${read} as \`tree/<repository path>\`. \`facts/INDEX.md\` records every file the cap left out or cut short: by name for the files the change touched, their importers, the files holding a slice and any file the diff, a ticket or the spec names, and counted per directory for the rest. A file cut short holds some of its lines at their own numbers; a run of lines it does not hold is blank but for a marker on its first line. You cannot read what the packet does not hold; say what you would need from it instead.`,
  '',
  'Cite code as `tree/<repository path>:<line>`, or `tree/<repository path>:<first line>-<last line>` for a range, in a finding\'s `location` and in each step of a `trace`. A line a hunk of `diff/change.diff` shows is cited the same way, at the number its `@@` line gives it. A finding that cites a file the packet does not hold, or a line outside the lines it holds, is rejected as fabricated evidence.',
  '',
  'When the diff did not fit the packet cap, its first line says so and `facts/INDEX.md` lists each hunk it left out by file and line.',
  '',
  'A quote from `tree/REPO-MAP.md` or `facts/` is a fact the runner computed, never a recorded decision.',
].join('\n')

/** A code packet's context slices, as its prompt lists them. */
function sliceList(slices: readonly CodeSlice[]): string {
  return [
    '## Context slices',
    '',
    'The code around the change, at its own line numbers: each changed function or block, the symbols it calls and the functions that call it. Read these lines of these files:',
    '',
    ...(slices.length === 0
      ? ['None: the change touches no text file the packet holds.']
      : slices.map((slice) => `- \`${slice.path}:${String(slice.first)}-${String(slice.last)}\`: ${slice.reason}`)),
  ].join('\n')
}

/**
 * Four blocks in order (preamble, role, lenses, output contract with the
 * finding schema), then the critic's tools, named exactly as it has them
 * (ADR-0006), and the list of packet files it is pointed at. A critic that
 * may only read learns the paths from this list alone: a document packet
 * lists every file, and a code packet its
 * repo map first, the diff, the tickets, the spec and the facts index, then
 * its context slices, with a note on reading the rest of its tree. Nothing
 * about other critics, models or the conversation goes in.
 */
export function assemblePrompt(
  sources: PromptSources,
  files: readonly PacketFile[],
  stage: CritiqueStage,
  slices: readonly CodeSlice[] = [],
  toolset: CriticToolset = READ_ONLY_TOOLSET,
): CriticPrompt {
  const read = readTool(toolset)
  const codePacket = isCodeStage(stage)
  const repositoryPacket = !codePacket && files.some((file) => file.kind === 'fact')
  const describe = ({ kind, path }: PacketFile): string =>
    (stage === 'tickets' ? TICKETS_STAGE_DESCRIPTIONS[kind] : undefined) ??
    (kind === 'check' ? CHECK_DESCRIPTIONS[path] : undefined) ??
    (repositoryPacket ? REPOSITORY_DESCRIPTIONS[kind] : undefined) ??
    (codePacket ? CODE_FACT_DESCRIPTIONS[path] : undefined) ??
    DESCRIPTIONS[kind]
  const lenses = sources.lenses.map((lens) => lens.text.trim()).join('\n\n')
  const contract = `${sources.contract.trim()}\n\n\`\`\`json\n${sources.schema.trim()}\n\`\`\``
  const fileList = [
    toolsSection(toolset),
    '',
    '# Files',
    '',
    codePacket
      ? `Your working directory holds the change under review. Read each of these files with ${read}, using the path exactly as written here:`
      : `Your working directory holds these files and no others. Read each one with ${read}, using the path exactly as written here:`,
    '',
    ...files.map((file) => `- \`${file.path}\`: ${describe(file)}`),
    ...(codePacket ? ['', sliceList(slices), '', codePacketNote(read)] : []),
    ...(repositoryPacket ? ['', REPOSITORY_PACKET_NOTE] : []),
  ].join('\n')
  return {
    lens_hashes: Object.fromEntries(sources.lenses.map((lens) => [lens.name, sha256(lens.text)])),
    prompt_hashes: {
      contract: sha256(sources.contract),
      preamble: sha256(sources.preamble),
      role: sha256(sources.role),
      schema: sha256(sources.schema),
    },
    // A role with no lenses, such as the refuter, has no lens block.
    text: `${[sources.preamble.trim(), sources.role.trim(), lenses, contract, fileList].filter((block) => block !== '').join(BLOCK_SEPARATOR)}\n`,
  }
}
