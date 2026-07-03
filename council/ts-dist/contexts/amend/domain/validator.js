const restructuringActions = new Set(['modify', 'cancel']);
export function validateAmendmentDelta(delta, context) {
    const taskById = new Map(context.tasks.map((task) => [task.id, task]));
    const errors = [];
    const guardrails = [];
    const addedIds = new Set();
    let cancelCount = 0;
    if (!Number.isInteger(delta.plan_version) || delta.plan_version <= context.current_plan_version) {
        errors.push({
            code: 'invalid_plan_version',
            message: 'plan_version must be an integer greater than current_plan_version',
        });
    }
    delta.operations.forEach((operation, operationIndex) => {
        if (operation.action === 'add') {
            validateAddOperation(operation, operationIndex, taskById, addedIds, errors);
            return;
        }
        const target = taskById.get(operation.id);
        if (target === undefined) {
            errors.push({
                code: 'unknown_task_id',
                message: 'operation references a task id that is not in the current plan',
                operation_index: operationIndex,
                task_id: operation.id,
            });
            return;
        }
        if (operation.action === 'cancel') {
            cancelCount += 1;
        }
        validateLandedWorkGuardrail(target, operationIndex, guardrails);
        validateRestructuringInvariant(operation, target, operationIndex, errors);
        validateStableModifyId(operation, operationIndex, errors);
    });
    if (cancelCount > 2) {
        guardrails.push({
            code: 'bulk_cancel',
            message: 'more than two cancels require a human checkpoint',
        });
    }
    return {
        valid: errors.length === 0,
        errors,
        guardrails,
        requires_human_checkpoint: guardrails.length > 0,
        next_plan_version: delta.plan_version,
    };
}
function validateAddOperation(operation, operationIndex, taskById, addedIds, errors) {
    if (operation.task.id.length === 0) {
        errors.push({
            code: 'missing_task_id',
            message: 'added tasks must carry a stable id',
            operation_index: operationIndex,
        });
        return;
    }
    if (taskById.has(operation.task.id) || addedIds.has(operation.task.id)) {
        errors.push({
            code: 'duplicate_task_id',
            message: 'added task id must not duplicate an existing or newly added task',
            operation_index: operationIndex,
            task_id: operation.task.id,
        });
        return;
    }
    addedIds.add(operation.task.id);
}
function validateLandedWorkGuardrail(task, operationIndex, guardrails) {
    if (task.status !== 'landed' && task.has_landed_work !== true) {
        return;
    }
    guardrails.push({
        code: 'landed_work',
        message: 'amending a task with landed work requires a human checkpoint',
        operation_index: operationIndex,
        task_id: task.id,
    });
}
function validateRestructuringInvariant(operation, task, operationIndex, errors) {
    if (!restructuringActions.has(operation.action) || task.status === 'unstarted') {
        return;
    }
    errors.push({
        code: 'restructure_started_task',
        message: 'modify and cancel may only restructure unstarted tasks',
        operation_index: operationIndex,
        task_id: task.id,
    });
}
function validateStableModifyId(operation, operationIndex, errors) {
    if (operation.action !== 'modify' || operation.task.id === operation.id) {
        return;
    }
    errors.push({
        code: 'id_changed_on_modify',
        message: 'modify must preserve the target task id',
        operation_index: operationIndex,
        task_id: operation.id,
    });
}
