import { describe, expect, it } from 'vitest'

import {
  answerCurrentQuestion,
  currentQuestion,
  defaultAssumption,
  deriveAssumption,
  publishedQuestionList,
  startGrillSession,
  type GrillQuestion,
} from './index.js'

const questions: readonly GrillQuestion[] = [
  {
    id: 'scope',
    prompt: 'What files are in scope?',
    why: 'This prevents collision with parallel work.',
  },
  {
    id: 'proof',
    prompt: 'What proof will show the work is complete?',
    why: 'This turns ambiguity into testable acceptance.',
  },
]

describe('grill session', () => {
  it('starts interactive sessions by exposing exactly one question', () => {
    const session = startGrillSession({ mode: 'interactive', questions })

    expect(session.phase).toBe('exploring')
    expect(currentQuestion(session)).toEqual(questions[0])
    expect(publishedQuestionList(session)).toEqual([questions[0]])
  })

  it('records asked assumptions and advances one question at a time', () => {
    const started = startGrillSession({ mode: 'interactive', questions })
    const answeredScope = answerCurrentQuestion(started, 'Only the allowed domain folders.')

    expect(answeredScope.ledger).toEqual([
      {
        id: 'scope',
        statement: 'Only the allowed domain folders.',
        status: 'asked',
        evidence: 'What files are in scope?',
      },
    ])
    expect(currentQuestion(answeredScope)).toEqual(questions[1])

    const complete = answerCurrentQuestion(answeredScope, 'Unit tests with coverage.')

    expect(complete.phase).toBe('complete')
    expect(currentQuestion(complete)).toBeUndefined()
    expect(publishedQuestionList(complete)).toEqual([])
  })

  it('fails closed when there is no active interactive question', () => {
    const session = startGrillSession({ mode: 'interactive', questions: [] })

    expect(session.phase).toBe('complete')
    expect(() => answerCurrentQuestion(session, 'nothing')).toThrow('No active grill question')
  })

  it('tracks derived and defaulted assumptions without changing the active question', () => {
    const started = startGrillSession({ mode: 'interactive', questions })
    const derived = deriveAssumption(started, {
      id: 'repo-evidence',
      statement: 'The repository has no existing grill domain.',
      evidence: 'source inspection',
    })
    const defaulted = defaultAssumption(derived, {
      id: 'fallback',
      statement: 'Ask all unresolved questions if non-interactive.',
      evidence: 'session mode',
    })

    expect(currentQuestion(defaulted)).toEqual(questions[0])
    expect(defaulted.ledger.map((entry) => entry.status)).toEqual(['derived', 'defaulted'])
  })

  it('degrades non-interactive sessions to a published question list', () => {
    const session = startGrillSession({
      mode: 'non-interactive',
      questions,
      assumptions: [
        {
          id: 'known',
          statement: 'The brief exists.',
          status: 'derived',
          evidence: 'input',
        },
      ],
    })

    expect(session.phase).toBe('published-question-list')
    expect(currentQuestion(session)).toBeUndefined()
    expect(publishedQuestionList(session)).toEqual(questions)
    expect(() => answerCurrentQuestion(session, 'not interactive')).toThrow('No active grill question')
  })
})
