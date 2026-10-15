import { createRequire } from 'node:module';
import { readFileSync, mkdirSync, lstatSync, existsSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { once } from 'node:events';
import { isDeepStrictEqual } from 'node:util';
import { query } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import type { Query, CanUseTool, OnElicitation, Options, SDKUserMessage } from '@anthropic-ai/claude-agent-sdk';
import { SDK_VERSION, CLI_VERSION, FRAME_LIMIT, READ_TOOLS, DENIED_TOOLS, selectedWrite, selectedWriteIdentity, QuestionInputSchema, RunSchema, SDKPolicyError, querySelection, prepareModels, assertAppliedModel, credentialRoute, consoleProfile, sdkOutcome } from './policy.ts';

const require = createRequire(import.meta.url);
const sdkEntry = require.resolve('@anthropic-ai/claude-agent-sdk');
const sdkPackage = z.object({ version: z.literal(SDK_VERSION), claudeCodeVersion: z.literal(CLI_VERSION) }).parse(JSON.parse(readFileSync(join(dirname(sdkEntry), 'package.json'), 'utf8')));
if (sdkPackage.version !== SDK_VERSION || sdkPackage.claudeCodeVersion !== CLI_VERSION) throw new Error('Unsupported SDK/CLI version');
if (Number(process.versions.node.split('.')[0]) < 24) throw new Error('Native TypeScript sidecar requires Node 24 or later');
const report = z.object({ header: z.object({ glibcVersionRuntime: z.string().optional() }) }).parse(process.report.getReport());
const platform = `${process.platform}-${process.arch}${process.platform === 'linux' && report.header.glibcVersionRuntime === undefined ? '-musl' : ''}`;
const nativePackage = dirname(require.resolve(`@anthropic-ai/claude-agent-sdk-${platform}/package.json`));
const cli = realpathSync(join(nativePackage, process.platform === 'win32' ? 'claude.exe' : 'claude'));

const PermissionSchema = z.union([
  z.object({ behavior: z.literal('deny'), message: z.string().max(8192) }).strict(),
  z.object({ behavior: z.literal('allow'), updatedInput: z.record(z.string(), z.unknown()) }).strict(),
]);
const ElicitationSchema = z.object({ action: z.enum(['decline', 'cancel']) }).strict();
const ResponseSchema = z.union([PermissionSchema, ElicitationSchema]);
type Response = z.infer<typeof ResponseSchema>;
const QuestionAnswerSchema = z.object({
  behavior: z.literal('allow'), updatedInput: z.object({ answers: z.record(z.string(), z.string().min(1).max(8192)) }).strict(),
}).strict();
const CommandSchema = z.discriminatedUnion('type', [
  RunSchema,
  z.object({ protocol: z.literal(1), type: z.literal('cancel'), generation: z.number().int().positive() }).strict(),
  z.object({ protocol: z.literal(1), type: z.literal('answer'), generation: z.number().int().positive(), request_id: z.string().max(256), answer: ResponseSchema }).strict(),
]);
let written = 0;
let pendingWrites = 0;
let outputChain = Promise.resolve();
function emit(value: Record<string, unknown>): Promise<void> {
  const wire = JSON.stringify({ protocol: 1, ...value }) + '\n';
  const size = Buffer.byteLength(wire);
  if (size > FRAME_LIMIT || written + size > 64 * 1024 * 1024 || pendingWrites >= 128) {
    controller?.abort();
    throw new Error('Sidecar output bounds exceeded');
  }
  written += size;
  pendingWrites++;
  outputChain = outputChain.then(async () => { if (!process.stdout.write(wire)) await once(process.stdout, 'drain'); }).finally(() => { pendingWrites--; });
  return outputChain;
}

let current: Query | undefined;
let controller: AbortController | undefined;
let started = false;
let cancelled = false;
let session = '';
let generation = 0;
let taskId = 0;
let promptSequence = 0;
type Pending = { resolve: (answer: Response) => void; accept: (answer: Response) => boolean; deny: Response };
const pending = new Map<string, Pending>();
const seenRequests = new Set<string>();
const deny = { behavior: 'deny' as const, message: 'User declined or request cancelled' };
const meta = () => ({ generation, task_id: taskId });

async function ask(id: string, tool: string, input: unknown, kind: string, signal: AbortSignal, fallback: Response, accept: (answer: Response) => boolean): Promise<Response> {
  if (cancelled || signal.aborted || seenRequests.has(id) || pending.size !== 0 || seenRequests.size >= 256) return fallback;
  seenRequests.add(id);
  const { promise, resolve: resolveAnswer } = Promise.withResolvers<Response>();
  const close = (answer: Response) => {
    if (!pending.delete(id)) return;
    signal.removeEventListener('abort', abort);
    void emit({ type: 'prompt_closed', ...meta(), request_id: id }).catch(fatal);
    resolveAnswer(answer);
  };
  const abort = () => close(fallback);
  pending.set(id, { resolve: close, accept, deny: fallback });
  signal.addEventListener('abort', abort, { once: true });
  void emit({ type: 'prompt', ...meta(), request_id: id, session_id: session, item_id: tool, kind, input }).catch(fatal);
  return promise;
}

function cleanEnvironment(configDir: string, externalCloud: boolean, profile: ReturnType<typeof consoleProfile>) {
  // Only explicitly approved Bedrock/Vertex chains may use the inherited home for
  // provider-owned SSO/ADC caches. Claude's own configuration remains isolated.
  // A Console profile is pinned by explicit directory/name before HOME is replaced;
  // other routes get an empty profile directory so no implicit profile is discovered.
  delete process.env.ANTHROPIC_PROFILE;
  delete process.env.ANTHROPIC_CONFIG_DIR;
  for (const name of Object.keys(process.env)) {
    if ((name.startsWith('CLAUDE_') && !['CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX'].includes(name)) ||
      ['ANTHROPIC_AUTH_TOKEN', 'NODE_OPTIONS', 'NODE_PATH', 'BUN_OPTIONS'].includes(name)) delete process.env[name];
  }
  mkdirSync(configDir, { recursive: true, mode: 0o700 });
  if (lstatSync(configDir).isSymbolicLink()) throw new Error('SDK state directory must not be a symlink');
  for (const file of ['settings.json', 'settings.local.json', '.credentials.json', 'plugins', 'hooks', 'mcp.json']) {
    if (existsSync(join(configDir, file))) throw new Error('Isolated SDK state contains unreviewed executable/auth configuration');
  }
  const isolatedHome = join(realpathSync(configDir), 'home');
  mkdirSync(isolatedHome, { recursive: true, mode: 0o700 });
  if (lstatSync(isolatedHome).isSymbolicLink()) throw new Error('SDK isolated home must not be a symlink');
  for (const file of ['.claude', '.claude.json']) {
    if (existsSync(join(isolatedHome, file))) throw new Error('SDK isolated home contains an unreviewed provider store/configuration');
  }
  if (!externalCloud) {
    process.env.HOME = isolatedHome;
    process.env.USERPROFILE = isolatedHome;
  }
  process.env.CLAUDE_CONFIG_DIR = configDir;
  process.env.ANTHROPIC_CONFIG_DIR = profile ? profile.dir : join(realpathSync(configDir), 'no-anthropic-profile');
  if (profile) process.env.ANTHROPIC_PROFILE = profile.name;
  process.env.CLAUDE_AGENT_SDK_CLIENT_APP = 'eevee/1.0';
  process.env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = '1';
}

async function run(command: z.infer<typeof RunSchema>) {
  if (started) throw new Error('One sidecar accepts exactly one query; follow-up turns are not queued');
  started = true;
  generation = command.generation;
  taskId = command.task_id;
  if (Buffer.byteLength(command.prompt) > FRAME_LIMIT / 2) throw new Error('Query byte bound exceeded');
  const profile = consoleProfile(process.env);
  const account = credentialRoute(process.env, profile);
  if (!account) throw new Error('Permitted API key, Console OAuth profile or exactly one supported cloud route required; claude.ai subscriptions are not supported');
  const expectedProvider = process.env.CLAUDE_CODE_USE_BEDROCK === '1' ? 'bedrock' :
    process.env.CLAUDE_CODE_USE_VERTEX === '1' ? 'vertex' : 'firstParty';
  cleanEnvironment(command.state_dir, expectedProvider !== 'firstParty' && process.env.EEVEE_CLAUDE_CLOUD_AUTH === 'provider-chain', profile);
  controller = new AbortController();
  const tools = command.inspect_models ? [] : command.operation === 'implement' ? [...READ_TOOLS, 'Edit', 'Write'] : READ_TOOLS;
  let activeWrite: string | undefined;
  const canUseTool: CanUseTool = async (tool, input, options) => {
    if (cancelled || options.signal.aborted || !tools.includes(tool)) return deny;
    const id = options.requestId || `permission-${++promptSequence}`;
    if (tool === 'AskUserQuestion') {
      const questions = QuestionInputSchema.safeParse(input);
      if (!questions.success) return { behavior: 'deny', message: 'Unsupported question schema (multi-select, duplicate or oversized questions)' };
      const answer = await ask(id, options.toolUseID, input, 'questions', options.signal, deny, a => {
        const parsed = QuestionAnswerSchema.safeParse(a);
        if (!parsed.success) return false;
        const values = parsed.data.updatedInput.answers;
        return Object.keys(values).length === questions.data.questions.length &&
          questions.data.questions.every(q => typeof values[q.question] === 'string');
      });
      if (cancelled || options.signal.aborted) return deny;
      const parsed = QuestionAnswerSchema.safeParse(answer);
      return parsed.success ? { behavior: 'allow', updatedInput: { ...input, answers: parsed.data.updatedInput.answers } } : deny;
    }
    const writes = tool === 'Edit' || tool === 'Write';
    if (writes && activeWrite !== undefined) return { behavior: 'deny', message: 'Another write is awaiting review or a terminal tool result' };
    const baseline = writes ? selectedWriteIdentity(command.cwd, command.edit_paths, input.file_path) : undefined;
    if (writes && baseline === undefined) return { behavior: 'deny', message: 'Write outside reviewed selected paths or through symlink refused' };
    if (writes) activeWrite = options.toolUseID;
    let approved = false;
    try {
      const allowed = { behavior: 'allow' as const, updatedInput: input };
      const answer = await ask(id, options.toolUseID, { tool, input, description: options.description, title: options.title, blockedPath: options.blockedPath,
        options: [{ label: 'Deny', answer: deny }, { label: 'Allow once', answer: allowed }], defaultToNo: true, suppressAlwaysAllowRule: true },
        'approval', options.signal, deny, a => isDeepStrictEqual(a, allowed) || isDeepStrictEqual(a, deny));
      if (cancelled || options.signal.aborted ||
        (writes && selectedWriteIdentity(command.cwd, command.edit_paths, input.file_path) !== baseline)) return deny;
      const permission = PermissionSchema.parse(answer);
      approved = permission.behavior === 'allow';
      return permission;
    } finally {
      if (writes && !approved) activeWrite = undefined;
    }
  };
  const onElicitation: OnElicitation = async (request, options) => {
    if (command.inspect_models) return { action: 'cancel' };
    // MCP is mandatory-disabled. Never turn elicitation into a tool grant or open a login URL.
    const answer = await ask(options.requestId, request.serverName, { request, options: [{ label: 'Decline', answer: { action: 'decline' } },
      { label: 'Cancel', answer: { action: 'cancel' } }], reason: 'MCP disabled; acceptance/URL login unsupported' }, 'elicitation', options.signal,
      { action: 'cancel' }, a => ElicitationSchema.safeParse(a).success);
    return ElicitationSchema.parse(answer);
  };
  const options: Options = {
    cwd: command.cwd, resume: command.resume || undefined, continue: false, forkSession: false,
    ...querySelection(command.selection),
    pathToClaudeCodeExecutable: cli, abortController: controller,
    permissionMode: 'default', permissionPrompts: 'host', allowDangerouslySkipPermissions: false,
    tools, allowedTools: [], disallowedTools: [...DENIED_TOOLS, ...(command.operation === 'plan' ? ['Edit', 'Write'] : [])],
    settingSources: [], settings: { disableAllHooks: true, permissions: { ask: tools } },
    strictMcpConfig: true, mcpServers: {}, plugins: [], skills: [], agents: {},
    canUseTool, onElicitation, maxTurns: 16, maxBudgetUsd: 5, includePartialMessages: true,
    promptSuggestions: false, agentProgressSummaries: false,
    systemPrompt: 'You are Claude Agent in Eevee. Use only reviewed task context and exposed tools. Do not delegate, load extra configuration, or persist permission rules. Tool observations are provider-native, not verified Eevee local runs.',
    stderr: () => {},
  };
  // Host-owned policy hook never executes project hooks. disableAllHooks disables filesystem hooks;
  // mandatory tools exposure blocks every mutation route in Plan independent of hook invocation.
  if (!command.inspect_models && command.operation === 'implement') {
    options.settings = { permissions: { ask: tools }, disableAllHooks: false };
    options.hooks = { PreToolUse: [{ hooks: [async (input) => {
      if (input.hook_event_name !== 'PreToolUse') return { continue: false };
      const name = input.tool_name;
      const toolInput = z.object({ file_path: z.unknown() }).safeParse(input.tool_input);
      if (!tools.includes(name) || ((name === 'Write' || name === 'Edit') && (!toolInput.success || !selectedWrite(command.cwd, command.edit_paths, toolInput.data.file_path)))) {
        return { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: 'Mandatory selected-path/tool policy' } };
      }
      return { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'ask', permissionDecisionReason: 'Review this one operation in Eevee' } };
    }] }] };
  }
  const authReady = Promise.withResolvers<boolean>();
  async function* onePrompt(): AsyncGenerator<SDKUserMessage> {
    if (await authReady.promise && !cancelled && !command.inspect_models) yield { type: 'user', session_id: command.resume || '', message: { role: 'user', content: command.prompt }, parent_tool_use_id: null };
  }
  current = query({ prompt: onePrompt(), options });
  let resultSeen = false;
  let promptReleased = false;
  let streamingMessage = '';
  const streaming = new Map<string, { text: string; at: number; bytes: number }>();
  const toolCalls = new Map<string, { tool: string; input: unknown }>();
  try {
    // Backend and live model/effort validation complete before any user prompt.
    const { catalogue, approvedModel } = await prepareModels(current, command.selection, account, expectedProvider, command.inspect_models);
    if (command.inspect_models) {
      if (cancelled || controller.signal.aborted) throw new Error('Model inspection interrupted');
      await emit({ type: 'models', ...meta(), catalogue });
      await emit({ type: 'finished', ...meta(), session_id: '', outcome: 'completed', message: 'Live SDK model capabilities inspected; no prompt or model turn sent' });
      resultSeen = true;
      return;
    }
    if (catalogue.models.length === 0) throw new SDKPolicyError('Model selection unsupported: the live SDK returned no supported models; no prompt sent');
    promptReleased = true;
    authReady.resolve(true);
    for await (const message of current) {
      if (session && 'session_id' in message && message.session_id !== session) throw new Error('SDK message changed session identity');
      if (message.type === 'system' && message.subtype === 'init') {
        if (message.claude_code_version !== CLI_VERSION || message.permissionMode !== 'default' ||
          message.tools.some(t => !tools.includes(t)) || message.mcp_servers.length || message.plugins.length || message.skills.length ||
          (account === 'ANTHROPIC_API_KEY' && message.apiKeySource !== 'ANTHROPIC_API_KEY') ||
          (profile !== undefined && ['ANTHROPIC_API_KEY', 'apiKeyHelper', '/login managed key'].includes(message.apiKeySource))) throw new Error('Effective SDK identity/tool/config/auth policy mismatch');
        assertAppliedModel(approvedModel, message.model);
        if (session) throw new Error('SDK emitted a second session initialization');
        session = message.session_id;
        if (command.resume && session !== command.resume) throw new Error('SDK resumed a different session');
        await emit({ type: 'thread', ...meta(), session_id: session, model: message.model, account });
      } else if (message.type === 'stream_event') {
        const event = message.event;
        if (event.type === 'message_start') {
          if (!session || message.session_id !== session) throw new Error('Unqualified SDK stream');
          streamingMessage = event.message.id;
        }
        if (event.type === 'content_block_delta' && event.delta.type === 'text_delta') {
          if (!streamingMessage || message.session_id !== session) throw new Error('Unqualified SDK text stream');
          const id = `${streamingMessage}:${event.index}`;
          const item = streaming.get(id) ?? { text: '', at: 0, bytes: 0 };
          item.text += event.delta.text;
          const size = Buffer.byteLength(item.text);
          if (size > 32 * 1024 || (!streaming.has(id) && streaming.size >= 128)) throw new Error('SDK text stream retention bound');
          if (Date.now() - item.at >= 30 || size - item.bytes >= 256) {
            await emit({ type: 'item', ...meta(), session_id: session, item_id: id, kind: 'message', text: item.text, complete: false });
            item.at = Date.now(); item.bytes = size;
          }
          streaming.set(id, item);
        }
      } else if (message.type === 'assistant') {
        for (let i = 0; i < message.message.content.length; i++) {
          const block = message.message.content[i];
          if (block.type === 'text') {
            streaming.delete(`${message.message.id}:${i}`);
            await emit({ type: 'item', ...meta(), session_id: message.session_id, item_id: `${message.message.id}:${i}`, kind: 'message', text: block.text, complete: true });
          }
          if (block.type === 'tool_use') {
            if (toolCalls.size >= 128 || Buffer.byteLength(JSON.stringify(block.input)) > 32 * 1024) throw new Error('SDK tool input retention bound');
            toolCalls.set(block.id, { tool: block.name, input: block.input });
            await emit({ type: 'item', ...meta(), session_id: message.session_id, item_id: block.id, kind: 'tool', text: JSON.stringify({ provider: 'Claude Agent SDK', sdk_version: SDK_VERSION, tool: block.name, input: block.input, status: 'requested' }), complete: false });
          }
        }
      } else if (message.type === 'user' && Array.isArray(message.message.content)) {
        for (const block of message.message.content) if (block.type === 'tool_result') {
          if (!toolCalls.has(block.tool_use_id)) throw new Error('SDK tool result lacks an observed call');
          const call = toolCalls.get(block.tool_use_id);
          toolCalls.delete(block.tool_use_id);
          if (activeWrite === block.tool_use_id) activeWrite = undefined;
          await emit({ type: 'item', ...meta(), session_id: message.session_id, item_id: block.tool_use_id, kind: 'tool', text: JSON.stringify({
            provider: 'Claude Agent SDK', sdk_version: SDK_VERSION, cli_version: CLI_VERSION, coverage: 'provider-native tool result; not an Eevee local command/check or proof of confinement',
            ...call, result: block.content, is_error: block.is_error, status: block.is_error ? 'failed' : 'completed',
          }), complete: true });
        }
      } else if (message.type === 'result') {
        resultSeen = true;
        if ((message.queued_turn_count ?? 0) !== 0) throw new Error('SDK reports an unexpected queued follow-up; stop without replay');
        for (const [id, item] of streaming) {
          await emit({ type: 'item', ...meta(), session_id: session, item_id: id, kind: 'message', text: item.text, complete: false });
        }
        streaming.clear();
        await emit({ type: 'usage', ...meta(), session_id: message.session_id, scope: 'query-pipeline cumulative snapshot including restored session; replace, never sum; excludes out-of-pipeline helpers',
          estimated_cost_usd: message.total_cost_usd, model_usage: message.modelUsage, main_loop_per_turn: message.usage });
        const outcome = sdkOutcome(cancelled, message.is_error, message.subtype);
        await emit({ type: 'finished', ...meta(), session_id: message.session_id, outcome,
          message: outcome === 'unknown' ? 'SDK terminal error observed after interrupt; cancellation was requested, not confirmed; prior actions are not rolled back' :
            message.is_error ? `SDK ${message.subtype}: provider reported an error; raw diagnostics withheld` : message.subtype });
        break;
      }
    }
    if (!resultSeen) throw new Error('SDK ended without terminal result; outcome unknown');
  } catch (error) {
    if (!(error instanceof SDKPolicyError) || session !== '' || promptReleased) throw error;
    await emit({ type: 'rejected', ...meta(), session_id: '', message: error.message });
    resultSeen = true;
  } finally {
    authReady.resolve(false);
    for (const [id, item] of streaming) {
      await emit({ type: 'item', ...meta(), session_id: session, item_id: id, kind: 'message', text: item.text, complete: false });
    }
    for (const p of [...pending.values()]) p.resolve(p.deny);
    current.close();
    current = undefined;
    await outputChain;
    process.stdin.pause();
    process.exitCode = resultSeen ? 0 : 1;
  }
}
function fatal(_error: unknown) {
  cancelled = true;
  controller?.abort();
  for (const p of [...pending.values()]) p.resolve(p.deny);
  void emit({ type: 'fatal', ...meta(), message: 'Claude SDK sidecar failed; raw diagnostics withheld; outcome unknown' }).finally(() => { current?.close(); process.exitCode = 1; process.stdin.pause(); }).catch(() => process.exit(1));
}
if (process.argv.includes('--inspect')) {
  await emit({ type: 'capabilities', bridge_revision: 2, sdk_version: SDK_VERSION, cli_version: CLI_VERSION, node_version: process.versions.node, node_executable: realpathSync(process.execPath), sdk_entry: sdkEntry, cli,
    query: typeof query === 'function', approvals: 'canUseTool with exact once/deny answers; no persistent rules', questions: 'AskUserQuestion single-select/free text',
    elicitation: 'distinct onElicitation; MCP disabled so decline/cancel only', resume: 'explicit same-task session ID only',
    usage: 'latest cumulative query-pipeline snapshot, not additive', confinement: 'agent policy; not OS sandbox', credential_route: credentialRoute(process.env) ?? 'unavailable' });
} else {
  let bytes = Buffer.alloc(0);
  process.stdin.on('data', (chunk: Buffer) => {
    bytes = Buffer.concat([bytes, chunk]);
    if (bytes.length > FRAME_LIMIT) return fatal(new Error('IPC frame bound'));
    while (true) {
      const end = bytes.indexOf(10);
      if (end < 0) break;
      const line = bytes.subarray(0, end); bytes = bytes.subarray(end + 1);
      try {
        const command = CommandSchema.parse(JSON.parse(line.toString('utf8')));
        if (command.type === 'run') void run(command).catch(fatal);
        else if (command.type === 'cancel' && command.generation === generation) {
          cancelled = true;
          for (const p of [...pending.values()]) p.resolve(p.deny);
          if (current) void current.interrupt().then(() => emit({ type: 'status', ...meta(), message: 'SDK interrupt acknowledged; awaiting terminal result' })).catch(fatal);
          else controller?.abort();
        } else if (command.type === 'answer' && command.generation === generation) {
          const p = pending.get(command.request_id);
          if (!cancelled && p && p.accept(command.answer)) p.resolve(command.answer);
          else void emit({ type: 'status', ...meta(), message: 'Stale or invalid SDK answer refused' }).catch(fatal);
        } else throw new Error('Unsupported or stale IPC request');
      } catch (error) { fatal(error); }
    }
  });
  process.stdin.on('end', () => { controller?.abort(); current?.close(); });
}
