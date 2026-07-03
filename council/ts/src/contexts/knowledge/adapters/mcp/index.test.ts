import { describe, expect, it } from 'vitest'

import {
  KnowledgeMcpAuthError,
  KnowledgeMcpJsonRpcAdapter,
  type KnowledgeMcpFetch,
  type KnowledgeMcpFetchInit,
  type KnowledgeMcpFetchResponse,
  type KnowledgeMcpRpcError,
} from './index.js'

interface RecordedRequest {
  readonly init: KnowledgeMcpFetchInit
  readonly url: string
}

function env(values: Readonly<Record<string, string>>): { require: (name: string) => string } {
  return {
    require(name) {
      return values[name] ?? ''
    },
  }
}

function jsonResponse(body: unknown, status = 200): KnowledgeMcpFetchResponse {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  }
}

function mcpText(value: unknown): unknown {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
  }
}

function recordingFetch(responses: readonly KnowledgeMcpFetchResponse[]): {
  readonly fetch: KnowledgeMcpFetch
  readonly requests: readonly RecordedRequest[]
} {
  const requests: RecordedRequest[] = []
  let nextResponse = 0

  return {
    fetch: async (url, init) => {
      const response = responses[nextResponse] as KnowledgeMcpFetchResponse
      nextResponse += 1
      requests.push({ init, url })
      return response
    },
    requests,
  }
}

