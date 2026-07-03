import type { JsonRecord } from '../../../../shared-kernel/index.js'
import type { EngineAdapterPorts, EngineChild, EngineRunRequest } from './types.js'

export async function writeClaudePrompt(child: EngineChild, prompt: string): Promise<void> {
  await child.writeStdin(`${JSON.stringify(toClaudeInputMessage(prompt))}\n`)
}

export async function writeStdinPromptAndClose(
  child: EngineChild,
  prompt: string,
): Promise<void> {
  await child.writeStdin(prompt)
  await child.closeStdin()
}

export async function deliverGenericPrompt(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
  child: EngineChild,
): Promise<void> {
  if (request.engine.promptDelivery === 'prompt_file') {
    await ports.files.writeText(request.promptFile, request.prompt)
  } else {
    await writeStdinPromptAndClose(child, request.prompt)
  }
}

function toClaudeInputMessage(prompt: string): JsonRecord {
  return {
    type: 'user',
    message: {
      role: 'user',
      content: [{ type: 'text', text: prompt }],
    },
  }
}
