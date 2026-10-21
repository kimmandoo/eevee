import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, linkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { credentialRoute, consoleProfile, accountMatches, selectedWrite, selectedWriteIdentity, questionSchema, sdkOutcome, RunSchema, SelectionSchema, modelCatalogue, prepareModels, assertAppliedModel, SDKPackageSchema, assertInitialization, assertSessionPolicy, SDKDispatchReceipt, SDKUsageLedger, BRIDGE_POLICY } from './policy.ts';

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

test('catalogue omissions do not erase custom identity or turn requested effort into an applied claim', () => {
  const catalogue = modelCatalogue([
    { value: 'known', displayName: 'Known', description: '', supportsEffort: true, supportedEffortLevels: ['high'] },
    { value: 'without-capability', displayName: 'Other', description: '', supportedEffortLevels: ['max'] },
  ], { model: 'custom-exact', effort: 'medium' });
  assert.equal(catalogue.current, 'custom-exact');
  assert.deepEqual(catalogue.models.map(model => model.efforts), [['high'], []]);
  assert.equal(modelCatalogue([], { model: 'custom-exact', effort: null }).current, 'custom-exact');
});

test('catalogues reject duplicate identifiers, oversized fields and display controls', () => {
  const model = { value: 'a', displayName: 'A', description: 'Live' };
  assert.throws(() => modelCatalogue([model, model], { model: null, effort: null }), /Duplicate/);
  assert.throws(() => modelCatalogue(Array.from({ length: 4097 }, (_, index) => ({ ...model, value: `m-${index}` })), { model: null, effort: null }), /invalid bounds/);
  for (const changed of [{ value: 'bad\u202e' }, { displayName: 'A'.repeat(513) }, { description: 'x'.repeat(4097) }, { description: 'line\nbreak' }]) {
    assert.throws(() => modelCatalogue([{ ...model, ...changed }], { model: null, effort: null }), /invalid bounds/);
  }
});

test('strict inspection commands cannot carry prompts, prior sessions, writes or unreviewed options', () => {
  const command = { protocol: 1, type: 'run', generation: 1, task_id: 1, request_id: null, cwd: '/project', state_dir: '/state', prompt: '',
    operation: 'plan', edit_paths: [], resume: null, inspect_models: true, selection: { model: null, effort: null } };
  assert.equal(RunSchema.safeParse(command).success, true);
  for (const changed of [{ request_id: 'claimed-inference' }, { prompt: 'must not send' }, { resume: '01234567-89ab-4cde-8123-456789abcdef' },
    { edit_paths: ['file.ts'] }, { operation: 'implement' }, { tools: ['Bash'] }, { selection: { model: null, effort: null, fallback: 'a' } }]) {
    assert.equal(RunSchema.safeParse({ ...command, ...changed }).success, false);
  }
  assert.equal(SelectionSchema.safeParse({ model: 'a\u001b', effort: null }).success, false);
  assert.equal(SelectionSchema.safeParse({ model: 'a', effort: 'invented' }).success, false);
});

