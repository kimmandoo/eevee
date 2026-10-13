import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, symlinkSync, linkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { credentialRoute, selectedWrite, selectedWriteIdentity, questionSchema, sdkOutcome } from './policy.ts';

test('missing API/cloud credentials never selects installed subscription auth', () => {
  assert.equal(credentialRoute({}), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_OAUTH_TOKEN: 'unit-test-not-a-real-credential' }), undefined);
  assert.equal(credentialRoute({ ANTHROPIC_API_KEY: 'unit-test-not-a-real-credential' }), 'ANTHROPIC_API_KEY');
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: '1' }), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: 'false', AWS_ACCESS_KEY_ID: 'test', AWS_SECRET_ACCESS_KEY: 'test' }), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_BEDROCK: '1', AWS_ACCESS_KEY_ID: 'test', AWS_SECRET_ACCESS_KEY: 'test' }), 'Amazon Bedrock (provider-owned AWS credentials)');
  assert.equal(credentialRoute({ ANTHROPIC_API_KEY: 'unit-test', CLAUDE_CODE_USE_VERTEX: '1' }), undefined);
});

test('cloud chains are explicit external policy, not automatic profile or subscription discovery', () => {
  const optIn = { EEVEE_CLAUDE_CLOUD_AUTH: 'provider-chain' };
  const bedrock = { CLAUDE_CODE_USE_BEDROCK: '1' };
  assert.equal(credentialRoute({ ...bedrock, AWS_PROFILE: 'work' }), undefined);
  assert.match(credentialRoute({ ...bedrock, ...optIn, AWS_PROFILE: 'work' })!, /provider credential chain/);
  assert.match(credentialRoute({ ...bedrock, AWS_BEARER_TOKEN_BEDROCK: 'unit-test' })!, /bearer/);
  assert.match(credentialRoute({ ...bedrock, AWS_WEB_IDENTITY_TOKEN_FILE: '/selected/cloud-file', AWS_ROLE_ARN: 'unit-role' })!, /web identity/);
  assert.match(credentialRoute({ CLAUDE_CODE_USE_VERTEX: '1', ANTHROPIC_VERTEX_PROJECT_ID: 'unit-project', ...optIn })!, /ADC/);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_VERTEX: '1', ...optIn }), undefined);
  assert.equal(credentialRoute(optIn), undefined);
  assert.equal(credentialRoute({ ...optIn, ANTHROPIC_API_KEY: 'unit-test' }), undefined);
  assert.equal(credentialRoute({ ...bedrock, ...optIn, CLAUDE_CODE_USE_VERTEX: '1' }), undefined);
  assert.equal(credentialRoute({ ...bedrock, EEVEE_CLAUDE_CLOUD_AUTH: 'implicit' }), undefined);
  assert.equal(credentialRoute({ CLAUDE_CODE_USE_VERTEX: '1', ANTHROPIC_VERTEX_PROJECT_ID: 'unit-project', GOOGLE_APPLICATION_CREDENTIALS: '/selected/cloud-file', GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES: '1' }), undefined);
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

test('interrupt requests cannot relabel SDK success or ambiguous errors as cancelled', () => {
  assert.equal(sdkOutcome(false, false, 'success'), 'completed');
  assert.equal(sdkOutcome(true, false, 'success'), 'completed');
  assert.equal(sdkOutcome(false, true, 'error_during_execution'), 'failed');
  assert.equal(sdkOutcome(true, true, 'error_during_execution'), 'unknown');
  assert.equal(sdkOutcome(true, true, 'error_max_turns'), 'unknown');
});

test('write consent identity becomes stale after edits, file creation, or symlink replacement', () => {
  const root = mkdtempSync(join(tmpdir(), 'eevee-sdk-stale-'));
  try {
    const paths = ['selected.ts', 'new.ts'];
    writeFileSync(join(root, 'selected.ts'), 'before');
    const before = selectedWriteIdentity(root, paths, 'selected.ts');
    assert.notEqual(before, undefined);
    writeFileSync(join(root, 'selected.ts'), 'new user working changes');
    assert.notEqual(selectedWriteIdentity(root, paths, 'selected.ts'), before);
    const absent = selectedWriteIdentity(root, paths, 'new.ts');
    assert.notEqual(absent, undefined);
    writeFileSync(join(root, 'new.ts'), 'new user file');
    assert.notEqual(selectedWriteIdentity(root, paths, 'new.ts'), absent);
    rmSync(join(root, 'selected.ts'));
    symlinkSync(join(root, 'new.ts'), join(root, 'selected.ts'));
    assert.equal(selectedWriteIdentity(root, paths, 'selected.ts'), undefined);
    assert.equal(selectedWriteIdentity(root, paths, '../outside.ts'), undefined);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
