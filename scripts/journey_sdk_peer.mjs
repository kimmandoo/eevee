// Opt-in controlled query producer; production bridge/policy and SDK helpers stay real.
import { createSdkMcpServer, tool } from './sdk-real.mjs';
export { createSdkMcpServer, tool };
import { readFileSync, appendFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import assert from 'node:assert/strict';
const version = JSON.parse(readFileSync(new URL('./package.json', import.meta.url))).claudeCodeVersion;
const models = [{ value: 'journey-rich', displayName: 'Journey rich', description: 'Controlled effort ladder', supportsEffort: true, supportedEffortLevels: ['low', 'high', 'max'] }, { value: 'journey-fixed', displayName: 'Journey fixed', description: 'Controlled provider-fixed effort', supportsEffort: false }];
function record(kind, value = {}) { appendFileSync(process.env.EEVEE_JOURNEY_LOG, JSON.stringify({ kind, ...value }) + '\n'); }
record('module_loaded');
export function query({ prompt, options }) {
  record('query_created');
  let interrupted = false;
  return {
    accountInfo: async () => { record('account_info'); return { apiProvider: 'firstParty' }; },
    initializationResult: async () => { record('initialization'); return { claude_code_version: version, hooks_applied: true }; },
    supportedModels: async () => { record('metadata'); return models; },
    setModel: async model => { options.model = model; },
    getContextUsage: async () => ({ model: options.model || 'journey-rich' }),
    close() {}, interrupt: async () => { interrupted = true; },
    async *[Symbol.asyncIterator]() {
      const { value: submission } = await prompt.next();
      assert.ok(submission, 'production host must release correlated user submission');
      const session_id = options.resume || '00000000-0000-4000-8000-000000000002';
      record('submitted', { model: options.model, prompt: submission.message.content, resume: options.resume ?? null, session_id });
      const implement = options.tools.includes('Write');
      yield { type: 'system', subtype: 'init', session_id, model: options.model || 'journey-rich', claude_code_version: version, permissionMode: 'default', apiKeySource: 'ANTHROPIC_API_KEY', tools: [...options.tools, 'mcp__eevee_source__discover', ...(implement ? ['mcp__eevee_command__run_command'] : [])], mcp_servers: [{ name: 'eevee_source', status: 'connected' }, ...(implement ? [{ name: 'eevee_command', status: 'connected' }] : [])], plugins: [], skills: [] };
      if (!existsSync(process.env.EEVEE_JOURNEY_RELEASE)) {
        yield { type: 'assistant', session_id, user_message_uuid: submission.uuid, message: { id: 'streaming', content: [{ type: 'text', text: 'GP10_STREAMING source inspection pending' }] } };
        while (!existsSync(process.env.EEVEE_JOURNEY_RELEASE) && !interrupted) await new Promise(resolve => setTimeout(resolve, 25));
      }
      if (options.model === 'journey-rich') {
        yield { type: 'assistant', session_id, user_message_uuid: submission.uuid, message: { id: 'todo-init', content: [{ type: 'tool_use', id: 'todo-init', name: 'TodoWrite', input: { todos: [{ content: 'Read source / 한글', status: 'in_progress', activeForm: 'Reading source' }] } }] } };
        yield { type: 'user', session_id, message: { content: [{ type: 'tool_result', tool_use_id: 'todo-init', content: 'Provider progress claim only' }] } };
      }
      const input = { file_path: join(options.cwd, 'source.txt') };
      yield { type: 'assistant', session_id, user_message_uuid: submission.uuid, message: { id: 'source-read', content: [{ type: 'tool_use', id: 'source-read', name: 'Read', input }] } };
      const hook = await options.hooks.PreToolUse[0].hooks[0]({ hook_event_name: 'PreToolUse', tool_use_id: 'source-read', tool_name: 'Read', tool_input: input });
      assert.notEqual(hook.hookSpecificOutput?.permissionDecision, 'deny');
      const consent = await options.canUseTool('Read', input, { toolUseID: 'source-read', requestId: 'source-read', signal: options.abortController.signal });
      assert.equal(consent.behavior, 'allow');
      const content = readFileSync(input.file_path, 'utf8');
      assert.match(content, /JOURNEY_SOURCE_한글/);
      record('actual_read', { content });
      yield { type: 'user', session_id, message: { content: [{ type: 'tool_result', tool_use_id: 'source-read', content }] } };
      if (JSON.stringify(submission.message.content).includes('GP10_HOLD')) {
        yield { type: 'assistant', session_id, message: { id: 'held-partial', content: [{ type: 'text', text: 'CANCELLED_PUBLIC_PARTIAL_한글界\n' }] } };
      }
      while (JSON.stringify(submission.message.content).includes('GP10_HOLD') && !interrupted) await new Promise(resolve => setTimeout(resolve, 25));
      if (!interrupted) yield { type: 'assistant', session_id, message: { id: 'answer', content: [{ type: 'text', text: 'Controlled actual source read completed.' }] } };
      record('finished', { interrupted });
      yield { type: 'result', session_id, is_error: false, subtype: 'success' };
    },
  };
}