test('backend and runtime identity mismatches prevent prompt release independently of discovery', async () => {
  const sdk = {
    async accountInfo() { return { apiProvider: 'firstParty' }; },
    async initializationResult() { return { claude_code_version: compatibility.cliVersion }; },
    async supportedModels() { throw new Error('discovery unavailable'); },
    async setModel() {},
    async getContextUsage() { return { model: 'custom-exact' }; },
  };
  await assert.rejects(() => prepareModels(sdk, { model: 'custom-exact', effort: null },
    'ANTHROPIC_API_KEY', 'bedrock', compatibility), /backend differs/);
  await assert.rejects(() => prepareModels({ ...sdk,
    async initializationResult() { return { claude_code_version: 'other' }; } },
    { model: 'custom-exact', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /version differs/);
  const prepared = await prepareModels(sdk, { model: 'custom-exact', effort: 'high' },
    'ANTHROPIC_API_KEY', 'firstParty', compatibility);
  assert.equal(prepared.approvedModel, 'custom-exact');
  await assert.rejects(() => prepareModels(sdk, { model: null, effort: null },
    'ANTHROPIC_API_KEY', 'firstParty', compatibility, true), /discovery unavailable/);
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
  for (const name of ['accountInfo', 'initializationResult', 'setModel', 'getContextUsage']) {
    const changed = { ...sdk, [name]: undefined } as unknown as typeof sdk;
    await assert.rejects(() => prepareModels(changed, { model: 'a', effort: null }, 'ANTHROPIC_API_KEY', 'firstParty', compatibility), /required control unavailable/);
  }
  assert.equal(modelsRequested, false);
});

test('dispatch receipts require admitted exact SDK submission acknowledgment, never bridge enqueue or metadata', () => {
  const uuid = '01234567-89ab-4cde-8123-456789abcdef';
  const ack = { type: 'user', isReplay: true, uuid };
  const receipt = new SDKDispatchReceipt(uuid, 'frozen-request');
  assert.equal(BRIDGE_POLICY.dispatch, 'correlated-sdk-submission');
  assert.equal(receipt.observe(ack), undefined, 'failed admission never arms a receipt');
  receipt.submittedToSDK();
  for (const message of [
    { type: 'system', subtype: 'init', uuid },
    { type: 'status', user_message_uuid: uuid },
    { type: 'user', uuid },
    { type: 'user', isReplay: true, uuid: 'older-history' },
    { type: 'assistant', user_message_uuid: 'another-submission' },
    { type: 'result', subtype: 'error_during_execution' },
    { type: 'stream_event', uuid },
    { type: 'run', request_id: 'frozen-request' },
  ]) assert.equal(receipt.observe(message), undefined);
  assert.equal(receipt.observe(ack), 'frozen-request');
  assert.equal(receipt.observe(ack), undefined, 'duplicate acknowledgment never counts twice');
  assert.equal(receipt.observe({ type: 'assistant', user_message_uuid: uuid }), undefined);
});

test('correlated SDK reply evidence covers streamed, assistant and error replies without inventing owner IDs', () => {
  for (const type of ['stream_event', 'assistant', 'result']) {
    const receipt = new SDKDispatchReceipt('fresh-sdk-message', 'immutable-owner-request');
    receipt.submittedToSDK();
    assert.equal(receipt.observe({ type, user_message_uuid: 'fresh-sdk-message' }), 'immutable-owner-request');
    assert.equal(receipt.observe({ type, user_message_uuid: 'fresh-sdk-message' }), undefined);
    for (const id of [null, undefined]) {
      const unclaimed = new SDKDispatchReceipt('fresh-sdk-message', id);
      unclaimed.submittedToSDK();
      assert.equal(unclaimed.observe({ type, user_message_uuid: 'fresh-sdk-message' }), undefined);
    }
  }
});

// Contract-shaped fixtures from installed SDK 0.3.300 sdk.d.ts result fields.
// These are deterministic fixtures, not recorded/live runtime evidence.
const usageSession = '01234567-89ab-4cde-8123-456789abcdef';
const submittedFirst = '11234567-89ab-4cde-8123-456789abcdef';
const submittedSecond = '21234567-89ab-4cde-8123-456789abcdef';
const submittedResume = '31234567-89ab-4cde-8123-456789abcdef';
function usageResult(uuid: string, input: number, cumulative: number) {
  return { type: 'result', uuid: `result-${uuid}`, session_id: usageSession, user_message_uuid: uuid, is_error: false, subtype: 'success',
    usage: { input_tokens: input, output_tokens: 5, cache_read_input_tokens: null },
    modelUsage: { 'provider-model-alias': { inputTokens: cumulative, outputTokens: 15, thinkingTokens: 2 } } };
}

test('usage requires actual SDK submission and exact current UUID, not restored history or admission', () => {
  const ledger = new SDKUsageLedger();
  const current = usageResult(submittedFirst, 10, 100);
  assert.deepEqual(ledger.observe(current), []);
  ledger.submitted(submittedFirst, 'frozen-request-1');
  assert.deepEqual(ledger.observe({ ...current, user_message_uuid: submittedSecond }), []);
  assert.deepEqual(ledger.observe({ ...current, isReplay: true }), []);
  assert.deepEqual(ledger.observe({ ...current, user_message_uuid: undefined }), []);
  const reports = ledger.observe(current);
  assert.equal(reports.length, 2);
  assert.equal(reports[0].request_id, 'frozen-request-1');
  assert.equal(reports[0].submission_id, submittedFirst);
  assert.equal(reports[0].report.scope, 'MainLoopTurn');
  assert.equal(reports[0].report.counters.input, 10);
  assert.equal(reports[0].report.counters.cache_read, null);
  assert.equal(reports[0].report.counters.cache_write, null);
  assert.equal(reports[0].report.model, null);
  assert.equal(reports[0].report.tokenizer, null);
  assert.equal(reports[1].report.scope, 'ModelPipeline');
  assert.equal(reports[1].report.model, 'provider-model-alias');
  assert.equal(reports[1].report.counters.reasoning, 2);
  assert.equal(reports[1].report.semantics.reasoning_in_output, true);
  assert.equal(reports[1].report.baseline, null);
  ledger.submitted(submittedSecond, null);
  assert.deepEqual(ledger.observe(usageResult(submittedSecond, 20, 120)), []);
});

test('two submitted turns freeze cumulative baselines and never charge prior turns as main-loop usage', () => {
  const ledger = new SDKUsageLedger();
  ledger.submitted(submittedFirst, 'request-1');
  const first = usageResult(submittedFirst, 10, 100);
  ledger.observe(first);
  ledger.observe(first); // A repeated final snapshot is not a second increment.
  ledger.submitted(submittedSecond, 'request-2');
  assert.deepEqual(ledger.observe(first), [], 'prior turn cannot be billed to current owner');
  const second = usageResult(submittedSecond, 20, 120);
  const reports = ledger.observe(second);
  assert.equal(reports[0].report.identity.turn, submittedSecond);
  assert.equal(reports[0].report.counters.input, 20);
  assert.equal(reports[0].report.baseline, null);
  assert.equal(reports[1].report.counters.input, 120);
  assert.equal(reports[1].report.baseline?.input, 100);
  assert.equal(reports[1].request_id, 'request-2');
  const partial = ledger.observe({ ...second, uuid: 'repeated-final', modelUsage: {
    'provider-model-alias': { inputTokens: null, outputTokens: 19 },
  } });
  assert.deepEqual(partial[1].report.identity.turn, reports[1].report.identity.turn);
  assert.equal(partial[1].report.counters.input, null);
  assert.equal(partial[1].report.baseline?.input, 100, 'baseline stays frozen across final/partial reports');
});

test('resumed query keeps restored model totals unknown without an observed baseline', () => {
  const resumed = new SDKUsageLedger();
  resumed.submitted(submittedResume, 'request-resumed');
  assert.deepEqual(resumed.observe(usageResult(submittedFirst, 10, 100)), []);
  const reports = resumed.observe(usageResult(submittedResume, 7, 1007));
  assert.equal(reports[0].report.counters.input, 7);
  assert.equal(reports[1].report.counters.input, 1007);
  assert.equal(reports[1].report.baseline, null, 'prior transcript is not an observed current-query baseline');
  assert.deepEqual(resumed.observe({ ...usageResult(submittedResume, 7, 1007),
    usage: null, modelUsage: null }), []);
});

test('failed correlated results preserve reported usage without establishing a completed baseline', () => {
  const ledger = new SDKUsageLedger();
  ledger.submitted(submittedFirst, 'failed-request');
  const failed = { ...usageResult(submittedFirst, 10, 100), is_error: true, subtype: 'error_max_turns' };
  const reports = ledger.observe(failed);
  assert.equal(reports.length, 2);
  assert.equal(reports[0].report.counters.input, 10);
  assert.equal(reports[1].report.counters.input, 100);
  assert.ok(reports.every(value => !value.report.complete));
  ledger.submitted(submittedSecond, 'next-request');
  assert.equal(ledger.observe(usageResult(submittedSecond, 20, 120))[1].report.baseline, null);
});
