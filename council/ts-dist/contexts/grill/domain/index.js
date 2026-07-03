export function startGrillSession(input) {
    const ledger = input.assumptions ?? [];
    if (input.mode === 'non-interactive') {
        return {
            mode: input.mode,
            phase: 'published-question-list',
            questions: input.questions,
            ledger,
            publishedQuestions: input.questions,
        };
    }
    return withNextQuestion({
        mode: input.mode,
        phase: 'exploring',
        questions: input.questions,
        ledger,
    });
}
export function currentQuestion(session) {
    return session.questions.find((question) => question.id === session.currentQuestionId);
}
export function answerCurrentQuestion(session, answer) {
    const question = currentQuestion(session);
    if (question === undefined) {
        throw new Error('No active grill question is available');
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
    });
}
export function deriveAssumption(session, assumption) {
    return appendAssumption(session, { ...assumption, status: 'derived' });
}
export function defaultAssumption(session, assumption) {
    return appendAssumption(session, { ...assumption, status: 'defaulted' });
}
export function publishedQuestionList(session) {
    return session.phase === 'published-question-list'
        ? (session.publishedQuestions ?? [])
        : [currentQuestion(session)].filter(isQuestion);
}
function withNextQuestion(session) {
    const answered = new Set(session.ledger.filter((assumption) => assumption.status === 'asked').map((assumption) => assumption.id));
    const nextQuestion = session.questions.find((question) => !answered.has(question.id));
    if (nextQuestion === undefined) {
        return {
            mode: session.mode,
            phase: 'complete',
            questions: session.questions,
            ledger: session.ledger,
        };
    }
    return {
        mode: session.mode,
        phase: 'exploring',
        questions: session.questions,
        ledger: session.ledger,
        currentQuestionId: nextQuestion.id,
    };
}
function appendAssumption(session, assumption) {
    return {
        ...session,
        ledger: [...session.ledger, assumption],
    };
}
function isQuestion(question) {
    return question !== undefined;
}
