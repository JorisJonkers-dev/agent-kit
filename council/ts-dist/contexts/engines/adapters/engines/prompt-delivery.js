const PROMPT_DELIVERY_POLICIES = Object.freeze({
    prompt_file: {
        async deliver(request, ports) {
            await ports.files.writeText(request.promptFile, request.prompt);
        },
    },
    stdin: {
        async deliver(request, _ports, child) {
            await writeStdinPromptAndClose(child, request.prompt);
        },
    },
});
export async function writeClaudePrompt(child, prompt) {
    await child.writeStdin(`${JSON.stringify(toClaudeInputMessage(prompt))}\n`);
}
export async function writeStdinPromptAndClose(child, prompt) {
    await child.writeStdin(prompt);
    await child.closeStdin();
}
export async function deliverGenericPrompt(request, ports, child) {
    await PROMPT_DELIVERY_POLICIES[request.engine.promptDelivery].deliver(request, ports, child);
}
function toClaudeInputMessage(prompt) {
    return {
        type: 'user',
        message: {
            role: 'user',
            content: [{ type: 'text', text: prompt }],
        },
    };
}
