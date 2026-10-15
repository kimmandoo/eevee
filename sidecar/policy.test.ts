import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, linkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { credentialRoute, consoleProfile, accountMatches, selectedWrite, selectedWriteIdentity, questionSchema, sdkOutcome, RunSchema, SelectionSchema, querySelection, modelCatalogue, prepareModels, assertAppliedModel, SDKPackageSchema, assertInitialization, assertSessionPolicy } from './policy.ts';

const compatibility = { sdkVersion: '0.3.999', cliVersion: '2.1.999', requireHooks: false };

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

test('Console OAuth profile is an exclusive explicit route, never a subscription fallback', () => {
  const dir = mkdtempSync(join(tmpdir(), 'eevee-ant-profile-'));
  try {
    const env = { ANTHROPIC_CONFIG_DIR: dir };
    assert.equal(consoleProfile(env), undefined);
    mkdirSync(join(dir, 'configs')); mkdirSync(join(dir, 'credentials'));
    writeFileSync(join(dir, 'configs', 'eevee.json'), '{}');
    assert.equal(consoleProfile(env), undefined);
    writeFileSync(join(dir, 'credentials', 'eevee.json'), '{}');
    const profile = consoleProfile(env)!;
    assert.equal(profile.name, 'eevee');
    assert.match(credentialRoute(env)!, /Console OAuth profile eevee/);
    assert.equal(credentialRoute({ ...env, ANTHROPIC_API_KEY: 'unit-test' }), undefined);
    assert.equal(credentialRoute({ ...env, CLAUDE_CODE_USE_BEDROCK: '1', EEVEE_CLAUDE_CLOUD_AUTH: 'provider-chain' }), undefined);
    assert.equal(credentialRoute({ ...env, ANTHROPIC_PROFILE: 'work', ANTHROPIC_API_KEY: 'unit-test' }), undefined);
    assert.equal(consoleProfile({ ...env, ANTHROPIC_PROFILE: '../eevee' }), undefined);
    assert.equal(consoleProfile({ ANTHROPIC_CONFIG_DIR: 'relative' }), undefined);
    assert.equal(consoleProfile({ XDG_CONFIG_HOME: join(dir, 'missing') }), undefined);
    const route = credentialRoute(env)!;
    assert.equal(accountMatches(route, { apiProvider: 'firstParty' }, 'firstParty'), true);
    assert.equal(accountMatches(route, { apiProvider: 'firstParty', tokenSource: 'none' }, 'firstParty'), false);
    assert.equal(accountMatches(route, { apiProvider: 'firstParty', tokenSource: 'none', apiKeySource: 'ANTHROPIC_API_KEY' }, 'firstParty'), false);
    assert.equal(accountMatches(route, { apiProvider: 'gateway' }, 'firstParty'), false);
    assert.equal(accountMatches('ANTHROPIC_API_KEY', { apiProvider: 'firstParty', apiKeySource: 'ANTHROPIC_API_KEY' }, 'firstParty'), true);
  } finally { rmSync(dir, { recursive: true, force: true }); }
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

test('capability metadata cannot advertise selectable effort without an applied-effort acknowledgement', () => {
  const catalogue = modelCatalogue([
    { value: 'alias-a', resolvedModel: 'wire-a', displayName: 'A', description: 'Advertised', supportsEffort: true, supportedEffortLevels: ['medium', 'high'] },
    { value: 'b', displayName: 'B', description: 'No capability field', supportedEffortLevels: ['max'] },
    { value: 'c', displayName: 'C', description: 'No levels', supportsEffort: true },
  ], { model: 'wire-a', effort: 'medium' });
  assert.deepEqual(catalogue.models.map(model => model.efforts), [[], [], []]);
  assert.match(catalogue.effort_note, /no public pre-prompt acknowledgement.*organization caps/);
  assert.equal(catalogue.current, 'alias-a');
  assert.equal(catalogue.models[0].default_effort, null);
  assert.deepEqual(querySelection({ model: 'wire-a', effort: 'medium' }), { model: 'wire-a', effort: 'medium' });
  assert.deepEqual(querySelection({ model: null, effort: null }), { model: undefined, effort: undefined });
  assert.match(modelCatalogue([], { model: null, effort: null }).effort_note, /no supported models/);
});

test('selection rejects unknown models and efforts without guessing a model default', () => {
  const models = [{ value: 'a', displayName: 'A', description: 'Live', supportsEffort: true, supportedEffortLevels: ['high' as const] }];
  assert.throws(() => modelCatalogue(models, { model: 'absent', effort: null }), /not advertised/);
  assert.throws(() => modelCatalogue(models, { model: 'a', effort: 'medium' }), /unsupported/);
  assert.throws(() => modelCatalogue(models, { model: null, effort: 'high' }), /unsupported/);
  assert.throws(() => modelCatalogue([{ ...models[0], supportsEffort: false }], { model: 'a', effort: 'high' }), /unsupported/);
  assert.throws(() => modelCatalogue([{ ...models[0], supportedEffortLevels: undefined }], { model: 'a', effort: 'high' }), /unsupported/);
});

test('catalogues reject duplicate identifiers, oversized fields and display controls', () => {
  const model = { value: 'a', displayName: 'A', description: 'Live' };
  assert.throws(() => modelCatalogue([model, model], { model: null, effort: null }), /Duplicate/);
  assert.throws(() => modelCatalogue(Array.from({ length: 257 }, (_, index) => ({ ...model, value: `m-${index}` })), { model: null, effort: null }), /invalid bounds/);
  for (const changed of [{ value: 'bad\u202e' }, { displayName: 'A'.repeat(513) }, { description: 'x'.repeat(4097) }, { description: 'line\nbreak' }]) {
    assert.throws(() => modelCatalogue([{ ...model, ...changed }], { model: null, effort: null }), /invalid bounds/);
  }
});

test('strict inspection commands cannot carry prompts, prior sessions, writes or unreviewed options', () => {
  const command = { protocol: 1, type: 'run', generation: 1, task_id: 1, cwd: '/project', state_dir: '/state', prompt: '',
    operation: 'plan', edit_paths: [], resume: null, inspect_models: true, selection: { model: null, effort: null } };
  assert.equal(RunSchema.safeParse(command).success, true);
  for (const changed of [{ prompt: 'must not send' }, { resume: '01234567-89ab-4cde-8123-456789abcdef' },
    { edit_paths: ['file.ts'] }, { operation: 'implement' }, { tools: ['Bash'] }, { selection: { model: null, effort: null, fallback: 'a' } }]) {
    assert.equal(RunSchema.safeParse({ ...command, ...changed }).success, false);
  }
  assert.equal(SelectionSchema.safeParse({ model: 'a\u001b', effort: null }).success, false);
  assert.equal(SelectionSchema.safeParse({ model: 'a', effort: 'invented' }).success, false);
});

test('SDK controls validate backend and live selection before the consumer can release a prompt', async () => {
  const calls: string[] = [];
  const sdk = {
    async accountInfo() { calls.push('account'); return { apiProvider: 'firstParty' }; },
    async initializationResult() { calls.push('initialize'); return { claude_code_version: compatibility.cliVersion }; },
    async supportedModels() { calls.push('models'); return [{ value: 'a', displayName: 'A', description: 'Live', supportsEffort: true, supportedEffortLevels: ['high' as const] }]; },
    async setModel(model?: string) { calls.push(`set:${model}`); },
    async getContextUsage(options: { detail: 'summary' }) { calls.push(`context:${options.detail}`); return { model: 'a' }; },
  };
  let released = false;
  await assert.rejects(async () => {
    await prepareModels(sdk, { model: 'a', effort: 'medium' }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility);
    released = true;
  }, /unsupported/);
  assert.equal(released, false);
  assert.deepEqual(calls, ['account', 'initialize', 'models']);
  calls.length = 0;
  await assert.rejects(() => prepareModels(sdk, { model: null, effort: null }, 'ANTHROPIC_API_KEY', 'bedrock', compatibility), /backend differs/);
  assert.deepEqual(calls, ['account']);
  calls.length = 0;
  await assert.rejects(() => prepareModels({ ...sdk, async initializationResult() { return { claude_code_version: 'other' }; } },
    { model: null, effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /version differs/);
  assert.deepEqual(calls, ['account']);
  calls.length = 0;
  for (const effort of [undefined, null, 'low', 'high']) {
    await assert.rejects(() => prepareModels({
      ...sdk,
      async getContextUsage() { calls.push('unofficial-effort'); return { model: 'a', effort }; },
    }, { model: 'a', effort: 'high' }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /pre-prompt applied-effort acknowledgement/);
  }
  assert.equal(calls.includes('unofficial-effort'), false);
  calls.length = 0;
  const prepared = await prepareModels(sdk, { model: 'a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility);
  released = true;
  assert.equal(released, true);
  assert.equal(prepared.approvedModel, 'a');
  assert.deepEqual(calls, ['account', 'initialize', 'models', 'set:a', 'context:summary']);
});

test('model qualification accepts only the authoritative resolved alias and rejects unknown or mismatched acknowledgements', async () => {
  const calls: string[] = [];
  const sdk = {
    async accountInfo() { return { apiProvider: 'firstParty' }; },
    async initializationResult() { return { claude_code_version: compatibility.cliVersion }; },
    async supportedModels() { return [{ value: 'alias-a', resolvedModel: 'wire-a', displayName: 'A', description: 'Live' }]; },
    async setModel(model?: string) { calls.push(`set:${model}`); },
    async getContextUsage(options: { detail: 'summary' }) { calls.push(options.detail); return { model: 'wire-a' }; },
  };
  const selected = await prepareModels(sdk, { model: 'alias-a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility);
  assert.equal(selected.approvedModel, 'wire-a');
  assert.deepEqual(calls, ['set:alias-a', 'summary']);
  const canonical = await prepareModels(sdk, { model: 'wire-a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility);
  assert.equal(canonical.approvedModel, 'wire-a');
  for (const actual of ['', 'alias-a', 'wire-other']) {
    let released = false;
    await assert.rejects(async () => {
      await prepareModels({ ...sdk, async getContextUsage() { return { model: actual }; } },
        { model: 'alias-a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility);
      released = true;
    }, /effective model differs/);
    assert.equal(released, false);
  }
  await assert.rejects(() => prepareModels({ ...sdk, async getContextUsage() { throw new Error('Unavailable'); } },
    { model: 'alias-a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /could not be acknowledged/);
  calls.length = 0;
  await prepareModels(sdk, { model: null, effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility, true);
  assert.deepEqual(calls, []);
  assert.doesNotThrow(() => assertAppliedModel('wire-a', 'wire-a'));
  for (const actual of [undefined, null, 'alias-a', 'wire-other']) {
    assert.throws(() => assertAppliedModel('wire-a', actual), /effective model differs/);
  }
});

test('new SDK releases remain admissible identity, not proof of required capabilities', () => {
  const metadata = { name: '@anthropic-ai/claude-agent-sdk', version: '1.5.700', claudeCodeVersion: '4.2.800' };
  assert.deepEqual(SDKPackageSchema.parse(metadata), metadata);
  for (const changed of [{ name: 'different-sdk' }, { version: '' }, { version: 'latest' },
    { claudeCodeVersion: undefined }, { claudeCodeVersion: '4.2.800\n' }, { version: '1'.repeat(129) }]) {
    assert.equal(SDKPackageSchema.safeParse({ ...metadata, ...changed }).success, false);
  }
  const observed = { sdkVersion: metadata.version, cliVersion: metadata.claudeCodeVersion, requireHooks: true };
  assert.doesNotThrow(() => assertInitialization({ claude_code_version: observed.cliVersion, hooks_applied: true }, observed));
  for (const raw of [{}, { claude_code_version: '4.2.801', hooks_applied: true }, { claude_code_version: observed.cliVersion },
    { claude_code_version: observed.cliVersion, hooks_applied: false }, { claude_code_version: observed.cliVersion, hooks_applied: 'true' }]) {
    assert.throws(() => assertInitialization(raw, observed));
  }
  assert.doesNotThrow(() => assertInitialization({ claude_code_version: observed.cliVersion }, { ...observed, requireHooks: false }));
});

test('new session schemas cannot silently widen tools, permissions, MCP, plugins or skills', () => {
  const message = { claude_code_version: compatibility.cliVersion, permissionMode: 'default',
    tools: ['Read'], mcp_servers: [], plugins: [], skills: [] };
  assert.doesNotThrow(() => assertSessionPolicy({ ...message, capabilities: ['future-independent-capability'] }, compatibility.cliVersion, ['Read']));
  for (const changed of [{ tools: ['Read', 'FutureWriteTool'] }, { tools: undefined }, { permissionMode: 'bypassPermissions' },
    { mcp_servers: [{}] }, { plugins: [{}] }, { skills: ['future-skill'] },
    { claude_code_version: '2.1.1000' }]) {
    assert.throws(() => assertSessionPolicy({ ...message, ...changed }, compatibility.cliVersion, ['Read']));
  }
});

test('missing live controls and unacknowledged mandatory hooks fail before model discovery', async () => {
  let modelsRequested = false;
  const sdk = {
    async accountInfo() { return { apiProvider: 'firstParty' }; },
    async initializationResult() { return { claude_code_version: compatibility.cliVersion }; },
    async supportedModels() { modelsRequested = true; return []; },
    async setModel() {},
    async getContextUsage() { return { model: 'a' }; },
  };
  await assert.rejects(() => prepareModels(sdk, { model: null, effort: null }, 'ANTHROPIC_API_KEY', 'firstParty',
    { ...compatibility, requireHooks: true }), /mandatory tool policy hooks/);
  for (const name of ['accountInfo', 'initializationResult', 'supportedModels', 'setModel', 'getContextUsage']) {
    const changed = { ...sdk, [name]: undefined } as unknown as typeof sdk;
    await assert.rejects(() => prepareModels(changed, { model: 'a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /required control unavailable/);
  }
  assert.equal(modelsRequested, false);
});
