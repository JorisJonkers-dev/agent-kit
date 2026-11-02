import { basename, dirname, join } from 'node:path'

import { ADR_DIRECTORY, CONTEXT_MAP, DOMAIN_DOC, GLOSSARIES, inForce, namedPaths, NOT_RECORDS } from '../domain/decision-docs.js'
import type { CritiqueFilesPort } from './ports.js'

/**
 * Finds the decision docs a spec is checked against. With a domain doc, they
 * are the files and directories it names, and for a `CONTEXT-MAP.md` each
 * context's `CONTEXT.md` and `docs/adr/` as well. Without one, they are the
 * first of `GLOSSARY.md` and `CONTEXT.md` that exists, and the ADRs in
 * `docs/adr/`. A record whose status says it is proposed, rejected,
 * superseded or deprecated is left out; one with no status counts as
 * accepted. Every path returned is repository-relative.
 */
export class DecisionDocs {
  constructor(private readonly files: CritiqueFilesPort) {}

  async resolve(root: string): Promise<readonly string[]> {
    const domainDoc = join(root, DOMAIN_DOC)
    const found = (await this.files.exists(domainDoc))
      ? await this.fromDomainDoc(root, await this.files.readText(domainDoc))
      : await this.fallback(root)
    const inForceDocs: string[] = []
    for (const path of new Set(found)) {
      if (NOT_RECORDS.includes(basename(path)) || inForce(await this.files.readText(join(root, path)))) inForceDocs.push(path)
    }
    return inForceDocs
  }

  private async fromDomainDoc(root: string, text: string): Promise<readonly string[]> {
    const found: string[] = []
    for (const path of namedPaths(text)) {
      found.push(...(await this.expand(root, path)))
      if (basename(path) === CONTEXT_MAP && (await this.isFile(root, path))) {
        found.push(...(await this.contexts(root, await this.files.readText(join(root, path)))))
      }
    }
    return found
  }

  /** Each `CONTEXT.md` a context map names, with the ADRs beside it. */
  private async contexts(root: string, map: string): Promise<readonly string[]> {
    const found: string[] = []
    for (const path of namedPaths(map).filter((named) => basename(named) === 'CONTEXT.md')) {
      found.push(...(await this.expand(root, path)), ...(await this.markdownIn(root, join(dirname(path), ADR_DIRECTORY))))
    }
    return found
  }

  private async fallback(root: string): Promise<readonly string[]> {
    const glossaries: string[] = []
    for (const candidate of GLOSSARIES) {
      if (await this.isFile(root, candidate)) {
        glossaries.push(candidate)
        break
      }
    }
    return [...glossaries, ...(await this.markdownIn(root, ADR_DIRECTORY))]
  }

  /** A file as itself, a directory as the markdown files directly inside it, anything missing as nothing. */
  private async expand(root: string, path: string): Promise<readonly string[]> {
    const inside = await this.markdownIn(root, path)
    if (inside.length > 0) return inside
    return (await this.isFile(root, path)) ? [path] : []
  }

  private async markdownIn(root: string, directory: string): Promise<readonly string[]> {
    const names = await this.files.listFiles(join(root, directory))
    return names.filter((name) => name.endsWith('.md')).map((name) => join(directory, name))
  }

  /** True only for a plain file, so an empty directory is never read as a doc. */
  private async isFile(root: string, path: string): Promise<boolean> {
    return (await this.files.listFiles(join(root, dirname(path)))).includes(basename(path))
  }
}
