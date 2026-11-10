import { describe, expect, it } from 'vitest'

import { identifyRepository, parseRemote } from './git-remote.js'

describe('the nedap remote detector', () => {
  it.each([
    'git@github.com:nedap/x.git',
    'github.com:nedap/x.git',
    'github.com:nedap/x',
    'ssh://git@github.com/nedap/x',
    'ssh://github.com/nedap/x',
    'https://github.com/nedap/x',
    'https://github.com/nedap/x.git',
    'https://github.com/nedap/x/',
    'https://github.com/nedap/x.git/',
    'git@github.com:NeDaP/x.git',
    'https://github.com/NeDaP/X.git',
    'ssh://git@github.com:22/Nedap/x/',
    'git@github-work:nedap/x.git',
  ])('counts %s as nedap', (url) => {
    expect(identifyRepository([{ name: 'origin', url }])).toMatchObject({ owner: 'nedap', status: 'denied-owner' })
  })

  it('still reads other owners and keeps scp and scheme forms apart', () => {
    expect(parseRemote('github.com:me/x.git')).toEqual({ host: 'github.com', owner: 'me', repo: 'x' })
    expect(parseRemote('https://github.com/me/x')).toEqual({ host: 'github.com', owner: 'me', repo: 'x' })
  })
})