describe('KnowledgeMcpJsonRpcAdapter', () => {
  it('reads environment, normalizes the MCP endpoint, and serializes all typed calls as tools/call', async () => {
    const transport = recordingFetch([
      jsonResponse({
        jsonrpc: '2.0',
        id: 1,
        result: {
          structuredContent: {
            hits: [
              {
                id: 'note-1',
                title: 'Relevant note',
                snippet: 'A useful excerpt.',
                score: 0.42,
                scope: 'project:personal-stack',
                topic: 'topic:agents',
                tags: ['codex'],
              },
            ],
          },
        },
      }),
      jsonResponse({
        jsonrpc: '2.0',
        id: 2,
        result: {
          structuredContent: {
            id: 'note-1',
            title: 'Relevant note',
            body: 'Full note body.',
            scope: 'project:personal-stack',
          },
        },
      }),
      jsonResponse({
        jsonrpc: '2.0',
        id: 3,
        result: {
          structuredContent: {
            note: { id: 'note-1', title: 'Relevant note' },
            relations: [{ from_id: 'note-1', to_id: 'note-2', kind: 'supports' }],
          },
        },
      }),
      jsonResponse({
        jsonrpc: '2.0',
        id: 4,
        result: { structuredContent: { id: 'lesson-1', created: true, path: 'lesson.md' } },
      }),
      jsonResponse({
        jsonrpc: '2.0',
        id: 5,
        result: { structuredContent: { id: 'decision-1', created: true } },
      }),
      jsonResponse({
        jsonrpc: '2.0',
        id: 6,
        result: { structuredContent: { id: 'note-imported', created: false } },
      }),
    ])
    const adapter = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'secret-token', KB_URL: 'https://kb.example.test/root/' }),
      fetch: transport.fetch,
    })

    await expect(adapter.recall({ query: 'hexagonal knowledge', limit: 5 })).resolves.toEqual({
      matches: [
        {
          id: 'note-1',
          title: 'Relevant note',
          snippet: 'A useful excerpt.',
          score: 0.42,
          scope: 'project:personal-stack',
          topic: 'topic:agents',
          tags: ['codex'],
        },
      ],
    })
    await expect(adapter.get_note({ id: 'note-1' })).resolves.toMatchObject({
      body: 'Full note body.',
      id: 'note-1',
    })
    await expect(adapter.relations({ id: 'note-1', depth: 1 })).resolves.toEqual({
      note: { id: 'note-1', title: 'Relevant note' },
      relations: [{ from_id: 'note-1', to_id: 'note-2', kind: 'supports' }],
    })
    await expect(
      adapter.capture_lesson({ title: 'Lesson', body: 'Body', tags: ['mcp'] }),
    ).resolves.toEqual({ id: 'lesson-1', created: true, path: 'lesson.md' })
    await expect(
      adapter.capture_decision({
        title: 'Decision',
        decision: 'Use JSON-RPC.',
        rationale: 'MCP requires it.',
      }),
    ).resolves.toEqual({ id: 'decision-1', created: true })
    await expect(
      adapter.ingest_note({
        title: 'Imported',
        body: 'Imported body.',
        scope: 'project:personal-stack',
      }),
    ).resolves.toEqual({ id: 'note-imported', created: false })

    expect(transport.requests.map((request) => request.url)).toEqual([
      'https://kb.example.test/root/mcp',
      'https://kb.example.test/root/mcp',
      'https://kb.example.test/root/mcp',
      'https://kb.example.test/root/mcp',
      'https://kb.example.test/root/mcp',
      'https://kb.example.test/root/mcp',
    ])
    expect(transport.requests.map((request) => request.init)).toEqual([
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/call',
          params: {
            name: 'knowledge.recall',
            arguments: { query: 'hexagonal knowledge', limit: 5 },
          },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 2,
          method: 'tools/call',
          params: { name: 'knowledge.get_note', arguments: { id: 'note-1' } },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 3,
          method: 'tools/call',
          params: { name: 'knowledge.relations', arguments: { id: 'note-1', depth: 1 } },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 4,
          method: 'tools/call',
          params: {
            name: 'knowledge.capture_lesson',
            arguments: { title: 'Lesson', body: 'Body', tags: ['mcp'] },
          },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 5,
          method: 'tools/call',
          params: {
            name: 'knowledge.capture_decision',
            arguments: {
              title: 'Decision',
              decision: 'Use JSON-RPC.',
              rationale: 'MCP requires it.',
            },
          },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
      {
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 6,
          method: 'tools/call',
          params: {
            name: 'knowledge.ingest_note',
            arguments: {
              title: 'Imported',
              body: 'Imported body.',
              scope: 'project:personal-stack',
            },
          },
        }),
        headers: {
          Authorization: 'Bearer secret-token',
          'Content-Type': 'application/json',
        },
        method: 'POST',
      },
    ])
  })

  it('preserves an existing /mcp endpoint and decodes MCP text fallback results', async () => {
    const transport = recordingFetch([
      jsonResponse({
        jsonrpc: '2.0',
        id: 1,
        result: mcpText({ id: 'note-1', title: 'Text note', body: 'Fallback JSON.' }),
      }),
    ])
    const adapter = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test/mcp/' }),
      fetch: transport.fetch,
    })

    await expect(adapter.get_note({ id: 'note-1' })).resolves.toEqual({
      id: 'note-1',
      title: 'Text note',
      body: 'Fallback JSON.',
    })
    expect(transport.requests[0]?.url).toBe('https://kb.example.test/mcp')
  })

  it('reports HTTP auth failures predictably', async () => {
    const transport = recordingFetch([jsonResponse({ error: 'denied' }, 401)])
    const adapter = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'bad-token', KB_URL: 'https://kb.example.test' }),
      fetch: transport.fetch,
    })

    const failure = adapter.recall({ query: 'auth' })

    await expect(failure).rejects.toThrow(KnowledgeMcpAuthError)
    await expect(failure).rejects.toThrow('Knowledge MCP authentication failed with HTTP 401.')
  })

  it('reports JSON-RPC errors predictably', async () => {
    const transport = recordingFetch([
      jsonResponse({
        jsonrpc: '2.0',
        id: 1,
        error: { code: -32602, message: 'Invalid params', data: { field: 'query' } },
      }),
    ])
    const adapter = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: transport.fetch,
    })

    await expect(adapter.recall({ query: 'bad' })).rejects.toMatchObject({
      code: -32602,
      data: { field: 'query' },
      message: 'Knowledge MCP JSON-RPC error -32602: Invalid params',
      name: 'KnowledgeMcpRpcError',
    } satisfies Partial<KnowledgeMcpRpcError>)
  })

  it('rejects malformed RPC envelopes and malformed knowledge payloads', async () => {
    const malformedRpc = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([jsonResponse({ jsonrpc: '2.0', id: 1 })]).fetch,
    })
    const malformedRecall = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { hits: [{ id: 'note-1', score: 'bad' }] } },
        }),
      ]).fetch,
    })
    const malformedNote = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { id: 'note-1', title: 'Missing body' } },
        }),
      ]).fetch,
    })
    const malformedRelations = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { note: { id: 'note-1' }, relations: [] } },
        }),
      ]).fetch,
    })
    const malformedLesson = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { id: 'lesson-1', created: 'yes' } },
        }),
      ]).fetch,
    })
    const malformedDecision = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { id: 'decision-1', created: 'yes' } },
        }),
      ]).fetch,
    })
    const malformedIngest = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { structuredContent: { id: 'note-imported', created: 'yes' } },
        }),
      ]).fetch,
    })
    const malformedVersion = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([jsonResponse({ jsonrpc: '1.0', id: 1, result: mcpText({}) })]).fetch,
    })
    const mismatchedId = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([jsonResponse({ jsonrpc: '2.0', id: 2, result: mcpText({}) })]).fetch,
    })
    const toolError = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({
          jsonrpc: '2.0',
          id: 1,
          result: { isError: true, structuredContent: { id: 'note-1' } },
        }),
      ]).fetch,
    })

    await expect(malformedRpc.recall({ query: 'missing result' })).rejects.toThrow(
      'Knowledge MCP JSON-RPC response must include a result.',
    )
    await expect(malformedRecall.recall({ query: 'bad payload' })).rejects.toThrow(
      'Knowledge recall match title must be a string.',
    )
    await expect(malformedNote.get_note({ id: 'note-1' })).rejects.toThrow(
      'Knowledge note body must be a string.',
    )
    await expect(malformedRelations.relations({ id: 'note-1' })).rejects.toThrow(
      'Knowledge note reference title must be a string.',
    )
    await expect(malformedLesson.capture_lesson({ title: 'Lesson', body: 'Body' })).rejects.toThrow(
      'Knowledge capture result created flag must be a boolean.',
    )
    await expect(
      malformedDecision.capture_decision({
        title: 'Decision',
        decision: 'Use JSON-RPC.',
        rationale: 'MCP requires it.',
      }),
    ).rejects.toThrow('Knowledge capture result created flag must be a boolean.')
    await expect(
      malformedIngest.ingest_note({
        title: 'Imported',
        body: 'Body',
        scope: 'project:personal-stack',
      }),
    ).rejects.toThrow('Knowledge capture result created flag must be a boolean.')
    await expect(malformedVersion.recall({ query: 'bad version' })).rejects.toThrow(
      'Knowledge MCP JSON-RPC response jsonrpc must be "2.0".',
    )
    await expect(mismatchedId.recall({ query: 'bad id' })).rejects.toThrow(
      'Knowledge MCP JSON-RPC response id must match request id 1.',
    )
    await expect(toolError.recall({ query: 'tool error' })).rejects.toThrow(
      'Knowledge MCP result is an error result.',
    )
  })

  it('rejects invalid environment and HTTP response payloads before decoding tools', async () => {
    expect(
      () =>
        new KnowledgeMcpJsonRpcAdapter({
          env: env({ KB_BEARER_TOKEN: 'token', KB_URL: '' }),
          fetch: recordingFetch([]).fetch,
        }),
    ).toThrow('KB_URL must not be empty.')

    const invalidJson = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([jsonResponse('not an object')]).fetch,
    })
    const invalidError = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([
        jsonResponse({ jsonrpc: '2.0', id: 1, error: { code: 'bad', message: 42 } }),
      ]).fetch,
    })
    const unavailable = new KnowledgeMcpJsonRpcAdapter({
      env: env({ KB_BEARER_TOKEN: 'token', KB_URL: 'https://kb.example.test' }),
      fetch: recordingFetch([jsonResponse({ error: 'down' }, 503)]).fetch,
    })

    await expect(invalidJson.get_note({ id: 'note-1' })).rejects.toThrow(
      'Knowledge MCP JSON-RPC response must be an object.',
    )
    await expect(invalidError.get_note({ id: 'note-1' })).rejects.toThrow(
      'Knowledge MCP JSON-RPC error must include numeric code and string message.',
    )
    await expect(unavailable.get_note({ id: 'note-1' })).rejects.toThrow(
      'Knowledge MCP request failed with HTTP 503.',
    )
  })
})
