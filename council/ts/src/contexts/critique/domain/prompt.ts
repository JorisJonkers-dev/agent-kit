import { sha256, type PacketFile, type PacketFileKind } from './manifest.js'
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
  feature: 'the feature the decisions are for',
  reference: 'reference material',
  spec: 'the spec the tickets come from',
  ticket: 'a ticket the change delivers',
  tree: 'a file the change touched, as it is at the commit under review',
}

/** At the tickets stage the tickets are under review; at a code stage they are what the change delivers. */
const TICKETS_STAGE_DESCRIPTIONS: Readonly<Partial<Record<PacketFileKind, string>>> = { ticket: 'a ticket under review' }

/**
 * What a code packet's prompt adds after its file list: the listed files
 * are only what the change touched, so the critic learns where the rest of
 * the tree is and how to cite a line, which fan-out then checks.
 */
const CODE_PACKET_NOTE = [
  'Every other file of the repository at the commit under review is under `tree/`, at its repository path; read one with read_file as `tree/<repository path>`.',
  '',
  'Cite code as `tree/<repository path>:<line>`, or `tree/<repository path>:<first line>-<last line>` for a range, in a finding\'s `location` and in each step of a `trace`. A finding that cites a file the packet does not hold, or a line past the end of its file, is rejected as fabricated evidence.',
].join('\n')

/**
 * Four blocks in order (preamble, role, lenses, output contract with the
 * finding schema), then the list of packet files the critic is pointed at.
 * Vibe critics cannot list directories, so the list is the only way they
 * learn the paths: a document packet lists every file, and a code packet the
 * diff, the changed files, the tickets and the spec, with a note on reading
 * the rest of its tree. Nothing about other critics, models or the
 * conversation goes in.
 */
export function assemblePrompt(sources: PromptSources, files: readonly PacketFile[], stage: CritiqueStage): CriticPrompt {
  const codePacket = isCodeStage(stage)
  const describe = (kind: PacketFileKind): string =>
    (stage === 'tickets' ? TICKETS_STAGE_DESCRIPTIONS[kind] : undefined) ?? DESCRIPTIONS[kind]
  const lenses = sources.lenses.map((lens) => lens.text.trim()).join('\n\n')
  const contract = `${sources.contract.trim()}\n\n\`\`\`json\n${sources.schema.trim()}\n\`\`\``
  const fileList = [
    '# Files',
    '',
    codePacket
      ? 'Your working directory holds the change under review. Read each of these files with read_file, using the path exactly as written here:'
      : 'Your working directory holds these files and no others. Read each one with read_file, using the path exactly as written here:',
    '',
    ...files.map((file) => `- \`${file.path}\`: ${describe(file.kind)}`),
    ...(codePacket ? ['', CODE_PACKET_NOTE] : []),
  ].join('\n')
  return {
    lens_hashes: Object.fromEntries(sources.lenses.map((lens) => [lens.name, sha256(lens.text)])),
    prompt_hashes: {
      contract: sha256(sources.contract),
      preamble: sha256(sources.preamble),
      role: sha256(sources.role),
      schema: sha256(sources.schema),
    },
    text: `${[sources.preamble.trim(), sources.role.trim(), lenses, contract, fileList].join(BLOCK_SEPARATOR)}\n`,
  }
}
