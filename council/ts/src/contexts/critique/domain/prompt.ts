import { sha256, type PacketFile } from './manifest.js'

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

/**
 * Four blocks in order (preamble, role, lenses, output contract with the
 * finding schema), then the list of packet files the critic can read. Vibe
 * critics cannot list directories, so the list is the only way they learn the
 * paths. Nothing about other critics, models or the conversation goes in.
 */
export function assemblePrompt(sources: PromptSources, files: readonly PacketFile[]): CriticPrompt {
  const lenses = sources.lenses.map((lens) => lens.text.trim()).join('\n\n')
  const contract = `${sources.contract.trim()}\n\n\`\`\`json\n${sources.schema.trim()}\n\`\`\``
  const fileList = [
    '# Files',
    '',
    'Your working directory holds these files and no others. Read each one with read_file, using the path exactly as written here:',
    '',
    ...files.map((file) => `- \`${file.path}\`: ${file.kind === 'artifact' ? 'the artifact under review' : 'reference material'}`),
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
