import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import { CodeNavRepoMap } from './client.js'

class FakeNav implements ProcessPort {
  readonly commands: ProcessCommand[] = []
  constructor(private readonly reply: (command: ProcessCommand) => ProcessResult | Error) {}
  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    const reply = this.reply(command)
    return reply instanceof Error ? Promise.reject(reply) : Promise.resolve(reply)
  }
}

const dirs: string[] = []
afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { force: true, recursive: true })))
})

async function root(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'code-nav-map-'))
  dirs.push(dir)
  return dir
}

const reply = (text: unknown, id = 2): string =>
  `${JSON.stringify({ id: 1, jsonrpc: '2.0', result: {} })}\n${JSON.stringify({ id, jsonrpc: '2.0', result: text })}\n`
const content = (text: string, isError = false) => ({ content: [{ text, type: 'text' }], isError })
const ok = (stdout: string): ProcessResult => ({ exitCode: 0, stderr: '', stdout })

describe('the code-nav repository map client', () => {
  it('ranks the files the map lists, as paths inside the root, and asks only for the map', async () => {
    const dir = await root()
    const text = `files:\n${join(dir, 'src/a b.ts')} (9 refs, 2 symbols)\n${join(dir, 'z.ts')} (1 refs, 1 symbols)\n/elsewhere/x.ts (7 refs, 1 symbols)\nnot a file line\nsymbols:\n${join(dir, 'src/a b.ts')}:12 fn do it (9 refs)\n${join(dir, 'z.ts')}:3 struct Z (1 refs)\n/elsewhere/x.ts:1 fn x (7 refs)\nnot a symbol line\nshowing 2 of 2 files`
    const process = new FakeNav(() => ok(reply(content(text))))

    const ranking = await new CodeNavRepoMap(process).rank(dir)

    expect(ranking).toEqual({
      files: [
        { path: 'src/a b.ts', refs: 9 },
        { path: 'z.ts', refs: 1 },
      ],
      status: 'ranked',
      symbols: [
        { kind: 'fn', line: 12, name: 'do it', path: 'src/a b.ts', refs: 9 },
        { kind: 'struct', line: 3, name: 'Z', path: 'z.ts', refs: 1 },
      ],
    })
    const [command] = process.commands
    expect(command?.command).toBe('code-nav')
    expect(command?.args.slice(0, 1)).toEqual(['--root'])
    expect(command?.stdin).toContain('"name":"map"')
  })

  it('ranks nothing when the tree has no symbols', async () => {
    const ranking = await new CodeNavRepoMap(new FakeNav(() => ok(reply(content('no symbols'))))).rank(await root())

    expect(ranking).toEqual({ files: [], status: 'ranked', symbols: [] })
  })

  it('works on a root that does not exist, keeping the name it was given', async () => {
    const ranking = await new CodeNavRepoMap(new FakeNav(() => ok(reply(content('files:\n/gone/a.ts (1 refs, 1 symbols)')))), 'nav').rank('/gone')

    expect(ranking).toEqual({ files: [{ path: 'a.ts', refs: 1 }], status: 'ranked', symbols: [] })
  })

  it.each([
    ['is not installed', () => new Error('spawn code-nav ENOENT'), 'code-nav did not start: spawn code-nav ENOENT'],
    ['fails with a reason', () => ({ exitCode: 2, stderr: 'code-nav: none of the roots exists\nusage', stdout: '' }), 'code-nav exited 2: code-nav: none of the roots exists'],
    ['fails without a reason', () => ({ exitCode: 1, stderr: '', stdout: '' }), 'code-nav exited 1'],
    ['times out', () => ({ exitCode: 0, stderr: '', stdout: '', timedOut: true }), 'code-nav did not answer in time'],
    ['answers an error', () => ok(reply(content('boom', true))), 'code-nav answered no map'],
    ['answers no content', () => ok(reply({})), 'code-nav answered no map'],
    ['answers a non-object result', () => ok(reply('x')), 'code-nav answered no map'],
    ['answers a null result', () => ok(reply(null)), 'code-nav answered no map'],
    ['answers only noise', () => ok('garbage\n[1]\n"x"\n'), 'code-nav answered no map'],
    ['answers another id', () => ok(reply(content('files:'), 9)), 'code-nav answered no map'],
  ] as const)('is unavailable when code-nav %s', async (_name, run, detail) => {
    const process = new FakeNav(run as () => ProcessResult | Error)

    expect(await new CodeNavRepoMap(process).rank(await root())).toEqual({ detail, status: 'unavailable' })
  })

  it('reads text parts only', async () => {
    const parts = { content: [{ type: 'image' }, null, { text: 'files:\n' }, { text: `${'/r/a.ts'} (1 refs, 1 symbols)` }] }

    const ranking = await new CodeNavRepoMap(new FakeNav(() => ok(reply(parts)))).rank('/r')

    expect(ranking).toEqual({ files: [{ path: 'a.ts', refs: 1 }], status: 'ranked', symbols: [] })
  })
})
