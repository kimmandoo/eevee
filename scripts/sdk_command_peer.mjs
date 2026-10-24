// Controlled SDK query producer for sdk_command_smoke.py. No model/CLI/account call.
// Production main.ts/policy.ts and the installed SDK MCP server execute unchanged.
import { createSdkMcpServer, tool } from './sdk-real.mjs';
export { createSdkMcpServer, tool };
import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { join } from 'node:path';
import assert from 'node:assert/strict';
const version = JSON.parse(readFileSync(new URL('./package.json', import.meta.url))).claudeCodeVersion;
const source = 'fn main() { let answer = not_defined; println!("SDK-CODING:{}", answer); }\n';
const fixed = source.replace('not_defined', '42');
const commandTool = 'mcp__eevee_command__run_command';
const session_id = '00000000-0000-4000-8000-000000000001';
function record(kind, value) {
  appendFileSync(process.env.EV_SDK_PROOF_LOG, JSON.stringify({ kind, ...value }) + '\n');
}
export function query({ prompt, options }) {
  let interrupted = false;
  return {
    accountInfo: async () => ({ apiProvider: 'firstParty' }),
    initializationResult: async () => ({ claude_code_version: version, hooks_applied: true }),
    supportedModels: async () => [],
    close() {},
    interrupt: async () => { interrupted = true; },
    async *[Symbol.asyncIterator]() {
      const { value: submission } = await prompt.next();
      assert.ok(submission, 'host must release a real correlated submission');
      const implement = options.tools.includes('Write');
      yield { type: 'system', subtype: 'init', session_id, model: 'controlled-sdk-proof',
        claude_code_version: version, permissionMode: 'default', apiKeySource: 'ANTHROPIC_API_KEY',
        tools: [...options.tools, 'mcp__eevee_source__discover', ...(implement ? [commandTool] : [])],
        mcp_servers: [{ name: 'eevee_source', status: 'connected' }, ...(implement ? [{ name: 'eevee_command', status: 'connected' }] : [])], plugins: [], skills: [] };
      if (!implement) {
        assert.equal(options.mcpServers.eevee_command, undefined);
        record('readonly', { command_server: false });
        yield { type: 'assistant', session_id, user_message_uuid: submission.uuid, message: { id: 'readonly', content: [{ type: 'text', text: 'No command route exposed.' }] } };
        yield { type: 'result', session_id, is_error: false, subtype: 'success' };
        return;
      }
      const server = options.mcpServers.eevee_command.instance;
      const responses = new Map();
      let sequence = 0;
      const transport = { async start() {}, async close() {}, async send(message) { responses.get(message.id)?.(message); } };
      await server.connect(transport);
      const rpc = (method, params) => {
        const { promise, resolve } = Promise.withResolvers();
        const id = ++sequence;
        responses.set(id, resolve);
        transport.onmessage({ jsonrpc: '2.0', id, method, params });
        return promise;
      };
      assert.ok((await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'sdk-command-proof', version: '1' } })).result);
      transport.onmessage({ jsonrpc: '2.0', method: 'notifications/initialized' });
      assert.deepEqual((await rpc('tools/list', {})).result.tools.map(value => value.name), ['run_command']);
      const compiler = process.env.EV_SDK_COMPILER;
      const product = process.platform === 'win32' ? 'sdk-program.exe' : 'sdk-program';
      const build = { cwd: '', command: { Direct: { program: compiler, args: ['--edition=2021', '--color=never', 'main.rs', '-o', product] } } };
      const steps = [
        { id: 'write-broken', tool: 'Write', input: { file_path: 'main.rs', content: source } },
        { id: 'compile-failure', tool: commandTool, input: build },
        { id: 'write-fix', tool: 'Write', input: { file_path: 'main.rs', content: fixed } },
        { id: 'compile-success', tool: commandTool, input: build },
        { id: 'run-program', tool: commandTool, input: { cwd: '', command: { Direct: { program: join(options.cwd, product), args: [] } } } },
      ];
      for (const step of steps) {
        assert.equal(interrupted, false, 'interrupted query must not start another step');
        yield { type: 'assistant', session_id, user_message_uuid: submission.uuid,
          message: { id: step.id, content: [{ type: 'tool_use', id: step.id, name: step.tool, input: step.input }] } };
        const checked = await options.hooks.PreToolUse[0].hooks[0]({ hook_event_name: 'PreToolUse', tool_use_id: step.id, tool_name: step.tool, tool_input: step.input });
        assert.notEqual(checked.hookSpecificOutput.permissionDecision, 'deny');
        const consent = await options.canUseTool(step.tool, step.input, { toolUseID: step.id, requestId: step.id, signal: options.abortController.signal });
        assert.equal(consent.behavior, 'allow');
        let content, is_error = false;
        if (step.tool === 'Write') {
          // SDK-native writes remain external-policy effects, only after the
          // production host Write policy's exact user approval.
          writeFileSync(join(options.cwd, step.input.file_path), step.input.content);
          content = 'Reviewed file write completed';
        } else {
          const response = await rpc('tools/call', { name: 'run_command', arguments: step.input });
          assert.ok(response.result, JSON.stringify(response));
          content = response.result.content;
          is_error = response.result.isError === true;
          assert.match(content[0].text, /^\{/, JSON.stringify(response));
          const value = JSON.parse(content[0].text);
          if (step.id === 'compile-failure') {
            assert.equal(is_error, true);
            assert.equal(value.status, 'error');
            assert.notEqual(value.run.exit_code, 0);
            assert.match(value.stderr, /not_defined/);
          } else {
            assert.equal(is_error, false);
            assert.equal(value.status, 'success');
            assert.equal(value.run.exit_code, 0);
            if (step.id === 'run-program') assert.match(value.stdout, /SDK-CODING:42/);
          }
          assert.ok(value.artifact, 'durable actual output artifact must reach the model');
          record('command_result', { id: step.id, result: value, is_error });
        }
        yield { type: 'user', session_id, message: { content: [{ type: 'tool_result', tool_use_id: step.id, content, is_error }] } };
        record('step', { id: step.id });
      }
      await server.close();
      yield { type: 'assistant', session_id, message: { id: 'done', content: [{ type: 'text', text: 'SDK-CODING:42 observed from the actual built program.' }] } };
      record('finished', {});
      yield { type: 'result', session_id, is_error: false, subtype: 'success' };
    },
  };
}
