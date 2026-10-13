import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, symlinkSync, linkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { credentialRoute, selectedWrite, questionSchema } from './policy.ts';

test('missing API/cloud credentials never selects installed subscription auth', () => {
  assert.equal(credentialRoute({}), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_OAUTH_TOKEN: 'unit-test-not-a-real-credential' }), undefined);
  assert.equal(credentialRoute({ ANTHROPIC_API_KEY: 'unit-test-not-a-real-credential' }), 'ANTHROPIC_API_KEY');
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: '1' }), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: 'false', AWS_ACCESS_KEY_ID: 'test', AWS_SECRET_ACCESS_KEY: 'test' }), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: '1', AWS_ACCESS_KEY_ID: 'test', AWS_SECRET_ACCESS_KEY: 'test' }), 'Amazon Bedrock (provider-owned AWS credentials)');
  assert.equal(credentialRoute({ ANTHROPIC_API_KEY: 'unit-test', CLAUDE_CODE_USE_VERTEX: '1' }), undefined);
});

test('selected writes reject protected/unselected/escaping/symlink routes without touching files', () => {
  const root = mkdtempSync(join(tmpdir(), 'eevee-sdk-policy-'));
  try {
    writeFileSync(join(root, 'selected.ts'), 'dirty user working content');
    writeFileSync(join(root, 'protected.ts'), 'protected');
    assert.equal(selectedWrite(root, ['selected.ts'], 'selected.ts'), true);
    assert.equal(selectedWrite(root, ['selected.ts'], 'protected.ts'), false);
    assert.equal(selectedWrite(root, ['selected.ts'], '../escaped.ts'), false);
    assert.equal(selectedWrite(root, ['new.ts'], 'new.ts'), true);
    symlinkSync(join(root, 'protected.ts'), join(root, 'linked.ts'));
    assert.equal(selectedWrite(root, ['linked.ts'], 'linked.ts'), false);
    linkSync(join(root, 'protected.ts'), join(root, 'hardlinked.ts'));
    assert.equal(selectedWrite(root, ['hardlinked.ts'], 'hardlinked.ts'), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('question schema does not invent unsupported multi-select or duplicate answers', () => {
  const question = { question: 'Which database?', header: 'Database', multiSelect: false, options: [{ label: 'SQLite', description: 'Local' }] };
  assert.equal(questionSchema({ questions: [question] }), true);
  assert.equal(questionSchema({ questions: [{ ...question, multiSelect: true }] }), false);
  assert.equal(questionSchema({ questions: [question, question] }), false);
  assert.equal(questionSchema({ questions: [] }), false);
});
