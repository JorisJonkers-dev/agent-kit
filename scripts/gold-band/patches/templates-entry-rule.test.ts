// Verification for scripts/gold-band/patches/0001-entry-rule-failure-targets.patch (not part of CI:
// it needs Gold Band's source with `npm ci`). Copy it to web/tests/ in a patched checkout and run
//   GB_TEMPLATES=<agent-kit>/installer/gold-band/workflows npx vitest run --config web/vitest.config.ts web/tests/templates-entry-rule.test.ts
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { validateWorkflowForSave } from '../src/components/WorkflowEditor';
import { readyWorkflowProfileCatalog } from '../src/lib/workflow-profile-catalog';

const dir = process.env.GB_TEMPLATES ?? '';
const files = dir ? readdirSync(dir).filter((name) => name.endsWith('.json')).sort() : [];
const t = (key: string, options?: Record<string, unknown>) => `${key}${options ? `:${JSON.stringify(options)}` : ''}`;

describe('shipped templates against the entry rule', () => {
  it('finds the templates', () => expect(files.length).toBeGreaterThan(0));
  files.forEach((name) => {
    it(`${name} has exactly the declared entry`, () => {
      const { workflow } = JSON.parse(readFileSync(join(dir, name), 'utf8'));
      const result = validateWorkflowForSave(workflow, readyWorkflowProfileCatalog([]), [], t, null, null, null, false,
        { definitionRevision: '', bindingRevision: 0, bindings: [] }, false);
      const entryIssues = result.issues.filter((issue) => issue.message.includes('validationEntryCandidate'));
      expect(entryIssues.map((issue) => issue.message)).toEqual([]);
      expect(result.sanitizedWorkflow.entry).toBe(workflow.entry);
    });
  });
});
