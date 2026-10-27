import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, linkSync, rmSync, lstatSync, readFileSync, realpathSync, renameSync, chmodSync, opendirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import type { ChildProcessWithoutNullStreams } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { credentialRoute, consoleProfile, accountMatches, SDKWritePolicy, SDKFilePreviews, questionSchema, sdkOutcome, RunSchema, SelectionSchema, modelCatalogue, prepareModels, assertAppliedModel, SDKPackageSchema, assertInitialization, assertSessionPolicy, SDKDispatchReceipt, SDKUsageLedger, BRIDGE_POLICY, DISCOVERY_TOOL, DISCOVERY_SERVER, COMMAND_TOOL, COMMAND_SERVER, HostCommandSchema, CommandResultSchema } from './policy.ts';
import type { WriteScope } from './policy.ts';

const compatibility = { sdkVersion: '0.3.999', cliVersion: '2.1.999', requireHooks: false };

// Deterministic reviewed host IPC fixture, frozen before any simulated SDK effects.
function reviewedScope(root: string, paths: string[]): WriteScope {
  const identity = (path: string) => {
    const stat = lstatSync(path, { bigint: true });
    return process.platform === 'win32' ? `${stat.dev}:${stat.ino >> 32n}:${stat.ino & 0xffffffffn}` : `${stat.dev}:${stat.ino}`;
  };
  return { mode: 'Selected', protected_paths: [], excluded_paths: [], root_identity: identity(root), files: paths.map(path => {
    const parts = path.split('/');
    let content: string | null = null;
    try { content = readFileSync(join(root, path), 'utf8'); } catch {}
    return { path, kind: content === null ? 'Create' : 'Replace',
      identity: content === null ? null : identity(join(root, path)),
      sha256: content === null ? null : createHash('sha256').update(content).digest('hex'),
      working_content: content,
      ancestors: parts.slice(0, -1).map((_, i) => {
        const path = parts.slice(0, i + 1).join('/');
        let id: string | null = null;
        try { id = identity(join(root, path)); } catch {}
        return { path, identity: id };
      }) };
  }) };
}
function workspaceScope(root: string): WriteScope {
  return { ...reviewedScope(root, []), mode: 'Workspace' };
}

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

