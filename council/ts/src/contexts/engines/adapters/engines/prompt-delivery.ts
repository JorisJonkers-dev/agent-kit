import type { JsonRecord } from '../../../../shared-kernel/index.js'
import type { EnginePromptDelivery } from '../../../engines/index.js'
import type { EngineAdapterPorts, EngineChild, EngineRunRequest } from './types.js'

interface PromptDeliveryPolicy {
  deliver(request: EngineRunRequest, ports: EngineAdapterPorts, child: EngineChild): Promise<void>
}

const PROMPT_DELIVERY_POLICIES: Readonly<Record<EnginePromptDelivery, PromptDeliveryPolicy>> =
  Object.freeze({
    prompt_file: {
      async deliver(request, ports): Promise<void> {
        await ports.files.writeText(request.promptFile, request.prompt)
      },
    },
    stdin: {
      async deliver(request, _ports, child): Promise<void> {
        await writeStdinPromptAndClose(child, request.prompt)
      },
    },
  })

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
  await PROMPT_DELIVERY_POLICIES[request.engine.promptDelivery].deliver(request, ports, child)
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
