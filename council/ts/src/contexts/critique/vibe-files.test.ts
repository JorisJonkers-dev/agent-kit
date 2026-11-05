import { describe, expect, it } from 'vitest'

import { agentFileSearchProblem, homeConfigProblem, mcpServerNames } from './adapters/vibe/index.js'
import { readToml } from '../../shared-kernel/toml.js'

/**
 * What the runner reads from Vibe's files before it launches a critic that
 * searches (#255): the critic agent file and the home's `config.toml`, read
 * as TOML rather than line by line.
 */

const APPROVED = ['[tools.packet_grep]', 'permission = "always"', '[tools.packet_list]', 'permission = "always"', '[tools.packet_symbol]', 'permission = "always"', ''].join('\n')
const ENABLED = 'enabled_tools = ["read_file", "packet_grep", "packet_list", "packet_symbol"]\n'

describe('the TOML reader', () => {
  it('reads tables, arrays of tables, dotted and quoted keys, strings, numbers, booleans, arrays and inline tables', () => {
    const doc = readToml(
      [
        '# a comment',
        'title = "a \\"quoted\\" \\\\ word\\tand \\u00e9\\U0001F600" # trailing',
        "literal = 'C:\\path'",
        'multi = """',
        'two',
        'lines"""',
        "raw = '''",
        "kept \\n'''",
        'count = 1_000',
        'ratio = 0.5',
        'when = 2026-11-02T00:00:00Z',
        'on = true',
        'off = false',
        'list = [',
        '  "a", # first',
        "  'b',",
        '  [1, 2],',
        ']',
        'empty = {}',
        'inline = { name = "x", nested.deep = 1 }',
        'site."key with space".value = 3',
        '',
        '[server]',
        'port = 8080',
        '',
        '[[mcp_servers]]',
        'name = "one"',
        '[[mcp_servers]]',
        'name = "two"',
        '[mcp_servers.extra]',
        'flag = true',
        '',
      ].join('\r\n'),
    )

    expect(doc).toEqual({
      count: 1000,
      empty: {},
      inline: { name: 'x', nested: { deep: 1 } },
      list: ['a', 'b', [1, 2]],
      literal: 'C:\\path',
      mcp_servers: [{ name: 'one' }, { extra: { flag: true }, name: 'two' }],
      multi: 'two\r\nlines',
      off: false,
      on: true,
      ratio: 0.5,
      raw: 'kept \\n',
      server: { port: 8080 },
      site: { 'key with space': { value: 3 } },
      title: 'a "quoted" \\ word\tand é😀',
      when: '2026-11-02T00:00:00Z',
    })
  })

  it.each([
    ['a bare value', 'key = \n', 'line 1: expected a value'],
    ['a missing equals sign', 'key "x"\n', "line 1: expected '.'"],
    ['an unclosed string', 'key = "open\n', 'line 1: expected the closing "'],
    ['an unclosed multi-line string', 'key = """open', 'line 1: expected the closing """'],
    ['a bad escape', 'key = "\\q"\n', 'line 1: expected a valid escape'],
    ['a key defined twice', 'a = 1\na = 2\n', 'line 2: expected a key not already defined (a)'],
    ['two values on a line', 'a = 1 b = 2\n', "line 1: expected '\n'"],
    ['an unclosed array', 'a = [1 2]\n', "line 1: expected ','"],
    ['an unclosed inline table', 'a = { b = 1 c = 2 }\n', "line 1: expected ','"],
    ['a table over a value', 'a = 1\n[a.b]\n', 'line 2: expected a table at a'],
    ['an array of tables over a table', '[a]\n[[a]]\n', 'line 2: expected an array of tables at a'],
    ['a key that is not one', '= 1\n', 'line 1: expected a key'],
  ])('refuses %s', (_name, text, message) => {
    expect(() => readToml(text)).toThrow(message)
  })
})

describe('the pre-launch check of a critic agent file', () => {
  it('lets a critic search when the file enables and approves the search tools, in either quote style', () => {
    expect(agentFileSearchProblem(`${ENABLED}\n${APPROVED}`, 'critic-glm')).toBeUndefined()
    expect(agentFileSearchProblem(`${ENABLED.replace(/"/g, "'")}\n${APPROVED.replace(/"/g, "'")}`, 'critic-glm')).toBeUndefined()
  })

  it.each([
    ['a top-level mcp_servers array', `${ENABLED}mcp_servers = [{ name = "extra", url = "https://example.invalid/mcp" }]\n${APPROVED}`, 'extra'],
    ['an [[mcp_servers]] table', `${ENABLED}${APPROVED}[[mcp_servers]]\nname = "extra"\n`, 'extra'],
    ['a server without a name, inside another table', `${ENABLED}${APPROVED}[other]\nmcp_servers = { transport = "stdio" }\n`, '?'],
  ])('refuses %s', (_name, file, server) => {
    expect(agentFileSearchProblem(file, 'critic-glm')).toBe(
      `agent-file-declares-servers: ~/.vibe/agents/critic-glm.toml declares MCP servers (${server}) that would run beside the search server; re-run agent-kit setup to install the current critic agents`,
    )
  })

  it('refuses a file it cannot read', () => {
    expect(agentFileSearchProblem('enabled_tools = [\n', 'critic-glm')).toMatch(/^agent-file-unreadable: ~\/\.vibe\/agents\/critic-glm\.toml is not TOML the runner can read \(line 2: expected/)
  })
})

describe('the home’s config.toml', () => {
  it('names the servers it declares, in either form, and none for a file it cannot read', () => {
    expect(mcpServerNames('[[mcp_servers]]\nname = "a"\n[[mcp_servers]]\ntransport = "stdio"\n')).toEqual(['a'])
    expect(mcpServerNames("mcp_servers = [{ name = 'b' }]\n")).toEqual(['b'])
    expect(mcpServerNames('active_model = "x"\n')).toEqual([])
    expect(mcpServerNames('[[mcp_servers]\n')).toEqual([])
  })

  it('blocks search when it cannot be read, since its servers could not be switched off', () => {
    expect(homeConfigProblem('active_model = "x"\n')).toBeUndefined()
    expect(homeConfigProblem('[[mcp_servers]\n')).toMatch(/^home-config-unreadable: ~\/\.vibe\/config\.toml is not TOML the runner can read/)
  })
})
