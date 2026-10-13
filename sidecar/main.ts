import { createRequire } from 'node:module';
import { readFileSync, mkdirSync, lstatSync, existsSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { once } from 'node:events';
import { isDeepStrictEqual } from 'node:util';
import { query } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import type { Query, CanUseTool, OnElicitation, Options, SDKUserMessage } from '@anthropic-ai/claude-agent-sdk';
import { SDK_VERSION, CLI_VERSION, FRAME_LIMIT, READ_TOOLS, DENIED_TOOLS, selectedWrite, QuestionInputSchema, credentialRoute } from './policy.ts';

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
const RunSchema = z.object({
  protocol: z.literal(1), type: z.literal('run'), generation: z.number().int().positive(), task_id: z.number().int().positive(),
  prompt: z.string().max(FRAME_LIMIT / 2), cwd: z.string().min(1).max(4096), state_dir: z.string().min(1).max(4096),
  operation: z.enum(['plan', 'implement']), edit_paths: z.array(z.string().min(1).max(4096)).max(256),
  resume: z.string().uuid().nullish(),
});
const CommandSchema = z.discriminatedUnion('type', [
  RunSchema,
  z.object({ protocol: z.literal(1), type: z.literal('cancel'), generation: z.number().int().positive() }),
  z.object({ protocol: z.literal(1), type: z.literal('answer'), generation: z.number().int().positive(), request_id: z.string().max(256), answer: ResponseSchema }),
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
  if (cancelled || signal.aborted || seenRequests.has(id) || pending.size >= 16 || seenRequests.size >= 256) return fallback;
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

function cleanEnvironment(configDir: string) {
  // Credentials are inherited, never read from provider stores or serialized.
  // Isolated SDK home ensures the installed claude.ai subscription is unavailable.
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
  process.env.HOME = isolatedHome;
  process.env.USERPROFILE = isolatedHome;
  process.env.CLAUDE_CONFIG_DIR = configDir;
  process.env.CLAUDE_AGENT_SDK_CLIENT_APP = 'eevee/1.0';
  process.env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = '1';
}

async function run(command: z.infer<typeof RunSchema>) {
  if (started) throw new Error('One sidecar accepts exactly one query; follow-up turns are not queued');
  started = true;
  generation = command.generation;
  taskId = command.task_id;
  if (Buffer.byteLength(command.prompt) > FRAME_LIMIT / 2) throw new Error('Query byte bound exceeded');
  const account = credentialRoute(process.env);
  if (!account) throw new Error('Permitted API key or exactly one supported cloud route required; claude.ai subscriptions are not supported');
  cleanEnvironment(command.state_dir);
  controller = new AbortController();
  const tools = command.operation === 'implement' ? [...READ_TOOLS, 'Edit', 'Write'] : READ_TOOLS;
  const canUseTool: CanUseTool = async (tool, input, options) => {
    if (cancelled || !tools.includes(tool)) return deny;
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
      const parsed = QuestionAnswerSchema.safeParse(answer);
      return parsed.success ? { behavior: 'allow', updatedInput: { ...input, answers: parsed.data.updatedInput.answers } } : deny;
    }
    if ((tool === 'Edit' || tool === 'Write') && !selectedWrite(command.cwd, command.edit_paths, input.file_path)) return { behavior: 'deny', message: 'Write outside reviewed selected paths or through symlink refused' };
    const allowed = { behavior: 'allow' as const, updatedInput: input };
    const answer = await ask(id, options.toolUseID, { tool, input, description: options.description, title: options.title, blockedPath: options.blockedPath,
      options: [{ label: 'Deny', answer: deny }, { label: 'Allow once', answer: allowed }], defaultToNo: true, suppressAlwaysAllowRule: true },
      'approval', options.signal, deny, a => isDeepStrictEqual(a, allowed) || isDeepStrictEqual(a, deny));
    return PermissionSchema.parse(answer);
  };
  const onElicitation: OnElicitation = async (request, options) => {
    // MCP is mandatory-disabled. Never turn elicitation into a tool grant or open a login URL.
    const answer = await ask(options.requestId, request.serverName, { request, options: [{ label: 'Decline', answer: { action: 'decline' } },
      { label: 'Cancel', answer: { action: 'cancel' } }], reason: 'MCP disabled; acceptance/URL login unsupported' }, 'elicitation', options.signal,
      { action: 'cancel' }, a => ElicitationSchema.safeParse(a).success);
    return ElicitationSchema.parse(answer);
  };
  const options: Options = {
    cwd: command.cwd, resume: command.resume || undefined, continue: false, forkSession: false,
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
  if (command.operation === 'implement') {
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
  async function* onePrompt(): AsyncGenerator<SDKUserMessage> {
    if (!cancelled) yield { type: 'user', session_id: command.resume || '', message: { role: 'user', content: command.prompt }, parent_tool_use_id: null };
  }
  current = query({ prompt: onePrompt(), options });
  let resultSeen = false;
  let streamingMessage = '';
  const streaming = new Map<string, { text: string; at: number; bytes: number }>();
  const toolCalls = new Map<string, { tool: string; input: unknown }>();
  try {
    for await (const message of current) {
      if (message.type === 'system' && message.subtype === 'init') {
        if (message.claude_code_version !== CLI_VERSION || message.permissionMode !== 'default' ||
          message.tools.some(t => !tools.includes(t)) || message.mcp_servers.length || message.plugins.length || message.skills.length ||
          (account === 'ANTHROPIC_API_KEY' && message.apiKeySource !== 'ANTHROPIC_API_KEY')) throw new Error('Effective SDK identity/tool/config/auth policy mismatch');
        session = message.session_id;
        if (command.resume && session !== command.resume) throw new Error('SDK resumed a different session');
        await emit({ type: 'thread', ...meta(), session_id: session, model: message.model, account });
      } else if (message.type === 'stream_event') {
        const event = message.event;
        if (event.type === 'message_start') streamingMessage = event.message.id;
        if (event.type === 'content_block_delta' && event.delta.type === 'text_delta') {
          if (!streamingMessage || message.session_id !== session) throw new Error('Unqualified SDK text stream');
          const id = `${streamingMessage}:${event.index}`;
          const item = streaming.get(id) ?? { text: '', at: 0, bytes: 0 };
          item.text += event.delta.text;
          const size = Buffer.byteLength(item.text);
          if (size > 32 * 1024 || (!streaming.has(id) && streaming.size >= 128)) throw new Error('SDK text stream retention bound');
          if (Date.now() - item.at >= 250 || size - item.bytes >= 1024) {
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
          const call = toolCalls.get(block.tool_use_id);
          toolCalls.delete(block.tool_use_id);
          await emit({ type: 'item', ...meta(), session_id: message.session_id, item_id: block.tool_use_id, kind: 'tool', text: JSON.stringify({
            provider: 'Claude Agent SDK', sdk_version: SDK_VERSION, cli_version: CLI_VERSION, coverage: 'provider-native tool result; not an Eevee local command/check or proof of confinement',
            ...call, result: block.content, is_error: block.is_error, status: block.is_error ? 'failed' : 'completed',
          }), complete: true });
        }
      } else if (message.type === 'result') {
        resultSeen = true;
        if ((message.queued_turn_count ?? 0) !== 0) throw new Error('SDK reports an unexpected queued follow-up; stop without replay');
        await emit({ type: 'usage', ...meta(), session_id: message.session_id, scope: 'query-pipeline cumulative snapshot including restored session; replace, never sum; excludes out-of-pipeline helpers',
          estimated_cost_usd: message.total_cost_usd, model_usage: message.modelUsage, main_loop_per_turn: message.usage });
        await emit({ type: 'finished', ...meta(), session_id: message.session_id, outcome: cancelled ? 'cancelled' : message.is_error || message.subtype !== 'success' ? 'failed' : 'completed',
          message: cancelled ? 'SDK terminal result observed after interrupt request; no queued follow-up' :
            message.is_error ? `SDK ${message.subtype}: provider reported an error; raw diagnostics withheld` : message.subtype });
        break;
      }
    }
    if (!resultSeen) throw new Error('SDK ended without terminal result; outcome unknown');
  } finally {
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
  await emit({ type: 'capabilities', sdk_version: SDK_VERSION, cli_version: CLI_VERSION, node_version: process.versions.node, node_executable: realpathSync(process.execPath), sdk_entry: sdkEntry, cli,
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
        } else if (command.type === 'answer' && command.generation === generation && !cancelled) {
          const p = pending.get(command.request_id);
          if (p && p.accept(command.answer)) p.resolve(command.answer);
          else void emit({ type: 'status', ...meta(), message: 'Stale or invalid SDK answer refused' }).catch(fatal);
        } else throw new Error('Unsupported or stale IPC request');
      } catch (error) { fatal(error); }
    }
  });
  process.stdin.on('end', () => { controller?.abort(); current?.close(); });
}