test('workspace admission is transactional, bounded and frozen through post-consent checks', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-proposals-')));
  try {
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    assert.equal(policy.begin('invalid', 'Write', { file_path: 'fresh.txt', content: 'x'.repeat(256 * 1024 + 1) }), undefined);
    writeFileSync(join(root, 'fresh.txt'), 'working content after invalid proposal');
    const input = { file_path: 'fresh.txt', content: 'x'.repeat(256 * 1024 - 2) + '\r\n' };
    assert.equal(policy.begin('valid', 'Write', input)?.kind, 'Replace');
    assert.equal(policy.approve('valid', 'Write', input, true, false), true);
    writeFileSync(join(root, 'fresh.txt'), 'external change after consent');
    assert.equal(policy.preToolUse('valid', 'Write', input), false);
    assert.equal(policy.finish('valid', 'Write', input, false), false);
    assert.equal(policy.begin('fresh-authority', 'Write', input), undefined);
    const bounded = new SDKWritePolicy(root, workspaceScope(root));
    for (let i = 0; i < 256; i++) {
      const proposal = { file_path: `file-${i}.txt`, content: '' };
      assert.ok(bounded.begin(`id-${i}`, 'Write', proposal));
      assert.equal(bounded.approve(`id-${i}`, 'Write', proposal, false, false), false);
    }
    assert.equal(bounded.begin('overflow', 'Write', { file_path: 'overflow.txt', content: '' }), undefined);
    assert.equal(bounded.preToolUse('overflow', 'Write', { file_path: 'overflow.txt', content: '' }), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('host exclusion prefixes do not inherit write-target grammar or disable adjacent generation', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-exclusions-')));
  try {
    const scope = workspaceScope(root);
    scope.excluded_paths = ['Context', 'unused.', 'unused ', 'con', 'literal:cache', 'unwritable\nname',
      process.platform === 'win32' ? 'odd/cache' : 'odd\\cache'];
    const policy = new SDKWritePolicy(root, scope);
    assert.equal(policy.begin('excluded', 'Write', { file_path: 'context/new.ts', content: '' }), undefined);
    const input = { file_path: 'Context-prefix.ts', content: 'export const value = 1;\r\n' };
    assert.equal(policy.begin('neighbor', 'Write', input)?.kind, 'Create');
    assert.equal(policy.approve('neighbor', 'Write', input, true, false), true);
    writeFileSync(join(root, input.file_path), input.content);
    assert.equal(policy.finish('neighbor', 'Write', input, true), true);
    assert.equal(readFileSync(join(root, input.file_path), 'utf8'), input.content);
    for (const prefix of ['/outside', '../outside', 'context/../outside', 'context//nested', 'context/']) {
      assert.throws(() => new SDKWritePolicy(root, { ...workspaceScope(root), excluded_paths: [prefix] }));
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('workspace proposals create natural nested files only after exact one-shot approval', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-workspace-')));
  try {
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    const input = { file_path: '생성/nested/main.ts', content: 'export const value = 1;\r\n'.repeat(2048) };
    assert.equal(policy.begin('invalid', 'Write', { file_path: input.file_path, content: 7 }), undefined);
    assert.equal(policy.preToolUse('first', 'Write', input), true);
    assert.deepEqual(policy.begin('first', 'Write', input), { kind: 'Create', path: input.file_path, directories: ['생성', '생성/nested'] });
    assert.equal(policy.approve('foreign', 'Write', input, true, false), false);
    assert.equal(policy.preToolUse('first', 'Write', { ...input, content: 'altered' }), false);
    assert.equal(policy.approve('first', 'Write', input, true, false), true);
    mkdirSync(join(root, '생성/nested'), { recursive: true });
    writeFileSync(join(root, input.file_path), input.content);
    assert.equal(policy.finish('first', 'Write', input, true), true);
    assert.equal(policy.begin('first', 'Write', input), undefined);
    const sibling = { file_path: '생성/nested/empty.ts', content: '' };
    assert.deepEqual(policy.begin('sibling', 'Write', sibling)?.directories, []);
    assert.equal(policy.approve('sibling', 'Write', sibling, true, true), false);
    assert.equal(policy.finish('sibling', 'Write', sibling, false), true);
    assert.equal(readFileSync(join(root, input.file_path), 'utf8'), input.content);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('workspace admission enforces root, component boundaries, exclusions, runtime and alias safety', () => {
  const fixture = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-boundaries-')));
  const root = join(fixture, 'workspace');
  mkdirSync(root);
  try {
    mkdirSync(join(root, 'Data'));
    mkdirSync(join(root, 'Data-prefix'));
    mkdirSync(join(root, 'Context'));
    mkdirSync(join(root, 'Context-prefix'));
    writeFileSync(join(root, 'application'), 'runtime');
    const scope = workspaceScope(root);
    scope.protected_paths = [join(root, 'Data'), join(root, 'application')];
    scope.excluded_paths = ['Context'];
    const policy = new SDKWritePolicy(root, scope);
    for (const file_path of ['Data/new', 'data/new', 'Context/new', 'context/new', 'application', '../outside', '.git/config', '.env.local', 'Bad\\name', join(fixture, 'outside'), process.execPath]) {
      assert.equal(policy.begin('refused', 'Write', { file_path, content: '' }), undefined, file_path);
      assert.equal(policy.preToolUse('refused', 'Write', { file_path, content: '' }), false, file_path);
    }
    assert.equal(policy.preToolUse('prefix', 'Write', { file_path: 'Data-prefix/new', content: '' }), true);
    assert.equal(policy.preToolUse('prefix', 'Write', { file_path: 'Context-prefix/new', content: '' }), true);
    symlinkSync(join(root, 'Data-prefix'), join(root, 'alias'), process.platform === 'win32' ? 'junction' : 'dir');
    assert.equal(policy.begin('link', 'Write', { file_path: 'alias/new', content: '' }), undefined);
    assert.throws(() => new SDKWritePolicy(join(root, 'alias'), scope));
    const frozenRoot = workspaceScope(root);
    renameSync(root, join(fixture, 'old'));
    mkdirSync(root);
    assert.throws(() => new SDKWritePolicy(root, frozenRoot));
  } finally { rmSync(fixture, { recursive: true, force: true }); }
});

test('workspace targets and shared missing parents never refresh stale or failed authority', () => {
  for (const effect of ['target', 'parent', 'failed-parent', 'no-effect'] as const) {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-workspace-stale-')));
    try {
      const policy = new SDKWritePolicy(root, workspaceScope(root));
      const input = { file_path: 'nested/a', content: 'reviewed' };
      assert.ok(policy.begin('a', 'Write', input));
      if (effect === 'failed-parent' || effect === 'no-effect') {
        assert.equal(policy.approve('a', 'Write', input, true, false), true);
        if (effect === 'failed-parent') mkdirSync(join(root, 'nested'));
        assert.equal(policy.finish('a', 'Write', input, false), effect === 'no-effect');
      } else {
        assert.equal(policy.approve('a', 'Write', input, false, false), false);
        mkdirSync(join(root, 'nested'));
        if (effect === 'target') writeFileSync(join(root, 'nested/a'), 'foreign');
      }
      if (effect !== 'no-effect') {
        assert.equal(policy.begin('retry', 'Write', input), undefined);
        assert.equal(policy.begin('sibling', 'Write', { file_path: 'nested/b', content: '' }), undefined);
      } else {
        assert.deepEqual(policy.begin('sibling', 'Write', { file_path: 'nested/b', content: '' })?.directories, ['nested']);
      }
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test('frozen selected writes reject protected, invalid, unselected and linked routes', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-policy-')));
  try {
    writeFileSync(join(root, 'selected.ts'), 'dirty user working content');
    writeFileSync(join(root, 'protected.ts'), 'protected');
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['selected.ts', 'new.ts']));
    assert.equal(policy.preToolUse('1', 'Write', { file_path: 'selected.ts', content: 'x' }), true);
    for (const path of ['protected.ts', '../escaped.ts', '.git/new', '.DS_Store', '.ev-install/a', 'x:y', 'CON.txt', 'a//b', 'a/../new.ts', `${root}/a/../new.ts`]) {
      assert.equal(policy.preToolUse('x', 'Write', { file_path: path, content: '' }), false);
    }
    assert.equal(policy.preToolUse('2', 'Write', { file_path: 'new.ts', content: '' }), true);
    symlinkSync(join(root, 'protected.ts'), join(root, 'new.ts'));
    assert.equal(policy.preToolUse('2', 'Write', { file_path: 'new.ts', content: '' }), false);
    linkSync(join(root, 'selected.ts'), join(root, 'hardlinked.ts'));
    assert.equal(policy.preToolUse('1', 'Write', { file_path: 'selected.ts', content: '' }), false);
    assert.throws(() => new SDKWritePolicy(root, reviewedScope(root, ['.git/new'])));
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

test('frozen creation and dirty-file expectations cannot be recaptured after external changes', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-stale-')));
  try {
    writeFileSync(join(root, 'selected.ts'), 'before');
    const scope = reviewedScope(root, ['selected.ts', 'new.ts', 'nested/child.ts']);
    const policy = new SDKWritePolicy(root, scope);
    writeFileSync(join(root, 'selected.ts'), 'new user working changes');
    writeFileSync(join(root, 'new.ts'), 'new user file');
    mkdirSync(join(root, 'nested'));
    for (const file_path of ['selected.ts', 'new.ts', 'nested/child.ts']) {
      assert.equal(policy.preToolUse('id', 'Write', { file_path, content: 'x' }), false);
      assert.equal(policy.begin('id', 'Write', { file_path, content: 'x' }), undefined);
    }
    // Delivery after an external appearance must not silently recapture Create.
    const late = new SDKWritePolicy(root, scope);
    assert.equal(late.begin('late', 'Write', { file_path: 'new.ts', content: 'x' }), undefined);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('nested Unicode and empty creation advances siblings only after successful exact own result', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-siblings-')));
  try {
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['새 폴더/nested/a.txt', '새 폴더/nested/빈 파일.txt']));
    const first = { file_path: '새 폴더/nested/a.txt', content: 'own content\n' };
    assert.deepEqual(policy.begin('a', 'Write', first), { kind: 'Create', path: first.file_path, directories: ['새 폴더', '새 폴더/nested'] });
    assert.equal(policy.approve('a', 'Write', first, true, false), true);
    assert.equal(policy.begin('overlap', 'Write', { file_path: '새 폴더/nested/빈 파일.txt', content: '' }), undefined);
    mkdirSync(join(root, '새 폴더/nested'), { recursive: true });
    writeFileSync(join(root, first.file_path), first.content);
    assert.equal(policy.finish('foreign', 'Write', first, true), false);
    assert.equal(policy.finish('a', 'Write', first, true, true), false);
    assert.equal(policy.finish('a', 'Write', first, true), true);
    assert.equal(policy.finish('a', 'Write', first, true), false);
    const second = { file_path: '새 폴더/nested/빈 파일.txt', content: '' };
    assert.deepEqual(policy.begin('b', 'Write', second), { kind: 'Create', path: second.file_path, directories: [] });
    assert.equal(policy.approve('b', 'Write', second, true, false), true);
    writeFileSync(join(root, second.file_path), '');
    assert.equal(policy.finish('b', 'Write', second, true), true);
    const edit = { file_path: first.file_path, old_string: 'own', new_string: 'updated' };
    assert.equal(policy.begin('c', 'Edit', edit)?.kind, 'Replace');
    assert.equal(policy.approve('c', 'Edit', edit, true, false), true);
    writeFileSync(join(root, first.file_path), 'updated content\n');
    assert.equal(policy.finish('c', 'Edit', edit, true), true);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('stale approval, substituted parents and failed results never confer sibling authority', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-parents-')));
  try {
    mkdirSync(join(root, 'existing'));
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['existing/a', 'missing/a', 'missing/b']));
    const existing = { file_path: 'existing/a', content: 'x' };
    assert.ok(policy.begin('stale', 'Write', existing));
    renameSync(join(root, 'existing'), join(root, 'old-existing'));
    mkdirSync(join(root, 'existing'));
    assert.equal(policy.approve('stale', 'Write', existing, true, false), false);
    const missing = { file_path: 'missing/a', content: 'x' };
    assert.ok(policy.begin('failed', 'Write', missing));
    assert.equal(policy.approve('failed', 'Write', missing, true, false), true);
    mkdirSync(join(root, 'missing'));
    assert.equal(policy.finish('failed', 'Write', missing, false), false);
    assert.equal(policy.begin('sibling', 'Write', { file_path: 'missing/b', content: '' }), undefined);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('failed writes and edits settle only when the selected baseline and parents remain unchanged', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-no-effects-')));
  try {
    writeFileSync(join(root, 'existing.txt'), 'unchanged working bytes');
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['missing/deeper/a', 'missing/deeper/b', 'existing.txt']));
    const create = { file_path: 'missing/deeper/a', content: 'approved' };
    assert.ok(policy.begin('create', 'Write', create));
    assert.equal(policy.approve('create', 'Write', create, true, false), true);
    assert.equal(policy.finish('create', 'Write', create, false), true);
    assert.deepEqual(policy.begin('sibling', 'Write', { file_path: 'missing/deeper/b', content: '' })?.directories,
      ['missing', 'missing/deeper']);
    assert.equal(policy.approve('sibling', 'Write', { file_path: 'missing/deeper/b', content: '' }, false, false), false);
    for (const [id, tool, input] of [
      ['replace', 'Write', { file_path: 'existing.txt', content: 'new bytes' }],
      ['edit', 'Edit', { file_path: 'existing.txt', old_string: 'unchanged', new_string: 'changed' }],
    ] as const) {
      assert.ok(policy.begin(id, tool, input));
      assert.equal(policy.approve(id, tool, input, true, false), true);
      assert.equal(policy.finish(id, tool, input, false), true);
    }
    assert.equal(readFileSync(join(root, 'existing.txt'), 'utf8'), 'unchanged working bytes');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('failed results reject partial parents, partial content, replaced ancestors and ambiguous leaf state', () => {
  for (const effect of ['parent', 'partial-create', 'partial-replace', 'same-content-replace', 'ancestor', 'ambiguous'] as const) {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-failed-effects-')));
    try {
      mkdirSync(join(root, 'existing'));
      writeFileSync(join(root, 'existing/a'), 'baseline');
      const path = effect === 'parent' || effect === 'partial-create' ? 'missing/deeper/a' : 'existing/a';
      const sibling = path.replace(/a$/, 'b');
      const policy = new SDKWritePolicy(root, reviewedScope(root, [path, sibling]));
      const input = { file_path: path, content: 'approved new bytes' };
      assert.ok(policy.begin(effect, 'Write', input));
      assert.equal(policy.approve(effect, 'Write', input, true, false), true);
      if (effect === 'parent') mkdirSync(join(root, 'missing'));
      else if (effect === 'partial-create') {
        mkdirSync(join(root, 'missing/deeper'), { recursive: true });
        writeFileSync(join(root, path), 'partial');
      } else if (effect === 'partial-replace') writeFileSync(join(root, path), 'partial');
      else if (effect === 'same-content-replace') {
        renameSync(join(root, path), join(root, 'original'));
        writeFileSync(join(root, path), 'baseline');
      } else if (effect === 'ancestor') {
        renameSync(join(root, 'existing'), join(root, 'original'));
        mkdirSync(join(root, 'existing'));
        renameSync(join(root, 'original/a'), join(root, path));
      } else {
        // A directory where a regular leaf was reviewed is not an observed baseline.
        rmSync(join(root, path));
        mkdirSync(join(root, path));
      }
      assert.equal(policy.finish(effect, 'Write', input, false), false, effect);
      if (effect === 'parent' || effect === 'partial-create' || effect === 'ancestor')
        assert.equal(policy.begin('sibling', 'Write', { file_path: sibling, content: '' }), undefined, effect);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test('the real bridge creates workspace proposals and preserves fatal failed-write effects', { timeout: 30_000 }, async () => {
  for (const scenario of ['success', 'nested', 'readonly', 'command-cancel', 'command-unknown', 'protected-root', 'none', 'parent', 'content', 'ambiguous'] as const) {
    const effect = scenario === 'nested' ? 'success' : scenario;
    const fixture = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-terminal-')));
    let child: ChildProcessWithoutNullStreams | undefined;
    try {
      const project = join(fixture, 'workspace');
      mkdirSync(project);
      mkdirSync(join(project, '.git'));
      writeFileSync(join(project, '.git/config'), 'protected Git configuration');
      const root = scenario === 'nested' ? join(project, 'nested-project') :
        scenario === 'protected-root' ? join(project, 'secrets', 'nested-project') : project;
      if (root !== project) mkdirSync(root, { recursive: true });
      mkdirSync(join(root, '.claude'));
      writeFileSync(join(root, '.claude/settings.json'), 'protected configuration');
      mkdirSync(join(root, 'excluded'));
      writeFileSync(join(root, 'excluded/context.txt'), 'excluded content');
      mkdirSync(join(root, 'host-state'));
      writeFileSync(join(root, 'host-state/state.txt'), 'protected host content');
      writeFileSync(join(root, 'sensitive.txt'), 'token = "test-sensitive-value"');
      writeFileSync(join(root, 'safe.txt'), 'visible source\n');
      if (process.platform !== 'win32') symlinkSync('safe.txt', join(root, 'alias.txt'));
      mkdirSync(join(root, 'existing'));
      writeFileSync(join(root, 'existing/a'), 'reviewed baseline');
      const path = effect === 'content' || effect === 'ambiguous' ? 'existing/a' : 'missing/deeper/a';
      const generatedLine = 'export const n = 1;\r\n';
      const generatedContent = generatedLine.repeat(Math.floor((256 * 1024) / Buffer.byteLength(generatedLine)));
      // Only SDK message production is synthetic. Execute the production main.ts
      // IPC, approval, filesystem policy and fatal path without loading a model,
      // reading real credentials or launching a native provider executable.
      const runtime = join(fixture, 'runtime');
      mkdirSync(runtime);
      const sdk = join(runtime, 'sdk.mjs');
      writeFileSync(join(runtime, 'package.json'), JSON.stringify({
        name: '@anthropic-ai/claude-agent-sdk', version: '0.3.999', claudeCodeVersion: '2.1.999',
      }));
      writeFileSync(join(runtime, process.platform === 'win32' ? 'claude.exe' : 'claude'), '');
      writeFileSync(sdk, `
        import { mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
        import { join } from 'node:path';
        import { setImmediate as nextTurn } from 'node:timers/promises';
        export { createSdkMcpServer, tool } from ${JSON.stringify(new URL('./node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs', import.meta.url).href)};
        import assert from 'node:assert/strict';
        export function query({ prompt, options }) {
          return {
            accountInfo: async () => ({ apiProvider: 'firstParty' }),
            initializationResult: async () => ({ claude_code_version: '2.1.999', hooks_applied: true }),
            supportedModels: async () => [],
            interrupt: async () => {},
            close() {},
            async *[Symbol.asyncIterator]() {
              try {
              const { value: submission } = await prompt.next();
              const session_id = '00000000-0000-4000-8000-000000000001';
              const input = { file_path: ${JSON.stringify(path)}, content: ${JSON.stringify(effect === 'success' ? generatedContent : 'approved complete bytes')} };
              yield { type: 'system', subtype: 'init', session_id, model: 'synthetic',
                claude_code_version: '2.1.999', permissionMode: 'default', tools: [...options.tools, ${JSON.stringify(DISCOVERY_TOOL)}, ...(options.mcpServers.eevee_command ? [${JSON.stringify(COMMAND_TOOL)}] : [])],
                mcp_servers: [{ name: 'eevee_source', status: 'connected' }, ...(options.mcpServers.eevee_command ? [{ name: 'eevee_command', status: 'connected' }] : [])], plugins: [], skills: [], apiKeySource: 'ANTHROPIC_API_KEY' };
              const hook = options.hooks.PreToolUse[0].hooks[0];
              const observe = async (name, input, id) => {
                const checked = await hook({ hook_event_name: 'PreToolUse', tool_use_id: id, tool_name: name, tool_input: input });
                if (checked.hookSpecificOutput.permissionDecision === 'deny') return false;
                return (await options.canUseTool(name, input, {
                  toolUseID: id, requestId: id, signal: options.abortController.signal,
                })).behavior === 'allow';
              };
              // Use the real installed SDK MCP server and wire protocol, not a
              // fake custom-tool callback or a source-string wiring assertion.
              const server = options.mcpServers.eevee_source.instance;
              let nextId = 0;
              const responses = new Map();
              const transport = {
                async start() {}, async close() {},
                async send(message) { responses.get(message.id)?.(message); },
              };
              await server.connect(transport);
              const rpc = (method, params, target = transport) => {
                const { promise, resolve } = Promise.withResolvers();
                const id = ++nextId;
                responses.set(id, resolve);
                target.onmessage({ jsonrpc: '2.0', id, method, params });
                return promise;
              };
              const initialized = await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'fixture', version: '1' } });
              assert.ok(initialized.result);
              transport.onmessage({ jsonrpc: '2.0', method: 'notifications/initialized' });
              const listed = await rpc('tools/list', {});
              assert.deepEqual(listed.result.tools.map(tool => tool.name), ['discover']);
              for (const mode of ['glob', 'grep']) {
                const args = { mode, path: '.', glob: '**/*', ...(mode === 'grep' ? { text: 'visible' } : {}) };
                assert.equal(await observe(${JSON.stringify(DISCOVERY_TOOL)}, args, mode), true);
                const response = await rpc('tools/call', { name: 'discover', arguments: args });
                assert.equal(response.result.isError, undefined);
                const result = JSON.parse(response.result.content[0].text);
                if (mode === 'glob') assert.deepEqual(result.paths, ['existing/a', 'safe.txt']);
                else assert.deepEqual(result.matches, [{ path: 'safe.txt', line: 1, text: 'visible source' }]);
              }
              assert.equal(await observe('Glob', { pattern: '**/*', path: '.' }, 'native-root'), false);
              assert.equal(await observe('Grep', { pattern: 'visible', path: '.' }, 'native-root-grep'), false);
              assert.equal(await observe('Read', { file_path: 'safe.txt' }, 'read-safe'), true);
              assert.equal(readFileSync(join(options.cwd, 'safe.txt'), 'utf8'), 'visible source\\n');
              for (const file_path of ['.git/config', '.claude/settings.json', 'excluded/context.txt', 'host-state/state.txt', 'sensitive.txt', '../outside', 'alias.txt']) {
                assert.equal(await observe('Read', { file_path }, 'unsafe-' + file_path), false);
              }
              assert.equal(await observe('mcp__foreign__read', {}, 'foreign-server'), false);
              await server.close();
              if (!options.tools.includes('Write')) {
                assert.equal(options.mcpServers.eevee_command, undefined);
                assert.equal(await observe(${JSON.stringify(COMMAND_TOOL)}, { cwd: '', command: { Direct: { program: 'rustc', args: [] } } }, 'readonly-command'), false);
              } else if (['success', 'command-cancel', 'command-unknown'].includes(${JSON.stringify(effect)})) {
                assert.equal(options.mcpServers.eevee_command.timeout, 86400000);
                // Each MCP server owns a distinct transport. Reusing discovery's
                // transport chains its old handler and can race a false
                // "tool not found" response against the command's actual result.
                const commandTransport = { async start() {}, async close() {},
                  async send(message) { responses.get(message.id)?.(message); } };
                await options.mcpServers.eevee_command.instance.connect(commandTransport);
                assert.ok((await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'fixture', version: '1' } }, commandTransport)).result);
                commandTransport.onmessage({ jsonrpc: '2.0', method: 'notifications/initialized' });
                const args = { cwd: '', command: { Direct: { program: 'rustc', args: ['main.rs'] } } };
                yield { type: 'assistant', session_id, user_message_uuid: submission.uuid,
                  message: { id: 'host-command', content: [{ type: 'tool_use', id: 'host-command', name: ${JSON.stringify(COMMAND_TOOL)}, input: args }] } };
                assert.equal(await observe(${JSON.stringify(COMMAND_TOOL)}, args, 'host-command-routing'), true);
                const waiting = rpc('tools/call', { name: 'run_command', arguments: args }, commandTransport);
                await nextTurn(); // Drain MCP request dispatch, not a guessed elapsed-time delay.
                assert.equal(await observe('Write', input, 'command-overlap-write'), false);
                assert.equal(await observe(${JSON.stringify(COMMAND_TOOL)}, args, 'command-overlap-command'), false);
                const concurrent = await rpc('tools/call', { name: 'run_command', arguments: args }, commandTransport);
                assert.equal(concurrent.result.isError, true);
                yield { type: 'assistant', session_id, message: { id: 'command-exclusion-checked', content: [{ type: 'text', text: 'command-exclusion-checked' }] } };
                const response = await waiting;
                assert.equal(response.result.isError, true);
                assert.match(response.result.content[0].text, /^\\{/, JSON.stringify(response));
                assert.equal(JSON.parse(response.result.content[0].text).stderr, 'actual host compiler stderr');
                yield { type: 'user', session_id, message: { content: [
                  { type: 'tool_result', tool_use_id: 'host-command', content: response.result.content, is_error: response.result.isError },
                ] } };
                await options.mcpServers.eevee_command.instance.close();
                if (['command-cancel', 'command-unknown'].includes(${JSON.stringify(effect)})) {
                  assert.equal(await observe('Write', input, 'cancelled-write'), false);
                  assert.equal(await observe(${JSON.stringify(COMMAND_TOOL)}, args, 'unknown-next-command'), false);
                  yield { type: 'result', session_id, is_error: true, subtype: 'error_during_execution' };
                  return;
                }
              }
              if (!options.tools.includes('Write')) {
                assert.equal(await observe('Write', input, 'readonly-write'), false);
                yield { type: 'assistant', session_id, user_message_uuid: submission.uuid,
                  message: { id: 'answer', content: [{ type: 'text', text: 'Observed visible source' }] } };
                yield { type: 'result', session_id, is_error: false, subtype: 'success' };
                return;
              }
              for (const [id, file_path, callbackOnly] of [
                ['protected-refusal', '.env', false],
                ['outside-refusal', '../outside-refused.txt', false],
                ['callback-refusal', '.claude/refused.txt', true],
              ]) {
                const refused = { file_path, content: 'must never be written' };
                yield { type: 'assistant', session_id, user_message_uuid: submission.uuid,
                  message: { id, content: [{ type: 'tool_use', id, name: 'Write', input: refused }] } };
                if (callbackOnly) {
                  assert.equal((await options.canUseTool('Write', refused, {
                    toolUseID: id, requestId: id, signal: options.abortController.signal,
                  })).behavior, 'deny');
                } else assert.equal(await observe('Write', refused, id), false);
                yield { type: 'user', session_id, message: { content: [
                  { type: 'tool_result', tool_use_id: id, is_error: true, content: 'Host refused before execution' },
                ] } };
                assert.throws(() => readFileSync(join(options.cwd, file_path)), { code: 'ENOENT' });
              }
              yield { type: 'stream_event', session_id, user_message_uuid: submission.uuid,
                event: { type: 'message_start', message: { id: 'call' } } };
              yield { type: 'stream_event', session_id,
                event: { type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'write', name: 'Write', input: {} } } };
              const argumentsJSON = JSON.stringify(input);
              for (let offset = 0; offset < argumentsJSON.length; offset += 4096) {
                yield { type: 'stream_event', session_id,
                  event: { type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: argumentsJSON.slice(offset, offset + 4096) } } };
              }
              yield { type: 'stream_event', session_id, event: { type: 'content_block_stop', index: 0 } };
              yield { type: 'assistant', session_id, user_message_uuid: submission.uuid,
                message: { id: 'call', content: [{ type: 'tool_use', id: 'write', name: 'Write', input }] } };
              // Exact Write input and one-shot consent still gate native effects.
              const hookInput = { hook_event_name: 'PreToolUse', tool_use_id: 'write', tool_name: 'Write', tool_input: input };
              if ((await hook(hookInput)).hookSpecificOutput.permissionDecision !== 'ask') throw new Error('Mandatory proposal hook refused');
              const permission = await options.canUseTool('Write', input, {
                toolUseID: 'write', requestId: 'approval', signal: options.abortController.signal,
              });
              if (permission.behavior !== 'allow') throw new Error('Synthetic write was not approved');
              if ((await hook({ ...hookInput, tool_input: { ...input, content: 'altered' } })).hookSpecificOutput.permissionDecision !== 'deny') throw new Error('Altered hook input accepted');
              if ((await hook(hookInput)).hookSpecificOutput.permissionDecision !== 'ask') throw new Error('Approved authority stale');
              if (${JSON.stringify(effect)} === 'success') {
                mkdirSync(join(options.cwd, 'missing/deeper'), { recursive: true });
                writeFileSync(join(options.cwd, input.file_path), input.content);
              }
              if (${JSON.stringify(effect)} === 'parent') mkdirSync(join(options.cwd, 'missing'));
              if (${JSON.stringify(effect)} === 'content') writeFileSync(join(options.cwd, input.file_path), 'partial');
              if (${JSON.stringify(effect)} === 'ambiguous') {
                rmSync(join(options.cwd, input.file_path));
                mkdirSync(join(options.cwd, input.file_path));
              }
              yield { type: 'user', session_id, message: { content: [
                { type: 'tool_result', tool_use_id: 'write', is_error: ${effect !== 'success'}, content: 'Synthetic operation result' },
              ] } };
              if (${JSON.stringify(effect)} === 'success') {
                const edit = { file_path: input.file_path, old_string: 'export const n = 1;', new_string: 'export const n = 2;', replace_all: true };
                yield { type: 'assistant', session_id, message: { id: 'editing', content: [{ type: 'tool_use', id: 'edit', name: 'Edit', input: edit }] } };
                assert.equal(await observe('Edit', edit, 'edit'), true);
                writeFileSync(join(options.cwd, edit.file_path), readFileSync(join(options.cwd, edit.file_path), 'utf8').split(edit.old_string).join(edit.new_string));
                yield { type: 'user', session_id, message: { content: [{ type: 'tool_result', tool_use_id: 'edit', content: 'Actual fixture edit completed' }] } };
                const refused = { file_path: 'denied.txt', content: 'must not exist' };
                yield { type: 'assistant', session_id, message: { id: 'denial', content: [{ type: 'tool_use', id: 'denied', name: 'Write', input: refused }] } };
                assert.equal(await observe('Write', refused, 'denied'), false);
                yield { type: 'user', session_id, message: { content: [{ type: 'tool_result', tool_use_id: 'denied', is_error: true, content: 'User denied' }] } };
              }
              yield { type: 'assistant', session_id,
                message: { id: 'later', content: [{ type: 'text', text: 'Later model success must not clear uncertainty' }] } };
              yield { type: 'result', session_id, is_error: false, subtype: 'success' };
              } catch (error) { console.error(error); throw error; }
            },
          };
        }
      `);
      const bootstrap = `
        import { registerHooks } from 'node:module';
        registerHooks({ resolve(specifier, context, nextResolve) {
          if (specifier === '@anthropic-ai/claude-agent-sdk')
            return { url: ${JSON.stringify(pathToFileURL(sdk).href)}, shortCircuit: true };
          if (specifier.startsWith('@anthropic-ai/claude-agent-sdk-') && specifier.endsWith('/package.json'))
            return { url: ${JSON.stringify(pathToFileURL(join(runtime, 'package.json')).href)}, shortCircuit: true };
          return nextResolve(specifier, context);
        } });
      `;
      child = spawn(process.execPath, ['--import', `data:text/javascript,${encodeURIComponent(bootstrap)}`,
        fileURLToPath(new URL('./main.ts', import.meta.url))], {
        env: { ...process.env, HOME: fixture, USERPROFILE: fixture, XDG_CONFIG_HOME: fixture, APPDATA: fixture,
          ANTHROPIC_API_KEY: 'deterministic-fixture-not-a-real-key', ANTHROPIC_PROFILE: '', ANTHROPIC_CONFIG_DIR: '',
          CLAUDE_CODE_USE_BEDROCK: '', CLAUDE_CODE_USE_VERTEX: '', EEVEE_CLAUDE_CLOUD_AUTH: '', NODE_OPTIONS: '' },
        stdio: ['pipe', 'pipe', 'pipe'],
        signal: AbortSignal.timeout(10_000),
      });
      const frames: Record<string, unknown>[] = [];
      let output = '';
      let diagnostics = '';
      child.stderr!.setEncoding('utf8').on('data', chunk => { diagnostics += chunk; });
      child.stdout!.setEncoding('utf8').on('data', chunk => {
        output += chunk;
        for (let end; (end = output.indexOf('\n')) !== -1;) {
          const frame = JSON.parse(output.slice(0, end));
          output = output.slice(end + 1);
          frames.push(frame);
          if (frame.type === 'file_preview' && frame.stage === 'delta') {
            if (effect === 'content' || effect === 'ambiguous') assert.equal(readFileSync(join(root, path), 'utf8'), 'reviewed baseline');
            else assert.throws(() => lstatSync(join(root, path)), { code: 'ENOENT' });
          }
          const command = frames.find(value => value.type === 'bridge_command');
          if (frame.type === 'item' && frame.text === 'command-exclusion-checked' && command) {
            if (scenario === 'command-cancel') child!.stdin!.write(JSON.stringify({ protocol: 1, type: 'cancel', generation: 1 }) + '\n');
            child!.stdin!.write(JSON.stringify({
            protocol: 1, type: 'bridge_command_result', generation: 1, task_id: 1, session_id: command.session_id, call_id: command.call_id,
            result: { status: 'error', run: { exit_code: 1, ...(scenario === 'command-unknown' ? { state: 'OutcomeUnknown', tree: 'OwnershipUnknown' } : {}) }, stdout: '', stderr: 'actual host compiler stderr', artifact: { id: 'fixture', revision: 1 } },
          }) + '\n');
          }
          if (frame.type === 'prompt') child!.stdin!.write(JSON.stringify({
            protocol: 1, type: 'answer', generation: 1, request_id: frame.request_id,
            answer: frame.request_id === 'denied' ? frame.input.options[0].answer :
              frame.input.options.find((option: { answer: { behavior: string } }) => option.answer.behavior === 'allow').answer,
          }) + '\n');
        }
      });
      const closed = once(child, 'close');
      child.stdin!.write(JSON.stringify({
        protocol: 1, bridge_revision: 11, type: 'run', generation: 1, task_id: 1, request_id: 'submission',
        prompt: 'Deterministic source and write operations', cwd: root, state_dir: join(fixture, 'state'),
        operation: scenario === 'readonly' ? 'plan' : 'implement',
        write_scope: { ...(scenario === 'readonly' ? reviewedScope(root, []) : workspaceScope(root)), excluded_paths: ['excluded'], protected_paths: [join(root, 'host-state')] }, inspect_models: false,
        selection: { model: null, effort: null },
      }) + '\n');
      const [code] = await closed;
      if (scenario === 'protected-root') {
        assert.equal(code, 1, diagnostics);
        assert.ok(frames.some(frame => frame.type === 'fatal'));
        assert.equal(frames.some(frame => ['dispatched', 'prompt', 'file_preview', 'thread'].includes(String(frame.type))), false);
        assert.throws(() => lstatSync(join(root, path)), { code: 'ENOENT' });
        continue;
      }
      assert.ok(frames.some(frame => frame.type === 'dispatched'), `${effect}: ${diagnostics}; terminal frames: ${JSON.stringify(frames.filter(frame => ['fatal', 'status', 'thread', 'bridge_command'].includes(String(frame.type))))}`);
      if (scenario === 'readonly') {
        assert.equal(code, 0, diagnostics);
        assert.equal(frames.find(frame => frame.type === 'finished')?.outcome, 'completed');
        assert.equal(frames.some(frame => frame.type === 'file_preview'), false);
        assert.throws(() => lstatSync(join(root, path)), { code: 'ENOENT' });
        continue;
      }
      if (scenario === 'command-cancel' || scenario === 'command-unknown') {
        assert.equal(code, 0, diagnostics);
        if (scenario === 'command-cancel') assert.ok(frames.some(frame => frame.type === 'bridge_command_cancel'));
        assert.equal(frames.find(frame => frame.type === 'finished')?.outcome, 'unknown');
        assert.equal(frames.some(frame => frame.type === 'prompt' && frame.item_id === 'cancelled-write'), false);
        assert.throws(() => lstatSync(join(root, path)), { code: 'ENOENT' });
        continue;
      }
      if (effect === 'success') assert.equal(code, 0, diagnostics);
      const previews = frames.filter(frame => frame.type === 'file_preview' && frame.tool_id === 'write');
      assert.ok(previews.some(frame => frame.stage === 'delta'));
      assert.deepEqual(JSON.parse(previews.filter(frame => frame.stage === 'delta').map(frame => frame.fragment).join('')), {
        file_path: path, content: effect === 'success' ? generatedContent : 'approved complete bytes',
      });
      assert.ok(previews.every(frame => frame.tool_id === 'write' && frame.session_id === '00000000-0000-4000-8000-000000000001'));
      assert.ok(frames.findIndex(frame => frame.type === 'file_preview' && frame.stage === 'stop') < frames.findIndex(frame => frame.type === 'prompt' && frame.item_id === 'write'));
      assert.ok(previews.some(frame => frame.stage === 'review'));
      assert.ok(previews.some(frame => frame.stage === 'approved'));
      assert.equal(previews.some(frame => frame.stage === 'applied'), false);
      assert.equal(previews.some(frame => frame.stage === 'reported'), effect === 'success');
      assert.equal(previews.some(frame => frame.stage === 'reported_failed'), effect === 'none');
      assert.equal(previews.some(frame => frame.stage === 'unknown'), !['success', 'none'].includes(effect));
      if (effect === 'none' || effect === 'success') {
        assert.equal(code, 0, diagnostics);
        assert.equal(frames.some(frame => frame.type === 'fatal'), false);
        assert.equal(frames.find(frame => frame.type === 'finished')?.outcome, 'completed');
        if (effect === 'none') assert.ok(frames.some(frame => frame.type === 'item' && typeof frame.text === 'string' && frame.text.includes('"status":"failed"')));
        else {
          assert.equal(readFileSync(join(root, path), 'utf8'), generatedContent.replaceAll('export const n = 1;', 'export const n = 2;'));
          assert.throws(() => lstatSync(join(root, 'denied.txt')), { code: 'ENOENT' });
          assert.ok(frames.some(frame => frame.type === 'file_preview' && frame.tool_id === 'edit' && frame.stage === 'reported'));
          assert.ok(frames.some(frame => frame.type === 'file_preview' && frame.tool_id === 'denied' && frame.stage === 'denied'));
          const prompt = frames.find(frame => frame.type === 'prompt' && frame.item_id === 'write') as { input: { input: { content: string } } } | undefined;
          assert.equal(prompt?.input.input.content, generatedContent);
          for (const id of ['protected-refusal', 'outside-refusal', 'callback-refusal']) {
            assert.ok(frames.some(frame => frame.type === 'file_preview' && frame.tool_id === id && frame.stage === 'refused'));
            assert.equal(frames.some(frame => frame.type === 'prompt' && frame.item_id === id), false);
            assert.ok(frames.some(frame => frame.type === 'item' && frame.item_id === id && frame.complete === true));
          }
          assert.ok(Buffer.byteLength(JSON.stringify(frames.filter(frame => frame.type === 'item'))) < 16 * 1024);
        }
      } else {
        assert.equal(code, 1, diagnostics);
        assert.match(String(frames.find(frame => frame.type === 'fatal')?.message), /outcome unknown/);
        assert.equal(frames.some(frame => frame.type === 'finished'), false);
        assert.equal(frames.some(frame => frame.item_id === 'later:0'), false);
      }
    } finally {
      child?.kill();
      rmSync(fixture, { recursive: true, force: true });
    }
  }
});

test('one-shot write consent rejects foreign, duplicate, altered and cancelled grants', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-consent-')));
  try {
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['new.txt']));
    const input = { file_path: 'new.txt', content: 'reviewed' };
    assert.equal(policy.commandAvailable(), true);
    assert.ok(policy.begin('one', 'Write', input));
    assert.equal(policy.commandAvailable(), false);
    assert.equal(policy.approve('foreign', 'Write', input, true, false), false);
    assert.equal(policy.approve('one', 'Write', { ...input, content: 'changed' }, true, false), false);
    assert.equal(policy.begin('one', 'Write', input), undefined);
    assert.ok(policy.begin('cancel', 'Write', input));
    assert.equal(policy.approve('cancel', 'Write', input, true, true), false);
    assert.equal(policy.finish('cancel', 'Write', input, true), false);
    assert.equal(policy.finish('cancel', 'Write', input, false), true);
    assert.ok(policy.begin('allow', 'Write', input));
    assert.equal(policy.approve('allow', 'Write', input, true, false), true);
    assert.equal(policy.commandAvailable(), false);
    assert.equal(policy.approve('allow', 'Write', input, true, false), false);
    assert.equal(policy.begin('overlap', 'Write', input), undefined);
    assert.equal(policy.preToolUse('allow', 'Write', { ...input, content: 'changed' }), false);
    writeFileSync(join(root, 'new.txt'), 'externally appeared');
    assert.equal(policy.preToolUse('allow', 'Write', input), false);
    assert.equal(policy.finish('allow', 'Write', input, true), false);
    assert.equal(policy.commandAvailable(), false, 'uncertain source effects cannot race a build');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('existing dirty replacement and mandatory post-approval stale checks preserve exact bytes', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-replace-')));
  try {
    writeFileSync(join(root, 'dirty.txt'), 'user dirty bytes');
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['dirty.txt']));
    const input = { file_path: join(root, 'dirty.txt'), content: '' };
    assert.equal(policy.begin('replace', 'Write', input)?.kind, 'Replace');
    assert.equal(policy.approve('replace', 'Write', input, true, false), true);
    assert.equal(policy.preToolUse('replace', 'Write', input), true);
    writeFileSync(join(root, 'dirty.txt'), '');
    assert.equal(policy.finish('replace', 'Write', input, true), true);
    assert.equal(policy.preToolUse('replace', 'Write', input), false);
    const edit = { file_path: 'dirty.txt', content: 'later' };
    assert.equal(policy.begin('stale', 'Write', edit)?.kind, 'Replace');
    assert.equal(policy.approve('stale', 'Write', edit, true, false), true);
    writeFileSync(join(root, 'dirty.txt'), 'external edit');
    assert.equal(policy.preToolUse('stale', 'Write', edit), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('successful-looking mismatched bytes and symlink parents never seed sibling authority', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-result-')));
  try {
    const policy = new SDKWritePolicy(root, reviewedScope(root, ['nested/a', 'nested/b']));
    const input = { file_path: 'nested/a', content: 'approved' };
    assert.ok(policy.begin('a', 'Write', input));
    assert.equal(policy.approve('a', 'Write', input, true, false), true);
    mkdirSync(join(root, 'nested'));
    writeFileSync(join(root, 'nested/a'), 'not approved');
    assert.equal(policy.finish('a', 'Write', input, true), false);
    assert.equal(policy.begin('b', 'Write', { file_path: 'nested/b', content: '' }), undefined);
    const missing = new SDKWritePolicy(root, reviewedScope(root, ['link/a']));
    symlinkSync(join(root, 'nested'), join(root, 'link'), process.platform === 'win32' ? 'junction' : 'dir');
    assert.equal(missing.preToolUse('link', 'Write', { file_path: 'link/a', content: '' }), false);
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
  const command = { protocol: 1, bridge_revision: 11, type: 'run', generation: 1, task_id: 1, request_id: null, cwd: '/project', state_dir: '/state', prompt: '',
    operation: 'plan', write_scope: { mode: 'Selected', protected_paths: [], excluded_paths: [], root_identity: null, files: [] }, resume: null, inspect_models: true, selection: { model: null, effort: null } };
  assert.equal(RunSchema.safeParse(command).success, true);
  assert.equal(RunSchema.safeParse({ ...command, task_id: 0 }).success, true);
  assert.equal(RunSchema.safeParse({ ...command, task_id: -1 }).success, false);
  for (const changed of [{ request_id: 'claimed-inference' }, { prompt: 'must not send' }, { resume: '01234567-89ab-4cde-8123-456789abcdef' },
    { bridge_revision: 10 }, { edit_paths: ['file.ts'] }, { operation: 'implement' }, { tools: ['Bash'] }, { selection: { model: null, effort: null, fallback: 'a' } }]) {
    assert.equal(RunSchema.safeParse({ ...command, ...changed }).success, false);
  }
  for (const field of ['mode', 'protected_paths', 'excluded_paths']) {
    const scope: Record<string, unknown> = { ...command.write_scope };
    delete scope[field];
    assert.equal(RunSchema.safeParse({ ...command, write_scope: scope }).success, false);
  }
  const workspace = { ...command, inspect_models: false, operation: 'implement',
    write_scope: { ...command.write_scope, mode: 'Workspace', root_identity: '1:2' } };
  assert.equal(RunSchema.safeParse(workspace).success, true);
  assert.equal(RunSchema.safeParse({ ...workspace, task_id: 0 }).success, false);
  assert.equal(RunSchema.safeParse({ ...workspace, write_scope: { ...workspace.write_scope, mode: 'Selected' } }).success, false);
  assert.equal(RunSchema.safeParse({ ...command, write_scope: { ...command.write_scope, mode: 'Workspace' } }).success, false);
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

test('observational SDK tools preserve project/privacy boundaries without enrolling writes', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-source-policy-')));
  const outside = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-source-outside-')));
  try {
    mkdirSync(join(root, 'src'));
    mkdirSync(join(root, 'private'));
    mkdirSync(join(root, 'excluded'));
    writeFileSync(join(root, 'src', 'safe.ts'), '// dirty\r\nexport const value = 1;\r\n');
    writeFileSync(join(root, 'private', 'state.txt'), 'private');
    writeFileSync(join(root, 'excluded', 'context.txt'), 'excluded');
    writeFileSync(join(root, '.env'), 'credential');
    writeFileSync(join(root, 'auth.json'), '{}');
    writeFileSync(join(root, 'token.txt'), 'password=private-value');
    writeFileSync(join(root, 'large.txt'), 'x'.repeat(256 * 1024 + 1));
    writeFileSync(join(outside, 'file.txt'), 'outside');
    const scope = reviewedScope(root, []);
    scope.protected_paths.push(join(root, 'private'));
    scope.excluded_paths.push('excluded');
    const policy = new SDKWritePolicy(root, scope);
    assert.equal(policy.readAdmission('Read', { file_path: join(root, 'src', 'safe.ts') }).allowed, true);
    assert.equal(policy.readAdmission('Grep', { path: 'src/safe.ts', pattern: 'value' }).allowed, true);
    assert.equal(policy.readAdmission('Glob', { path: 'src', pattern: '**/*.ts' }).allowed, true);
    for (const path of ['.env', 'auth.json', 'private/state.txt', 'excluded/context.txt',
      'token.txt', 'large.txt', '../outside.txt', join(outside, 'file.txt')]) {
      assert.equal(policy.readAdmission('Read', { file_path: path }).allowed, false, path);
    }
    assert.equal(policy.preToolUse('no-authority-from-read', 'Write', { file_path: 'src/safe.ts', content: 'changed' }), false);
    assert.equal(readFileSync(join(root, 'src', 'safe.ts'), 'utf8'), '// dirty\r\nexport const value = 1;\r\n');
  } finally { rmSync(root, { recursive: true, force: true }); rmSync(outside, { recursive: true, force: true }); }
});

test('unsafe recursive SDK discovery returns actionable actual safe filenames rather than empty success', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-discovery-policy-')));
  try {
    mkdirSync(join(root, '.git'));
    mkdirSync(join(root, 'src'));
    writeFileSync(join(root, '.git', 'config'), 'private repository configuration');
    writeFileSync(join(root, 'src', 'visible.ts'), 'export const visible = true;\n');
    const policy = new SDKWritePolicy(root, reviewedScope(root, []));
    const result = policy.readAdmission('Glob', { pattern: '**/*.ts' });
    assert.equal(result.allowed, false);
    assert.match(result.reason, /src\/visible\.ts/);
    assert.doesNotMatch(result.reason, /\.git\/config/);
    assert.equal(policy.readAdmission('Glob', { path: 'src', pattern: '*.ts' }).allowed, true);
    assert.equal(policy.readAdmission('Grep', { path: 'src', pattern: 'visible' }).allowed, true);
    for (const pattern of ['../*', '{src,/outside}/*', '[.][.]/*', '/outside/*']) {
      assert.equal(policy.readAdmission('Glob', { path: root, pattern }).allowed, false);
    }
    if (process.platform !== 'win32') {
      symlinkSync('src', join(root, 'alias'));
      assert.equal(policy.readAdmission('Read', { file_path: 'alias/visible.ts' }).allowed, false);
      linkSync(join(root, 'src', 'visible.ts'), join(root, 'hard.ts'));
      assert.equal(policy.readAdmission('Read', { file_path: 'hard.ts' }).allowed, false);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('noninspection SDK planning requires frozen observational root even without mutation targets', () => {
  const command = { protocol: 1, bridge_revision: 11, type: 'run', generation: 1, task_id: 1,
    request_id: 'owned-request', cwd: '/project', state_dir: '/state', prompt: 'Read project context',
    operation: 'plan', write_scope: { mode: 'Selected', protected_paths: ['/state'],
      excluded_paths: ['private'], root_identity: '1:2', files: [] },
    resume: null, inspect_models: false, selection: { model: null, effort: null } };
  assert.equal(RunSchema.safeParse(command).success, true);
  assert.equal(RunSchema.safeParse({ ...command, write_scope: { ...command.write_scope, root_identity: null } }).success, false);
  assert.equal(RunSchema.safeParse({ ...command, bridge_revision: 10 }).success, false);
});

test('file previews forward only actual correlated tool argument fragments and never grant write authority', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-preview-')));
  try {
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    const previews = new SDKFilePreviews(true);
    const start = { type: 'content_block_start', index: 2, content_block: { type: 'tool_use', id: 'write', name: 'Write', input: {} } };
    const delta = (partial_json: string, index = 2) => ({ type: 'content_block_delta', index, delta: { type: 'input_json_delta', partial_json } });
    assert.deepEqual(previews.stream(start), [], 'a block without message correlation is not a preview');
    previews.stream({ type: 'message_start', message: { id: 'message' } });
    assert.equal(previews.stream(start)[0]?.stage, 'start');
    assert.deepEqual(previews.stream(start), [], 'duplicate start cannot rewind fragments');
    assert.deepEqual(previews.stream(delta('wrong call', 1)), []);
    const fragments = ['{"file_path":"new.rs","content":"first\\n', 'second\\n"}'];
    assert.deepEqual(fragments.flatMap(fragment => previews.stream(delta(fragment))).map(frame => frame.fragment), fragments);
    assert.equal(previews.stream({ type: 'content_block_stop', index: 2 })[0]?.stage, 'stop');
    assert.deepEqual(previews.stream(delta('late')), []);
    assert.equal(policy.approve('write', 'Write', { file_path: 'new.rs', content: 'first\nsecond\n' }, true, false), false);
    assert.throws(() => lstatSync(join(root, 'new.rs')), { code: 'ENOENT' });
    const input = { file_path: 'new.rs', content: 'first\nsecond\n', opaque: 'never forward complete opaque fields' };
    const received = previews.complete('write', 'Write', input);
    assert.deepEqual(JSON.parse(received[0].fragment!), { file_path: input.file_path, content: input.content });
    assert.equal(previews.phase('write', 'review')[0]?.stage, 'review');
    assert.deepEqual(previews.complete('write', 'Write', input), [], 'assistant replay cannot regress active review');
    assert.equal(previews.phase('write', 'approved')[0]?.stage, 'approved');
    assert.equal(previews.phase('write', 'reported')[0]?.stage, 'reported');
    assert.deepEqual(previews.phase('write', 'review'), []);
    assert.equal(policy.approve('write', 'Write', input, true, false), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('preview guards, nonwrite blocks, bounds and denial cannot invent progress or success', () => {
  const disabled = new SDKFilePreviews(false);
  assert.deepEqual(disabled.complete('write', 'Write', { file_path: 'a', content: 'x' }), []);
  assert.deepEqual(disabled.phase('write', 'review'), []);
  const previews = new SDKFilePreviews(true);
  previews.stream({ type: 'message_start', message: { id: 'message' } });
  for (const [id, name] of [['reasoning', 'thinking'], ['read', 'Read'], ['shell', 'Bash']]) {
    assert.deepEqual(previews.stream({ type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id, name } }), []);
  }
  assert.deepEqual(previews.stream({ type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'private' } }), []);
  assert.deepEqual(previews.complete('shell', 'Bash', { command: 'private' }), []);
  assert.equal(previews.complete('edit', 'Edit', { file_path: 'a', old_string: 'do not display', new_string: 'replacement' })[0]?.stage, 'received');
  assert.equal(previews.phase('edit', 'denied')[0]?.stage, 'denied');
  assert.deepEqual(previews.phase('edit', 'reported'), []);
  previews.stream({ type: 'content_block_start', index: 1, content_block: { type: 'tool_use', id: 'large', name: 'Write' } });
  const overflow = previews.stream({ type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: 'x'.repeat(512 * 1024 + 1) } });
  assert.equal(overflow[0]?.stage, 'interrupted');
  assert.deepEqual(previews.complete('large', 'Write', { file_path: 'a', content: 'later' }), []);
});

test('host discovery bounds results while omitting inaccessible source and retaining native subtree guards', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-discovery-')));
  try {
    mkdirSync(join(root, '.git'));
    writeFileSync(join(root, '.git/config'), 'repository metadata');
    mkdirSync(join(root, 'src'));
    writeFileSync(join(root, 'src/safe.ts'), 'const visible = 1;\n');
    writeFileSync(join(root, 'src/sensitive.ts'), 'token = "not-for-source-context"');
    if (process.platform !== 'win32') {
      symlinkSync('safe.ts', join(root, 'src/link.ts'));
      writeFileSync(join(root, 'original.ts'), 'linked source');
      linkSync(join(root, 'original.ts'), join(root, 'hard.ts'));
    }
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    assert.deepEqual(policy.discover({ mode: 'glob' }).paths, ['src/safe.ts']);
    assert.deepEqual(policy.discover({ mode: 'grep', text: 'visible' }).matches,
      [{ path: 'src/safe.ts', line: 1, text: 'const visible = 1;' }]);
    assert.deepEqual(policy.discover({ mode: 'glob', path: 'src', glob: '*.ts' }).paths, ['src/safe.ts']);
    assert.equal(policy.readAdmission('Glob', { path: '.', pattern: '**/*' }).allowed, false);
    assert.equal(policy.begin('secret-edit', 'Edit', { file_path: 'src/sensitive.ts', old_string: 'token', new_string: 'changed' }), undefined);
    for (const input of [{ mode: 'glob', path: '..' }, { mode: 'glob', path: '.git' },
      { mode: 'glob', glob: '../*' }, { mode: 'glob', glob: '{src,/outside}/*' }, { mode: 'grep' }]) {
      assert.throws(() => policy.discover(input));
    }
    for (let index = 0; index < 130; index++) writeFileSync(join(root, `source-${index}.ts`), 'visible\n');
    const bounded = policy.discover({ mode: 'glob' });
    assert.equal(bounded.paths.length, 128);
    assert.equal(bounded.truncated, true);
    const matches = policy.discover({ mode: 'grep', text: 'visible' });
    assert.equal(matches.matches.length, 128);
    assert.equal(matches.truncated, true);
    assert.ok(Buffer.byteLength(JSON.stringify(matches)) < 64 * 1024);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('session policy admits only the exact connected host discovery server', () => {
  const message = { claude_code_version: compatibility.cliVersion, permissionMode: 'default',
    tools: ['Read', DISCOVERY_TOOL], mcp_servers: [{ name: DISCOVERY_SERVER, status: 'connected' }], plugins: [], skills: [] };
  assert.doesNotThrow(() => assertSessionPolicy(message, compatibility.cliVersion, ['Read', DISCOVERY_TOOL]));
  for (const servers of [[], [{ name: DISCOVERY_SERVER, status: 'failed' }],
    [{ name: 'external', status: 'connected' }], [...message.mcp_servers, ...message.mcp_servers]]) {
    assert.throws(() => assertSessionPolicy({ ...message, mcp_servers: servers }, compatibility.cliVersion, ['Read', DISCOVERY_TOOL]));
  }
  assert.throws(() => assertSessionPolicy({ ...message, tools: ['Read', 'mcp__external__discover'] }, compatibility.cliVersion, ['Read', DISCOVERY_TOOL]));
});

test('pre-execution refusals settle only exact nonreplayed errors and never replace active authority', () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-refusal-')));
  try {
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    const input = { file_path: '.env', content: 'refused content' };
    assert.equal(policy.begin('refused', 'Write', input), undefined);
    assert.equal(policy.refuse('refused', 'Write', input), true);
    assert.equal(policy.refuse('refused', 'Write', { ...input, content: 'changed' }), false);
    assert.equal(policy.refuse('refused', 'Edit', input), false);
    assert.equal(policy.begin('refused', 'Write', { file_path: 'safe.txt', content: 'no grant' }), undefined);
    assert.equal(policy.finish('refused', 'Write', input, true), false);
    assert.equal(policy.finish('refused', 'Write', input, false, true), false);
    assert.equal(policy.finish('refused', 'Write', { ...input, content: 'changed' }, false), false);
    assert.equal(policy.finish('foreign', 'Write', input, false), false);
    assert.equal(policy.finish('refused', 'Write', input, false), true);
    assert.equal(policy.finish('refused', 'Write', input, false), false);
    assert.equal(policy.refuse('refused', 'Write', input), false);
    const approved = { file_path: 'safe.txt', content: 'approved bytes' };
    assert.ok(policy.begin('approved', 'Write', approved));
    assert.equal(policy.refuse('approved', 'Write', approved), false);
    assert.equal(policy.approve('approved', 'Write', approved, true, false), true);
    assert.equal(policy.refuse('approved', 'Write', { ...approved, content: 'altered' }), false);
    writeFileSync(join(root, approved.file_path), 'partial effect');
    assert.equal(policy.finish('approved', 'Write', approved, false), false);
    assert.equal(policy.begin('later', 'Write', { file_path: 'later.txt', content: '' }), undefined);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('unreadable descendants preserve safe discovery siblings and report incomplete coverage', {
  skip: process.platform === 'win32' || process.getuid?.() === 0,
}, t => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-unreadable-')));
  const blocked = join(root, 'a-blocked');
  mkdirSync(blocked);
  writeFileSync(join(blocked, 'hidden.txt'), 'visible but inaccessible');
  mkdirSync(join(root, 'z-safe'));
  writeFileSync(join(root, 'z-safe/visible.txt'), 'visible safe source\n');
  try {
    const policy = new SDKWritePolicy(root, workspaceScope(root));
    chmodSync(blocked, 0o100);
    try {
      const directory = opendirSync(blocked);
      directory.closeSync();
      t.skip('Filesystem does not enforce directory read permission');
      return;
    } catch (error) {
      assert.ok(error && typeof error === 'object' && 'code' in error &&
        ['EACCES', 'EPERM'].includes(String(error.code)));
    }
    const files = policy.discover({ mode: 'glob' });
    assert.deepEqual(files.paths, ['z-safe/visible.txt']);
    assert.equal(files.truncated, true);
    const matches = policy.discover({ mode: 'grep', text: 'visible' });
    assert.deepEqual(matches.matches, [{ path: 'z-safe/visible.txt', line: 1, text: 'visible safe source' }]);
    assert.equal(matches.truncated, true);
    assert.throws(() => policy.discover({ mode: 'glob', path: 'a-blocked' }));
    chmodSync(root, 0o100);
    assert.throws(() => policy.discover({ mode: 'glob' }));
  } finally {
    chmodSync(root, 0o700);
    chmodSync(blocked, 0o700);
    rmSync(root, { recursive: true, force: true });
  }
});

test('canonical SDK roots retain built-in protected ancestors without excluding ordinary home or hidden projects', () => {
  const fixture = realpathSync(mkdtempSync(join(tmpdir(), 'eevee-sdk-root-scope-')));
  try {
    for (const name of ['secrets', 'CrEdEnTiAlS', '.git', '.ssh', '.claude', '.codex', '.gemini', '.opencode',
      '.ev-install', '.ds_store', '.env', '.env.local', '.npmrc', '.netrc', '.pypirc',
      'auth.json', 'oauth_creds.json', 'id_backup', 'private_key_cache', 'archive.pem', 'archive.key']) {
      const protectedRoot = join(fixture, name);
      const nested = join(protectedRoot, 'ordinary-project');
      mkdirSync(nested, { recursive: true });
      for (const root of [protectedRoot, nested]) {
        writeFileSync(join(root, 'recovery.txt'), 'ordinary-looking source');
        assert.throws(() => new SDKWritePolicy(root, reviewedScope(root, [])), /protected credential\/private directory/);
        assert.throws(() => new SDKWritePolicy(root, workspaceScope(root)), /protected credential\/private directory/);
        assert.throws(() => new SDKWritePolicy(root, reviewedScope(root, ['recovery.txt'])), /protected credential\/private directory/);
      }
    }
    for (const name of ['home/alice', '.hidden-project', 'workspace']) {
      const root = join(fixture, name);
      mkdirSync(root, { recursive: true });
      writeFileSync(join(root, 'recovery.txt'), 'ordinary source\n');
      mkdirSync(join(root, 'excluded'));
      writeFileSync(join(root, 'excluded/hidden.txt'), 'excluded source');
      const policy = new SDKWritePolicy(root, { ...workspaceScope(root), excluded_paths: ['excluded'] });
      assert.deepEqual(policy.discover({ mode: 'glob' }).paths, ['recovery.txt']);
      assert.equal(policy.readAdmission('Read', { file_path: 'recovery.txt' }).allowed, true);
      assert.equal(policy.readAdmission('Read', { file_path: 'excluded/hidden.txt' }).allowed, false);
      assert.ok(policy.begin('create', 'Write', { file_path: 'created.txt', content: 'exact proposal' }));
    }
    // User exclusions remain relative to the chosen root, unlike built-ins.
    const relative = join(fixture, 'excluded', 'ordinary-project');
    mkdirSync(relative, { recursive: true });
    writeFileSync(join(relative, 'recovery.txt'), 'ordinary source');
    const policy = new SDKWritePolicy(relative, { ...workspaceScope(relative), excluded_paths: ['excluded'] });
    assert.deepEqual(policy.discover({ mode: 'glob' }).paths, ['recovery.txt']);
  } finally { rmSync(fixture, { recursive: true, force: true }); }
});

test('host command schema is closed, literal and cannot inject configuration or grant shell in plan', () => {
  const direct = { cwd: '', command: { Direct: { program: 'rustc', args: ['main.rs', '-o', 'program'] } } };
  assert.equal(HostCommandSchema.safeParse(direct).success, true);
  assert.equal(HostCommandSchema.safeParse({ cwd: 'src', command: { Shell: { executable: '/bin/sh', dialect: 'Posix', text: 'cargo build' } } }).success, true);
  for (const cwd of ['/tmp', '../outside', 'src/../other', './src', 'src//other', 'bad\npath']) {
    assert.equal(HostCommandSchema.safeParse({ ...direct, cwd }).success, false, cwd);
  }
  for (const command of [{ Direct: { program: 'rustc', args: [], env: {} } }, { Shell: { executable: 'sh', dialect: 'bash', text: 'cargo build' } }, { Direct: { program: '', args: [] } }, { Direct: { program: 'rustc', args: [] }, Shell: { executable: 'sh', dialect: 'Posix', text: 'x' } }]) {
    assert.equal(HostCommandSchema.safeParse({ ...direct, command }).success, false);
  }
  assert.equal(HostCommandSchema.safeParse({ ...direct, timeout: 1 }).success, false);
  const init = { claude_code_version: compatibility.cliVersion, permissionMode: 'default', tools: [DISCOVERY_TOOL, COMMAND_TOOL],
    mcp_servers: [{ name: DISCOVERY_SERVER, status: 'connected' }, { name: COMMAND_SERVER, status: 'connected' }], plugins: [], skills: [] };
  assert.doesNotThrow(() => assertSessionPolicy(init, compatibility.cliVersion, [DISCOVERY_TOOL, COMMAND_TOOL]));
  assert.throws(() => assertSessionPolicy(init, compatibility.cliVersion, [DISCOVERY_TOOL]));
  assert.throws(() => assertSessionPolicy({ ...init, mcp_servers: [init.mcp_servers[0], init.mcp_servers[0]] }, compatibility.cliVersion, [DISCOVERY_TOOL, COMMAND_TOOL]));
  const result = { protocol: 1, type: 'bridge_command_result', generation: 1, task_id: 1,
    session_id: '00000000-0000-4000-8000-000000000001', call_id: '00000000-0000-4000-8000-000000000002', result: { status: 'error', stderr: 'compiler failure' } };
  assert.equal(CommandResultSchema.safeParse(result).success, true);
  assert.equal(CommandResultSchema.safeParse({ ...result, call_id: 'foreign' }).success, false);
  assert.equal(CommandResultSchema.safeParse({ ...result, result: { status: 'unknown' } }).success, false);
});
