export type GrillMode = 'interactive' | 'non-interactive'
export type GrillPhase = 'exploring' | 'complete' | 'published-question-list'
export type AssumptionStatus = 'asked' | 'derived' | 'defaulted'

export interface GrillQuestion {
  readonly id: string
  readonly prompt: string
  readonly why: string
}

export interface GrillAssumption {
  readonly id: string
  readonly statement: string
  readonly status: AssumptionStatus
  readonly evidence: string
}

export interface GrillSession {
  readonly mode: GrillMode
  readonly phase: GrillPhase
  readonly questions: readonly GrillQuestion[]
  readonly ledger: readonly GrillAssumption[]
  readonly currentQuestionId?: string
  readonly publishedQuestions?: readonly GrillQuestion[]
}

export interface StartGrillSessionInput {
  readonly mode: GrillMode
  readonly questions: readonly GrillQuestion[]
  readonly assumptions?: readonly GrillAssumption[]
}

export function startGrillSession(input: StartGrillSessionInput): GrillSession {
  const ledger = input.assumptions ?? []
  if (input.mode === 'non-interactive') {
    return {
      mode: input.mode,
      phase: 'published-question-list',
      questions: input.questions,
      ledger,
      publishedQuestions: input.questions,
    }
  }
  return withNextQuestion({
    mode: input.mode,
    phase: 'exploring',
    questions: input.questions,
    ledger,
  })
}

export function currentQuestion(session: GrillSession): GrillQuestion | undefined {
  return session.questions.find((question) => question.id === session.currentQuestionId)
}

export function answerCurrentQuestion(session: GrillSession, answer: string): GrillSession {
  const question = currentQuestion(session)
  if (question === undefined) {
    throw new Error('No active grill question is available')
  }
  return withNextQuestion({
    mode: session.mode,
    phase: 'exploring',
    questions: session.questions,
    ledger: [
      ...session.ledger,
      {
        id: question.id,
        statement: answer,
        status: 'asked',
        evidence: question.prompt,
      },
    ],
  })
}

export function deriveAssumption(session: GrillSession, assumption: Omit<GrillAssumption, 'status'>): GrillSession {
  return appendAssumption(session, { ...assumption, status: 'derived' })
}

export function defaultAssumption(session: GrillSession, assumption: Omit<GrillAssumption, 'status'>): GrillSession {
  return appendAssumption(session, { ...assumption, status: 'defaulted' })
}

export function publishedQuestionList(session: GrillSession): readonly GrillQuestion[] {
  return session.phase === 'published-question-list'
    ? (session.publishedQuestions ?? [])
    : [currentQuestion(session)].filter(isQuestion)
}

function withNextQuestion(session: Omit<GrillSession, 'currentQuestionId' | 'publishedQuestions'>): GrillSession {
  const answered = new Set(
    session.ledger.filter((assumption) => assumption.status === 'asked').map((assumption) => assumption.id),
  )
  const nextQuestion = session.questions.find((question) => !answered.has(question.id))
  if (nextQuestion === undefined) {
    return {
      mode: session.mode,
      phase: 'complete',
      questions: session.questions,
      ledger: session.ledger,
    }
  }
  return {
    mode: session.mode,
    phase: 'exploring',
    questions: session.questions,
    ledger: session.ledger,
    currentQuestionId: nextQuestion.id,
  }
}

function appendAssumption(session: GrillSession, assumption: GrillAssumption): GrillSession {
  return {
    ...session,
    ledger: [...session.ledger, assumption],
  }
}

function isQuestion(question: GrillQuestion | undefined): question is GrillQuestion {
  return question !== undefined
}
