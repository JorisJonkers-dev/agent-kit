import { describe, expect, it } from 'vitest'

import {
  CURATED_DEFAULT_KNOWLEDGE_SCOPE,
  canonicalProjectScopeFromGitRemote,
  resolveKnowledgeScope,
} from './scope-policy.js'

describe('knowledge scope policy', () => {
  it('parses canonical project scopes from GitHub SSH and HTTPS remotes', () => {
    expect(canonicalProjectScopeFromGitRemote('git@github.com:OpenAI/Codex.git')).toBe(
      'project:openai/codex',
    )
    expect(canonicalProjectScopeFromGitRemote('https://github.com/Acme-Corp/repo-name.git')).toBe(
      'project:acme-corp/repo-name',
    )
    expect(canonicalProjectScopeFromGitRemote('ssh://git@github.com/Tools/Runner')).toBe(
      'project:tools/runner',
    )
  })

  it('ignores non-GitHub or malformed remotes instead of inventing a scope', () => {
    expect(canonicalProjectScopeFromGitRemote('git@gitlab.com:OpenAI/Codex.git')).toBeUndefined()
    expect(canonicalProjectScopeFromGitRemote('https://github.com/OpenAI')).toBeUndefined()
    expect(canonicalProjectScopeFromGitRemote('git@github.com:/Codex.git')).toBeUndefined()
    expect(canonicalProjectScopeFromGitRemote('')).toBeUndefined()
  })

  it('prefers explicit scopes, otherwise infers from a Git remote', () => {
    expect(
      resolveKnowledgeScope({
        requestedScope: ' topic:agents ',
        gitRemote: 'git@github.com:OpenAI/Codex.git',
      }),
    ).toBe('topic:agents')
    expect(resolveKnowledgeScope({ gitRemote: 'git@github.com:OpenAI/Codex.git' })).toBe(
      'project:openai/codex',
    )
    expect(resolveKnowledgeScope({ gitRemote: 'https://example.test/repo.git' })).toBeUndefined()
  })

  it('omits the curated default scope for capture requests', () => {
    expect(resolveKnowledgeScope({ requestedScope: CURATED_DEFAULT_KNOWLEDGE_SCOPE })).toBeUndefined()
    expect(
      resolveKnowledgeScope({
        gitRemote: 'git@github.com:Custom/Default.git',
        curatedDefaultScope: 'project:custom/default',
      }),
    ).toBeUndefined()
  })

  it('forbids broad all-scope access unless the caller explicitly allows it', () => {
    expect(() => resolveKnowledgeScope({ requestedScope: 'all' })).toThrow(
      'knowledge scope "all" requires explicit allowAll.',
    )
    expect(resolveKnowledgeScope({ requestedScope: ' all ', allowAll: true })).toBe('all')
  })
})
