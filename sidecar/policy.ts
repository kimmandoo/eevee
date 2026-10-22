import { z } from 'zod';
import { lstatSync, realpathSync, readFileSync, readdirSync, opendirSync } from 'node:fs';
import type { BigIntStats } from 'node:fs';
import { isAbsolute, join, matchesGlob, parse, relative, resolve, sep, toNamespacedPath } from 'node:path';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import type { EffortLevel, ModelInfo } from '@anthropic-ai/claude-agent-sdk';

// Versions are observed identity, never a release allowlist.
const VersionSchema = z.string().max(128).regex(/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)*$/).refine(value => value.trim() === value);
export const SDKPackageSchema = z.object({
  name: z.literal('@anthropic-ai/claude-agent-sdk'),
  version: VersionSchema, claudeCodeVersion: VersionSchema,
});
export const BRIDGE_POLICY = {
  tools: 'closed-allowlist', permissions: 'default-host-once', mcp: 'host-discovery-only',
  configuration: 'isolated', authentication: 'api-console-cloud-only',
  dispatch: 'correlated-sdk-submission',
  usage: 'correlated-sdk-snapshots',
  file_preview: 'display-only-correlated-arguments-outcomes',
};
export const FRAME_LIMIT = 2 * 1024 * 1024;
export const READ_TOOLS = ['Read', 'Glob', 'Grep', 'AskUserQuestion'];
export const DISCOVERY_SERVER = 'eevee_source';
export const DISCOVERY_TOOL = 'mcp__eevee_source__discover';
export const DiscoverySchema = z.object({
  mode: z.enum(['glob', 'grep']),
  path: z.string().max(4096).default('.'),
  glob: z.string().min(1).max(512).default('**/*'),
  text: z.string().min(1).max(512).optional(),
}).strict();
export const DENIED_TOOLS = ['Bash', 'Agent', 'Task', 'TaskOutput', 'TaskStop', 'NotebookEdit', 'Skill', 'WebFetch', 'WebSearch', 'ToolSearch', 'EnterWorktree', 'ExitWorktree', 'EnterPlanMode', 'ExitPlanMode', 'CronCreate', 'CronDelete', 'CronList'];

const boundedText = (max: number) => z.string().min(1).max(max).refine(value =>
  !/[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(value), 'Display controls are forbidden');
// These are the official Options.effort wire values, not a model support catalogue.
const EffortSchema = z.enum(['low', 'medium', 'high', 'xhigh', 'max']);
export const SelectionSchema = z.object({
  model: boundedText(256).nullable(), effort: EffortSchema.nullable(),
}).strict();
export type Selection = z.infer<typeof SelectionSchema>;
const IdentitySchema = z.string().min(1).max(256);
export const WriteScopeSchema = z.object({
  mode: z.enum(['Selected', 'Workspace']),
  protected_paths: z.array(boundedText(4096)).max(256),
  excluded_paths: z.array(z.string().min(1).max(4096)).max(256),
  root_identity: IdentitySchema.nullable(),
  files: z.array(z.object({
    path: boundedText(4096), kind: z.enum(['Create', 'Replace']),
    identity: IdentitySchema.nullable(), sha256: z.string().regex(/^[a-f0-9]{64}$/).nullable(),
    working_content: z.string().max(256 * 1024).nullable(),
    ancestors: z.array(z.object({ path: z.string().max(4096), identity: IdentitySchema.nullable() }).strict()).max(256),
  }).strict()).max(256),
}).strict();
export type WriteScope = z.infer<typeof WriteScopeSchema>;
export const RunSchema = z.object({
  protocol: z.literal(1), bridge_revision: z.literal(9), type: z.literal('run'), generation: z.number().int().positive(), task_id: z.number().int().positive(),
  request_id: boundedText(256).nullish(),
  prompt: z.string().max(FRAME_LIMIT / 2), cwd: z.string().min(1).max(4096), state_dir: z.string().min(1).max(4096),
  operation: z.enum(['plan', 'implement']), write_scope: WriteScopeSchema,
  resume: z.string().uuid().nullish(), inspect_models: z.boolean(), selection: SelectionSchema,
}).strict().superRefine((command, context) => {
  const scope = command.write_scope;
  if ((command.operation === 'plan' && (scope.mode !== 'Selected' || scope.files.length !== 0)) ||
    (command.operation === 'implement' && (scope.root_identity === null || (scope.mode === 'Selected' ? scope.files.length === 0 : scope.files.length !== 0))) ||
    (!command.inspect_models && scope.root_identity === null) ||
    (command.inspect_models && (command.request_id != null || command.prompt !== '' || command.resume != null || scope.files.length !== 0 || command.operation !== 'plan'))) {
    context.addIssue({ code: 'custom', message: 'Invalid inspection/operation write scope' });
  }
});

export type FilePreviewFrame = {
  type: 'file_preview'; tool_id: string; tool: 'Write' | 'Edit';
  stage: 'start' | 'delta' | 'stop' | 'received' | 'review' | 'approved' | 'denied' | 'refused' | 'reported' | 'reported_failed' | 'unknown' | 'interrupted';
  fragment?: string;
};
const PreviewEventSchema = z.object({
  type: z.string(), index: z.number().int().nonnegative().optional(),
  message: z.object({ id: z.string() }).optional(),
  content_block: z.object({ type: z.string(), id: z.string().optional(), name: z.string().optional() }).optional(),
  delta: z.object({ type: z.string(), partial_json: z.string().optional() }).optional(),
});

// Display transport only: never enrolls a target, approves input or observes
// filesystem results. Retain correlation/byte counts, not growing arguments.
export class SDKFilePreviews {
  private message = '';
  private readonly blocks = new Map<number, string>();
  private readonly calls = new Map<string, { tool: 'Write' | 'Edit'; bytes: number; stopped: boolean; reviewed: boolean; terminal: boolean }>();
  private bytes = 0;
  private readonly enabled: boolean;
  constructor(enabled: boolean) { this.enabled = enabled; }
  private call(id: string, tool: string) {
    if (!this.enabled || !id || id.length > 256 || /[\p{Cc}]/u.test(id) || (tool !== 'Write' && tool !== 'Edit')) return undefined;
    const previous = this.calls.get(id);
    if (previous) return previous.tool === tool ? previous : undefined;
    if (this.calls.size >= 128) return undefined;
    const call = { tool, bytes: 0, stopped: false, reviewed: false, terminal: false };
    this.calls.set(id, call);
    return call;
  }
  stream(event: unknown): FilePreviewFrame[] {
    if (!this.enabled) return [];
    const parsed = PreviewEventSchema.safeParse(event);
    if (!parsed.success) return [];
    const value = parsed.data;
    if (value.type === 'message_start') {
      this.message = value.message?.id ?? '';
      this.blocks.clear();
      return [];
    }
    if (value.type === 'message_stop') {
      this.message = ''; this.blocks.clear(); return [];
    }
    if (!this.message || value.index === undefined) return [];
    if (value.type === 'content_block_start' && value.content_block?.type === 'tool_use') {
      const { id, name } = value.content_block;
      if (!id || !name || this.blocks.has(value.index) || this.calls.has(id)) return [];
      const call = this.call(id, name);
      if (!call) return [];
      this.blocks.set(value.index, id);
      return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage: 'start' }];
    }
    const id = this.blocks.get(value.index);
    const call = id === undefined ? undefined : this.calls.get(id);
    if (!id || !call || call.stopped || call.terminal) return [];
    if (value.type === 'content_block_delta' && value.delta?.type === 'input_json_delta' && value.delta.partial_json !== undefined) {
      const fragment = value.delta.partial_json;
      const size = Buffer.byteLength(fragment);
      if (size > FRAME_LIMIT / 4 || call.bytes + size > FRAME_LIMIT / 2 || this.bytes + size > 4 * FRAME_LIMIT) {
        call.stopped = true; call.terminal = true;
        return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage: 'interrupted' }];
      }
      call.bytes += size; this.bytes += size;
      return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage: 'delta', fragment }];
    }
    if (value.type === 'content_block_stop') {
      call.stopped = true;
      this.blocks.delete(value.index);
      return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage: 'stop' }];
    }
    return [];
  }
  complete(id: string, tool: string, input: unknown): FilePreviewFrame[] {
    const call = this.call(id, tool);
    if (!call || call.terminal || call.reviewed) return [];
    const object = z.object({
      file_path: z.string().max(4096),
      content: z.string().max(256 * 1024).optional(),
      new_string: z.string().max(256 * 1024).optional(),
    }).safeParse(input);
    if (!object.success) return [];
    // Complete callbacks can precede the assistant item. Send only display
    // fields, never old content, descriptions or arbitrary tool arguments.
    const fields = object.data;
    if (call.tool === 'Write' ? fields.content === undefined : fields.new_string === undefined) return [];
    const display = call.tool === 'Write'
      ? { file_path: fields.file_path, content: fields.content }
      : { file_path: fields.file_path, new_string: fields.new_string };
    const fragment = JSON.stringify(display);
    const size = Buffer.byteLength(fragment);
    if (size > FRAME_LIMIT / 2 || this.bytes + size > 4 * FRAME_LIMIT) return [];
    this.bytes += size;
    call.stopped = true;
    return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage: 'received', fragment }];
  }
  phase(id: string, stage: 'review' | 'approved' | 'denied' | 'refused' | 'reported' | 'reported_failed' | 'unknown' | 'interrupted'): FilePreviewFrame[] {
    const call = this.calls.get(id);
    if (!call || call.terminal) return [];
    if (stage === 'review') call.reviewed = true;
    if (stage !== 'review' && stage !== 'approved') call.terminal = true;
    return [{ type: 'file_preview', tool_id: id, tool: call.tool, stage }];
  }
}

const SubmissionAckSchema = z.object({
  type: z.string(), uuid: z.string().optional(), isReplay: z.boolean().optional(),
  user_message_uuid: z.string().optional(),
});

// Only a response tied to this fresh SDK message UUID proves query submission.
// Initialization, iterator release, bridge enqueue and restored history do not.
export class SDKDispatchReceipt {
  private submitted = false;
  private sent = false;
  private readonly uuid: string;
  private readonly requestId: string | null | undefined;
  constructor(uuid: string, requestId: string | null | undefined) {
    this.uuid = uuid;
    this.requestId = requestId;
  }
  submittedToSDK(): void { this.submitted = true; }
  observe(message: unknown): string | undefined {
    if (!this.submitted || this.sent || this.requestId == null) return undefined;
    const parsed = SubmissionAckSchema.safeParse(message);
    if (!parsed.success) return undefined;
    const value = parsed.data;
    const acknowledged = value.type === 'user' && value.isReplay === true && value.uuid === this.uuid
      || ['assistant', 'stream_event', 'result'].includes(value.type) && value.user_message_uuid === this.uuid;
    if (!acknowledged) return undefined;
    this.sent = true;
    return this.requestId;
  }
}

// SDK 0.3.300 sdk.d.ts: result.usage is main-loop/per-turn; modelUsage
// is cumulative query-pipeline coverage, including restored transcript totals.
// Neither includes out-of-pipeline helpers. No tokenizer is reported.
const CounterSchema = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER).nullish();
const MainUsageSchema = z.object({
  input_tokens: CounterSchema, output_tokens: CounterSchema,
  cache_read_input_tokens: CounterSchema, cache_creation_input_tokens: CounterSchema,
});
const ModelUsageSchema = z.object({
  inputTokens: CounterSchema, outputTokens: CounterSchema,
  cacheReadInputTokens: CounterSchema, cacheCreationInputTokens: CounterSchema,
  thinkingTokens: CounterSchema,
});
const ResultUsageSchema = z.object({
  type: z.literal('result'), uuid: boundedText(512), session_id: z.string().uuid(),
  user_message_uuid: z.string().optional(), isReplay: z.boolean().optional(),
  is_error: z.boolean(), subtype: boundedText(256),
  usage: MainUsageSchema.nullish(),
  modelUsage: z.record(boundedText(512), ModelUsageSchema).nullish(),
});
type UsageCounters = { input: number | null; output: number | null; cache_read: number | null; cache_write: number | null; reasoning: number | null };
type UsageReport = {
  identity: { session: string; turn: string; response: null; event: string };
  scope: 'MainLoopTurn' | 'ModelPipeline'; kind: 'Snapshot'; counters: UsageCounters;
  baseline: UsageCounters | null; model: string | null; tokenizer: null;
  semantics: { cache_read_in_input: false; cache_write_in_input: false; reasoning_in_output: true | null };
  complete: boolean;
};
type UsageEnvelope = { request_id: string; submission_id: string; session_id: string; report: UsageReport };

export class SDKUsageLedger {
  private active: { uuid: string; requestId: string; baseline: Map<string, UsageCounters> } | undefined;
  private readonly totals = new Map<string, UsageCounters>();
  private session: string | undefined;
  // Called at the actual SDKUserMessage yield, not at policy admission/enqueue.
  submitted(uuid: string, requestId: string | null | undefined): void {
    this.active = requestId == null ? undefined : { uuid, requestId, baseline: new Map(this.totals) };
  }
  observe(raw: unknown): UsageEnvelope[] {
    const parsed = ResultUsageSchema.safeParse(raw);
    if (!parsed.success || parsed.data.isReplay === true || !this.active ||
      parsed.data.user_message_uuid !== this.active.uuid) return [];
    const value = parsed.data;
    if (this.session !== undefined && this.session !== value.session_id) throw new Error('SDK usage changed session');
    this.session = value.session_id;
    const envelope = { request_id: this.active.requestId, submission_id: this.active.uuid, session_id: value.session_id };
    const identity = { session: value.session_id, turn: this.active.uuid, response: null, event: value.uuid };
    const complete = !value.is_error && value.subtype === 'success';
    const reports: UsageEnvelope[] = [];
    if (value.usage != null) reports.push({ ...envelope, report: {
      identity, scope: 'MainLoopTurn', kind: 'Snapshot',
      counters: { input: value.usage.input_tokens ?? null, output: value.usage.output_tokens ?? null,
        cache_read: value.usage.cache_read_input_tokens ?? null, cache_write: value.usage.cache_creation_input_tokens ?? null, reasoning: null },
      baseline: null, model: null, tokenizer: null,
      semantics: { cache_read_in_input: false, cache_write_in_input: false, reasoning_in_output: null }, complete,
    } });
    for (const [model, usage] of Object.entries(value.modelUsage ?? {})) {
      const counters = { input: usage.inputTokens ?? null, output: usage.outputTokens ?? null,
        cache_read: usage.cacheReadInputTokens ?? null, cache_write: usage.cacheCreationInputTokens ?? null, reasoning: usage.thinkingTokens ?? null };
      reports.push({ ...envelope, report: {
        identity, scope: 'ModelPipeline', kind: 'Snapshot', counters,
        baseline: this.active.baseline.get(model) ?? null, model, tokenizer: null,
        semantics: { cache_read_in_input: false, cache_write_in_input: false, reasoning_in_output: true }, complete,
      } });
      if (complete) {
        const previous = this.totals.get(model);
        this.totals.set(model, {
          input: counters.input ?? previous?.input ?? null, output: counters.output ?? previous?.output ?? null,
          cache_read: counters.cache_read ?? previous?.cache_read ?? null, cache_write: counters.cache_write ?? previous?.cache_write ?? null,
          reasoning: counters.reasoning ?? previous?.reasoning ?? null,
        });
      }
    }
    return reports;
  }
}
const ModelSchema = z.object({
  value: boundedText(256), resolvedModel: boundedText(256).optional(),
  displayName: boundedText(512).optional(), description: z.string().max(4096).refine(value =>
    !/[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(value)).optional().default(''),
  supportsEffort: z.boolean().optional(), supportedEffortLevels: z.array(EffortSchema).max(5).optional(),
});

export class SDKPolicyError extends Error {}

export function modelCatalogue(raw: ModelInfo[], selection: Selection) {
  const parsed = z.array(ModelSchema).max(4096).safeParse(raw);
  if (!parsed.success) throw new SDKPolicyError('Live SDK model catalogue unsupported: invalid bounds/schema/display text');
  const models = parsed.data;
  if (new Set(models.map(model => model.value)).size !== models.length) throw new SDKPolicyError('Duplicate SDK model identifiers');
  const selected = selection.model === null ? undefined :
    models.find(model => model.value === selection.model || model.resolvedModel === selection.model);
  return {
    models: models.map(model => ({
      id: model.value, name: model.displayName ?? model.value, description: model.description,
      efforts: model.supportsEffort ? model.supportedEffortLevels ?? [] : [],
      default_effort: null,
    })),
    current: selected?.value ?? selection.model ?? null,
    source: 'Claude Agent SDK supportedModels(); discovery is informational, not a request allowlist',
    effort_note: 'Effort choices are SDK-advertised request values. Applied effort is not reported; organization caps and model downgrades may apply.',
  };
}

export function querySelection(selection: Selection): { model?: string; effort?: EffortLevel } {
  return { model: selection.model ?? undefined, effort: selection.effort ?? undefined };
}

export function assertAppliedModel(expected: string | undefined, actual: unknown): void {
  if (expected !== undefined && actual !== expected) {
    throw new SDKPolicyError('SDK effective model differs from the reviewed selection or lacks an authoritative acknowledgement');
  }
}

export type SDKCompatibility = { sdkVersion: string; cliVersion: string; requireHooks: boolean };

export function assertInitialization(raw: unknown, compatibility: SDKCompatibility): void {
  const parsed = z.object({ claude_code_version: VersionSchema, hooks_applied: z.boolean().optional() }).safeParse(raw);
  if (!parsed.success || parsed.data.claude_code_version !== compatibility.cliVersion) {
    throw new Error('Effective SDK executable version differs from reviewed CLI or lacks identity acknowledgement');
  }
  if (compatibility.requireHooks && parsed.data.hooks_applied !== true) {
    throw new Error('SDK did not acknowledge mandatory tool policy hooks; no prompt sent');
  }
}

export function assertSessionPolicy(raw: unknown, cliVersion: string, tools: readonly string[]): void {
  const parsed = z.object({
    claude_code_version: VersionSchema, permissionMode: z.literal('default'),
    tools: z.array(z.string()).max(256),
    mcp_servers: z.array(z.object({ name: z.literal(DISCOVERY_SERVER), status: z.literal('connected') })).max(1),
    plugins: z.array(z.unknown()).max(0), skills: z.array(z.unknown()).max(0),
  }).safeParse(raw);
  if (!parsed.success || parsed.data.claude_code_version !== cliVersion ||
    parsed.data.tools.some(tool => !tools.includes(tool)) ||
    (tools.includes(DISCOVERY_TOOL) ? parsed.data.mcp_servers.length !== 1 || !parsed.data.tools.includes(DISCOVERY_TOOL) : parsed.data.mcp_servers.length !== 0)) {
    throw new Error('Effective SDK identity/tool/config policy mismatch');
  }
}

// This control call initializes the real SDK without releasing the prompt iterator.
// The caller may release it only after backend and model policy both succeed.
export async function prepareModels(
  sdk: { accountInfo(): Promise<{ apiProvider?: string; tokenSource?: string; apiKeySource?: string }>;
    initializationResult(): Promise<{ claude_code_version?: string; hooks_applied?: boolean }>; supportedModels(): Promise<ModelInfo[]>;
    setModel(model?: string): Promise<void>; getContextUsage(options: { detail: 'summary' }): Promise<{ model: string }> },
  selection: Selection, route: string, expectedProvider: string, compatibility: SDKCompatibility, inspectModels = false,
) {
  for (const name of ['accountInfo', 'initializationResult', ...(inspectModels ? ['supportedModels'] : []), ...(!inspectModels && selection.model !== null ? ['setModel', 'getContextUsage'] : [])] as const) {
    if (typeof sdk[name as keyof typeof sdk] !== 'function') throw new Error(`SDK required control unavailable: ${name}; no prompt sent`);
  }
  if (!accountMatches(route, await sdk.accountInfo(), expectedProvider)) throw new Error('Effective SDK authentication backend differs from reviewed route');
  assertInitialization(await sdk.initializationResult(), compatibility);
  let models: ModelInfo[] = [];
  let catalogue;
  try {
    models = await sdk.supportedModels();
    catalogue = modelCatalogue(models, selection);
    catalogue.source = `Claude Agent SDK ${compatibility.sdkVersion} supportedModels()`;
  } catch (error) {
    if (inspectModels) throw error;
    models = [];
    catalogue = modelCatalogue([], selection);
    catalogue.source = 'SDK discovery unavailable; the backend must acknowledge the exact requested model';
  }
  let approvedModel: string | undefined;
  if (!inspectModels && selection.model !== null) {
    const selected = models.find(model => model.value === selection.model || model.resolvedModel === selection.model);
    approvedModel = selected?.resolvedModel ?? selected?.value ?? selection.model;
    try {
      // Both are official streaming controls. Summary uses local estimates, not
      // per-category token-count requests or a model turn; no prompt is released.
      await sdk.setModel(selection.model);
      assertAppliedModel(approvedModel, (await sdk.getContextUsage({ detail: 'summary' })).model);
    } catch (error) {
      if (error instanceof SDKPolicyError) throw error;
      throw new SDKPolicyError('SDK model selection could not be acknowledged before prompt release; no prompt sent');
    }
  }
  return { catalogue, approvedModel };
}

// Mandatory tool exposure and PreToolUse checks are independent of canUseTool:
// provider-native/preapproved reads can bypass the approval callback.
// This is agent policy, NOT OS confinement or a race-free filesystem sandbox.
function exactRelative(path: string): boolean {
  return path.length > 0 && path.length <= 4096 && !isAbsolute(path) &&
    !/[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069\\:]/u.test(path) && path.split('/').every(part =>
      part.length > 0 && part !== '.' && part !== '..' && !/[. ]$/.test(part) &&
      !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part));
}
function exactExclusion(path: string): boolean {
  // Canonical host prefixes are not write targets: reserved or unwritable
  // components can exclude nothing, but must not disable the whole policy.
  return path.length > 0 && path.length <= 4096 && parse(path).root === '' &&
    path.split('/').every(part => part.length > 0 && part !== '.' && part !== '..');
}
function protectedPath(path: string): boolean {
  return path.split('/').some(part => {
    const name = part.toLowerCase();
    return ['.git', '.ssh', '.claude', '.codex', '.gemini', '.opencode', '.npmrc', '.netrc', '.pypirc', '.ds_store', '.ev-install', 'auth.json', 'oauth_creds.json'].includes(name) ||
      name === '.env' || name.startsWith('.env.') || name.startsWith('id_') || name.includes('secret') ||
      name.includes('credential') || name.startsWith('private_key') || name.endsWith('.pem') || name.endsWith('.key');
  });
}
function nativeIdentity(stat: BigIntStats): string {
  return process.platform === 'win32' ? `${stat.dev}:${stat.ino >> 32n}:${stat.ino & 0xffffffffn}` : `${stat.dev}:${stat.ino}`;
}
function directoryIdentity(path: string): string | undefined {
  try {
    const stat = lstatSync(path, { bigint: true });
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path) return undefined;
    return nativeIdentity(stat);
  } catch { return undefined; }
}
function absent(path: string): boolean {
  try { lstatSync(path); return false; }
  catch (error) { return !!error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT'; }
}
function fileState(path: string): { identity: string; sha256: string; content: string; revision: string } | undefined {
  try {
    const before = lstatSync(path, { bigint: true });
    if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n || before.size > 256n * 1024n || realpathSync(path) !== path) return undefined;
    const bytes = readFileSync(path);
    const after = lstatSync(path, { bigint: true });
    if (nativeIdentity(before) !== nativeIdentity(after) || before.size !== after.size ||
      before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs || after.nlink !== 1n) return undefined;
    const content = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
    return { identity: nativeIdentity(after), sha256: createHash('sha256').update(bytes).digest('hex'), content,
      revision: `${after.size}:${after.mtimeNs}:${after.ctimeNs}` };
  } catch { return undefined; }
}

// Workspace authority freezes the root at Run and targets only at valid proposal.
// A previously enrolled target is never recaptured to refresh stale authority.
export class SDKWritePolicy {
  private readonly root: string;
  private readonly scope: WriteScope;
  private readonly ownedParents = new Map<string, string>();
  private readonly readRootIdentity: string;
  private readonly protectedPaths: string[];
  private readonly ancestorLedger = new Map<string, string | null>();
  private uncertain = false;
  private readonly seen = new Set<string>();
  private readonly denied = new Map<string, { tool: string; input: unknown }>();
  private active: { id: string; tool: string; input: Record<string, unknown>; path: string; fingerprint: string; expected: string; approved: boolean } | undefined;
  constructor(cwd: string, scope: WriteScope, runtimePaths: string[] = []) {
    this.root = realpathSync(cwd);
    if (toNamespacedPath(resolve(cwd)) !== toNamespacedPath(this.root) || directoryIdentity(this.root) === undefined) throw new SDKPolicyError('SDK cwd must be a canonical non-link directory');
    // Built-in boundaries belong to the absolute source location, not merely
    // the relative child name. Choosing a protected subtree cannot erase them.
    if (protectedPath(this.root.split(sep).join('/'))) throw new SDKPolicyError('SDK cwd is inside a protected credential/private directory');
    this.readRootIdentity = directoryIdentity(this.root)!;
    this.scope = structuredClone(WriteScopeSchema.parse(scope));
    if (this.scope.root_identity !== null && directoryIdentity(this.root) !== this.scope.root_identity) throw new SDKPolicyError('SDK frozen root changed');
    if (this.scope.mode === 'Workspace' && (this.scope.root_identity === null || this.scope.files.length !== 0)) throw new SDKPolicyError('Invalid initial workspace scope');
    if (this.scope.excluded_paths.some(path => !exactExclusion(path)) || this.scope.protected_paths.some(path => !isAbsolute(path))) throw new SDKPolicyError('Invalid SDK boundaries');
    this.protectedPaths = [...this.scope.protected_paths, realpathSync(process.execPath), ...runtimePaths].map(path => realpathSync(path));
    if (new Set(this.scope.files.map(file => file.path)).size !== this.scope.files.length) throw new SDKPolicyError('Duplicate selected SDK paths');
    for (const file of this.scope.files) {
      const parts = file.path.split('/');
      const expected = parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join('/'));
      if (this.path(file.path) !== file.path || file.ancestors.length !== expected.length ||
        file.ancestors.some((ancestor, i) => ancestor.path !== expected[i]) ||
        (file.kind === 'Create' ? file.identity !== null || file.sha256 !== null || file.working_content !== null :
          file.identity === null || file.sha256 === null || file.working_content === null ||
          Buffer.byteLength(file.working_content) > 256 * 1024 ||
          createHash('sha256').update(file.working_content).digest('hex') !== file.sha256)) throw new SDKPolicyError('Invalid frozen SDK scope');
      for (const ancestor of file.ancestors) {
        if (this.ancestorLedger.has(ancestor.path) && this.ancestorLedger.get(ancestor.path) !== ancestor.identity) throw new SDKPolicyError('Inconsistent frozen SDK parents');
        this.ancestorLedger.set(ancestor.path, ancestor.identity);
      }
    }
  }
  private path(candidate: unknown): string | undefined {
    if (typeof candidate !== 'string' || candidate.length > 4096 ||
      candidate.split(sep).join('/').split('/').some(part => part === '.' || part === '..')) return undefined;
    const path = isAbsolute(candidate) ? relative(this.root, candidate).split(sep).join('/') : candidate;
    if (!exactRelative(path) || protectedPath(path)) return undefined;
    const full = resolve(this.root, path);
    const within = (target: string, boundary: string) => {
      const parts = target.normalize('NFC').toUpperCase().split(sep);
      const prefix = boundary.normalize('NFC').toUpperCase().split(sep);
      return prefix.every((part, i) => parts[i] === part);
    };
    return this.protectedPaths.some(boundary => within(full, boundary)) ||
      this.scope.excluded_paths.some(boundary => within(full, resolve(this.root, boundary))) ? undefined : path;
  }
  private unsafeContent(text: string): boolean {
    return /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(text) ||
      /-----BEGIN [^\r\n]*PRIVATE KEY-----|(?:^|[^A-Za-z0-9_])(?:sk-|gh[pousr]_|github_pat_|xox[bpar]-|AKIA|ASIA|AIza)[^\s"'`,;<>]{4,}|bearer[ \t]+[^\s"'`,;<>]+|(?:[A-Za-z0-9_-]+_)?(?:password|passwd|pwd|secret|token|api[_-]?key|access_key|private_key|client_secret|authorization)["']?\s*[:=]\s*["']?[^\s"',;}\]]+|:\/\/[^\s/?#]+@/im.test(text);
  }
  // The SDK's in-process MCP handler returns only host-filtered observations.
  // Native recursive tools retain their stricter whole-subtree admission below.
  discover(raw: unknown): { paths: string[]; matches: { path: string; line: number; text: string }[]; truncated: boolean } {
    const input = DiscoverySchema.parse(raw);
    if (directoryIdentity(this.root) !== this.readRootIdentity ||
      (input.mode === 'grep' && input.text === undefined) ||
      isAbsolute(input.glob) || /[\\:{}()!|\[\]\p{Cc}]/u.test(input.glob) ||
      input.glob.split('/').some(part => part === '.' || part.includes('..'))) {
      throw new SDKPolicyError('Discovery requires a current root, simple relative glob and literal grep text');
    }
    const path = input.path === '.' || input.path === this.root ? '' : this.path(input.path);
    if (path === undefined || (path && !path.split('/').slice(0, -1).every((_, index) =>
      directoryIdentity(resolve(this.root, ...path.split('/').slice(0, index + 1))) !== undefined))) {
      throw new SDKPolicyError('Discovery path is unavailable');
    }
    const result: { paths: string[]; matches: { path: string; line: number; text: string }[]; truncated: boolean } =
      { paths: [], matches: [], truncated: false };
    const pending = [{ path, depth: 0 }];
    const directories: { path: string; identity: string }[] = [];
    let entries = 0;
    let bytes = 0;
    let output = Buffer.byteLength(JSON.stringify(result));
    scan: while (pending.length) {
      const current = pending.pop()!;
      if (++entries > 10000 || current.depth > 64) { result.truncated = true; break; }
      if (current.path && this.path(current.path) !== current.path) continue;
      const full = resolve(this.root, current.path);
      const identity = directoryIdentity(full);
      if (identity !== undefined) {
        if (entries + pending.length >= 10000) { result.truncated = true; break; }
        directories.push({ path: full, identity });
        const names: string[] = [];
        try {
          const directory = opendirSync(full);
          try {
            for (let entry; (entry = directory.readSync()) !== null;) {
              if (entries + pending.length + names.length >= 10000) { result.truncated = true; break scan; }
              names.push(entry.name);
            }
          } finally { directory.closeSync(); }
        } catch (error) {
          // An inaccessible descendant is incomplete coverage, not a failure of
          // unrelated safe siblings. Requested-root and identity failures still
          // fail closed, and partial enumeration never publishes child names.
          if (current.path === path || !error || typeof error !== 'object' ||
            !('code' in error) || !['EACCES', 'EPERM'].includes(String(error.code)) ||
            directoryIdentity(full) !== identity || directoryIdentity(this.root) !== this.readRootIdentity) throw error;
          result.truncated = true;
          continue;
        }
        for (const name of names.sort().reverse()) {
          const child = current.path ? `${current.path}/${name}` : name;
          if (this.path(child) === child) pending.push({ path: child, depth: current.depth + 1 });
        }
        continue;
      }
      const candidate = path === current.path ? current.path.split('/').at(-1)! :
        path ? current.path.slice(path.length + 1) : current.path;
      if (!matchesGlob(candidate, input.glob)) continue;
      // Reuse the same file snapshot and whole-file privacy rules as Read.
      // Count inspected bytes even when privacy excludes the result.
      let size: number;
      try { size = Number(lstatSync(full).size); } catch { continue; }
      if (size > 256 * 1024) continue;
      if (bytes + size > 2 * 1024 * 1024) { result.truncated = true; break; }
      bytes += size;
      const parts = current.path.split('/');
      if (!parts.slice(0, -1).every((_, index) =>
        directoryIdentity(resolve(this.root, ...parts.slice(0, index + 1))) !== undefined)) continue;
      const state = fileState(full);
      if (!state || this.unsafeContent(state.content)) continue;
      if (input.mode === 'glob') {
        output += Buffer.byteLength(JSON.stringify(current.path)) + 1;
        if (output > 64 * 1024 || result.paths.length >= 128) { result.truncated = true; break; }
        result.paths.push(current.path);
      } else {
        const lines = state.content.split('\n');
        for (let index = 0; index < lines.length; index++) {
          if (!lines[index].includes(input.text!)) continue;
          const match = { path: current.path, line: index + 1, text: lines[index] };
          output += Buffer.byteLength(JSON.stringify(match)) + 1;
          if (output > 64 * 1024 || result.matches.length >= 128) { result.truncated = true; break scan; }
          result.matches.push(match);
        }
      }
    }
    if (directoryIdentity(this.root) !== this.readRootIdentity ||
      directories.some(directory => directoryIdentity(directory.path) !== directory.identity)) {
      throw new SDKPolicyError('Discovery tree changed; request a fresh bounded observation');
    }
    return result;
  }
  // Observations never enroll mutation targets or refresh their baselines.
  // This is mandatory agent policy, not a race-free OS filesystem sandbox.
  readAdmission(tool: string, input: Record<string, unknown>): { allowed: boolean; reason: string } {
    const refusal = (reason: string) => ({ allowed: false, reason });
    if (!['Read', 'Glob', 'Grep'].includes(tool) || directoryIdentity(this.root) !== this.readRootIdentity) {
      return refusal('Source tool or frozen project root is unavailable');
    }
    const safeFile = (path: string) => {
      const state = fileState(resolve(this.root, path));
      return state !== undefined && !this.unsafeContent(state.content);
    };
    const ancestors = (path: string) => {
      const parts = path.split('/');
      return parts.slice(0, -1).every((_, index) =>
        directoryIdentity(resolve(this.root, ...parts.slice(0, index + 1))) !== undefined);
    };
    if (tool === 'Read') {
      const path = this.path(input.file_path);
      return path !== undefined && ancestors(path) && safeFile(path) &&
        directoryIdentity(this.root) === this.readRootIdentity ?
        { allowed: true, reason: 'Bounded safe project source; read permission only' } :
        refusal('Read requires an existing safe UTF-8 project file (256 KiB maximum), with no links, excluded/protected paths or recognizable credentials; use a literal project path');
    }
    const pattern = tool === 'Glob' ? input.pattern : input.glob;
    if ((tool === 'Glob' && typeof pattern !== 'string') ||
      (pattern !== undefined && (typeof pattern !== 'string' || pattern.length > 4096 || isAbsolute(pattern) ||
        /[\\:{}()!|\[\]\p{Cc}]/u.test(pattern) || pattern.split('/').some(part => part === '.' || part.includes('..'))))) {
      return refusal('Discovery supports simple project-relative * and ? patterns, without parent traversal or alternate absolute paths');
    }
    const candidate = input.path ?? this.root;
    if (typeof candidate !== 'string') return refusal('Source search requires a literal project path');
    const rootSearch = candidate === this.root || candidate === '.';
    const path = rootSearch ? '' : this.path(candidate);
    if (path === undefined || (path !== '' && !ancestors(path))) {
      return refusal('Source search path is outside, protected, excluded or linked; choose a safe project file/directory');
    }
    const full = resolve(this.root, path);
    if (directoryIdentity(full) === undefined) {
      return path !== '' && safeFile(path) ? { allowed: true, reason: 'Safe exact source file search' } :
        refusal('Source search target must be a safe regular file or directory');
    }
    const pending = [{ path, identity: directoryIdentity(full)!, depth: 0 }];
    const retained: { path: string; identity: string }[] = [];
    const catalogue: string[] = [];
    let blocked = false;
    let entries = 0;
    let bytes = 0;
    while (pending.length) {
      const directory = pending.pop()!;
      if (directory.depth > 64 || directoryIdentity(resolve(this.root, directory.path)) !== directory.identity) {
        return refusal('Source search tree changed or exceeds 64 levels; narrow the search');
      }
      let names: string[];
      try { names = readdirSync(resolve(this.root, directory.path)).sort(); }
      catch { return refusal('Source search directory is unavailable'); }
      entries += names.length;
      if (entries > 10000) return refusal('Source discovery exceeds 10000 entries; choose a narrower safe directory');
      for (const name of names) {
        const child = directory.path ? `${directory.path}/${name}` : name;
        if (this.path(child) !== child) { blocked = true; continue; }
        const identity = directoryIdentity(resolve(this.root, child));
        if (identity !== undefined) {
          pending.push({ path: child, identity, depth: directory.depth + 1 });
        } else {
          const state = fileState(resolve(this.root, child));
          if (!state || this.unsafeContent(state.content)) { blocked = true; continue; }
          bytes += Buffer.byteLength(state.content);
          if (bytes > 2 * 1024 * 1024) return refusal('Source search exceeds 2 MiB of bounded source; narrow the search');
          if (catalogue.length < 128) catalogue.push(child);
        }
      }
      retained.push(directory);
    }
    if (retained.some(directory => directoryIdentity(resolve(this.root, directory.path)) !== directory.identity) ||
      directoryIdentity(this.root) !== this.readRootIdentity) return refusal('Source search tree changed; try a fresh narrower search');
    if (blocked) return refusal(`Recursive native source search could include protected/excluded/linked/credential-bearing files. Use Read on a safe filename or search a narrower safe directory. Actual bounded safe filename catalogue (up to 128): ${JSON.stringify(catalogue.sort())}`);
    return { allowed: true, reason: 'Bounded safe source tree; no mutation authority' };
  }
  private capture(path: string): WriteScope['files'][number] | undefined {
    if (this.scope.mode !== 'Workspace' || this.scope.files.length >= 256 || directoryIdentity(this.root) !== this.scope.root_identity) return undefined;
    const key = (value: string) => value.normalize('NFC').toUpperCase();
    const candidate = path.split('/');
    if (this.scope.files.some(file => {
      const existing = file.path.split('/');
      for (let i = 0; i < Math.min(candidate.length, existing.length); i++) {
        if (key(candidate[i]) !== key(existing[i])) return false;
        if (candidate[i] !== existing[i]) return true;
      }
      return candidate.length !== existing.length;
    })) return undefined;
    const ancestors: WriteScope['files'][number]['ancestors'] = [];
    const parts = path.split('/');
    let missing = false;
    for (let i = 0; i < parts.length; i++) {
      const parent = parts.slice(0, i).join('/');
      const current = parts.slice(0, i + 1).join('/');
      if (!missing) {
        // Conservatively refuse case/normalization aliases on every filesystem.
        try {
          const key = parts[i].normalize('NFC').toUpperCase();
          if (readdirSync(resolve(this.root, parent)).some(name => name !== parts[i] && name.normalize('NFC').toUpperCase() === key)) return undefined;
        } catch { return undefined; }
      }
      if (i === parts.length - 1) break;
      const full = resolve(this.root, current);
      const identity = directoryIdentity(full);
      if (identity === undefined && !absent(full)) return undefined;
      if (missing && identity !== undefined) return undefined;
      const frozen = this.ancestorLedger.get(current);
      const expected = frozen ?? this.ownedParents.get(current);
      if (this.ancestorLedger.has(current) && (expected === undefined ? identity !== undefined : identity !== expected)) return undefined;
      ancestors.push({ path: current, identity: identity ?? null });
      missing ||= identity === undefined;
    }
    const full = resolve(this.root, path);
    if (absent(full)) return { path, kind: 'Create', identity: null, sha256: null, working_content: null, ancestors };
    if (missing) return undefined;
    const state = fileState(full);
    return state ? { path, kind: 'Replace', identity: state.identity, sha256: state.sha256, working_content: state.content, ancestors } : undefined;
  }
  private expected(tool: string, input: Record<string, unknown>, file: WriteScope['files'][number]): string | undefined {
    if (tool === 'Write') return typeof input.content === 'string' && Buffer.byteLength(input.content) <= 256 * 1024 ? input.content : undefined;
    if (tool !== 'Edit' || typeof input.old_string !== 'string' || typeof input.new_string !== 'string' || !input.old_string ||
      (input.replace_all !== undefined && typeof input.replace_all !== 'boolean') || file.working_content === null) return undefined;
    const occurrences = file.working_content.split(input.old_string).length - 1;
    if (occurrences === 0 || (input.replace_all !== true && occurrences !== 1)) return undefined;
    const content = input.replace_all === true ? file.working_content.split(input.old_string).join(input.new_string) :
      file.working_content.replace(input.old_string, () => input.new_string as string);
    return Buffer.byteLength(content) <= 256 * 1024 ? content : undefined;
  }
  private fingerprint(candidate: unknown): string | undefined {
    try {
      const path = this.path(candidate);
      const file = this.scope.files.find(file => file.path === path);
      if (!file || directoryIdentity(this.root) !== this.scope.root_identity) return undefined;
      for (const ancestor of file.ancestors) {
        const full = resolve(this.root, ancestor.path);
        const identity = ancestor.identity ?? this.ownedParents.get(ancestor.path);
        if (identity === undefined ? !absent(full) : directoryIdentity(full) !== identity) return undefined;
      }
      const full = resolve(this.root, file.path);
      if (file.kind === 'Create') return absent(full) ? JSON.stringify(file) : undefined;
      const state = fileState(full);
      return state && state.identity === file.identity && state.sha256 === file.sha256 ? JSON.stringify({ file, revision: state.revision }) : undefined;
    } catch { return undefined; }
  }
  // Mandatory PreToolUse independently enforces scope, stale state and tool IDs.
  preToolUse(id: string, tool: string, input: Record<string, unknown>): boolean {
    if (this.uncertain || (tool !== 'Write' && tool !== 'Edit') || (!this.active && this.seen.has(id))) return false;
    const path = this.path(input.file_path);
    if (path === undefined) return false;
    const frozen = this.scope.files.find(file => file.path === path);
    const file = frozen ?? this.capture(path);
    if (!file || (file.working_content !== null && this.unsafeContent(file.working_content)) || this.expected(tool, input, file) === undefined) return false;
    if (!frozen) return this.active === undefined;
    const fingerprint = this.fingerprint(path);
    return fingerprint !== undefined &&
      (!this.active || this.active.id === id && this.active.tool === tool && isDeepStrictEqual(this.active.input, input) && this.active.fingerprint === fingerprint);
  }
  begin(id: string, tool: string, input: Record<string, unknown>): { kind: string; path: string; directories: string[] } | undefined {
    if (this.active || this.seen.has(id) || this.seen.size >= 256 || !this.preToolUse(id, tool, input)) return undefined;
    const path = this.path(input.file_path)!;
    let file = this.scope.files.find(file => file.path === path);
    if (!file) {
      file = this.capture(path);
      if (!file || this.expected(tool, input, file) === undefined) return undefined;
      // Commit the whole capture only after all proposal validation succeeds.
      this.scope.files.push(file);
      for (const ancestor of file.ancestors) if (!this.ancestorLedger.has(ancestor.path)) this.ancestorLedger.set(ancestor.path, ancestor.identity);
    }
    const fingerprint = this.fingerprint(path);
    const expected = this.expected(tool, input, file);
    if (fingerprint === undefined || expected === undefined) return undefined;
    this.seen.add(id);
    this.active = { id, tool, input: structuredClone(input), path, fingerprint, expected, approved: false };
    return { kind: file.kind, path, directories: file.ancestors.filter(parent => parent.identity === null && !this.ownedParents.has(parent.path)).map(parent => parent.path) };
  }
  // Record only a fresh, pre-execution refusal. Never let a duplicate hook,
  // changed input or later denial overwrite an active/previous write's effects.
  refuse(id: string, tool: string, input: unknown): boolean {
    if ((tool !== 'Write' && tool !== 'Edit') || !id || id.length > 256 ||
      /[\p{Cc}]/u.test(id) || this.active?.id === id) return false;
    const previous = this.denied.get(id);
    if (previous) return previous.tool === tool && isDeepStrictEqual(previous.input, input);
    if (this.seen.has(id) || this.seen.size >= 256 ||
      Buffer.byteLength(JSON.stringify(input) ?? 'null') > FRAME_LIMIT / 2) return false;
    this.seen.add(id);
    this.denied.set(id, { tool, input: structuredClone(input) });
    return true;
  }
  approve(id: string, tool: string, input: Record<string, unknown>, allowed: boolean, cancelled: boolean): boolean {
    const active = this.active;
    if (!active || active.id !== id || active.approved) return false;
    const accepted = !cancelled && allowed && this.preToolUse(id, tool, input);
    if (accepted) active.approved = true;
    else { this.denied.set(id, { tool: active.tool, input: active.input }); this.active = undefined; }
    return accepted;
  }
  finish(id: string, tool: string, input: unknown, success: boolean, replay = false): boolean {
    const active = this.active;
    if (!active || active.id !== id) {
      const denied = this.denied.get(id);
      if (!success && !replay && denied?.tool === tool && isDeepStrictEqual(denied.input, input)) {
        this.denied.delete(id); return true;
      }
      return false;
    }
    if (active.tool !== tool || !isDeepStrictEqual(active.input, input) || !active.approved || replay) return false;
    this.active = undefined;
    // A failed SDK call can still create parents or partially replace the leaf.
    // Only a fresh, unambiguous observation of the original baseline settles it;
    // failure never advances either the file baseline or sibling-parent ledger.
    if (!success) {
      const unchanged = this.fingerprint(active.path) === active.fingerprint;
      this.uncertain ||= !unchanged;
      return unchanged;
    }
    const file = this.scope.files.find(file => file.path === active.path)!;
    if (directoryIdentity(this.root) !== this.scope.root_identity) return false;
    const parents = new Map<string, string>();
    for (const ancestor of file.ancestors) {
      const identity = directoryIdentity(resolve(this.root, ancestor.path));
      const expected = ancestor.identity ?? this.ownedParents.get(ancestor.path);
      if (identity === undefined || (expected !== undefined && identity !== expected)) return false;
      if (expected === undefined) parents.set(ancestor.path, identity);
    }
    const state = fileState(resolve(this.root, file.path));
    // Advance only after a correlated successful result and exact approved bytes.
    if (!state || state.content !== active.expected || (tool === 'Edit' && state.identity !== file.identity)) return false;
    for (const [path, identity] of parents) this.ownedParents.set(path, identity);
    file.kind = 'Replace'; file.identity = state.identity; file.sha256 = state.sha256; file.working_content = state.content;
    return true;
  }
}

export const QuestionInputSchema = z.object({
  questions: z.array(z.object({
    question: z.string().min(1).max(4096),
    header: z.string().max(256),
    multiSelect: z.literal(false).optional(),
    options: z.array(z.object({ label: z.string().max(1024), description: z.string().max(4096) })).max(16),
  })).min(1).max(8),
}).refine(input => new Set(input.questions.map(q => q.question)).size === input.questions.length);

export function questionSchema(input: Record<string, unknown>): boolean {
  return QuestionInputSchema.safeParse(input).success;
}

export const CONSOLE_PROFILE = 'eevee';
export type ConsoleProfile = { dir: string; name: string };

// `ant auth login` (Anthropic Console OAuth, API billing) profile resolution, mirroring
// the ant CLI. Metadata only: credential contents are never read here.
export function consoleProfile(env: NodeJS.ProcessEnv, platform: NodeJS.Platform = process.platform): ConsoleProfile | undefined {
  const name = env.ANTHROPIC_PROFILE?.trim() || CONSOLE_PROFILE;
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(name)) return undefined;
  const base = env.ANTHROPIC_CONFIG_DIR?.trim() ||
    (platform === 'win32' && env.APPDATA ? join(env.APPDATA, 'Anthropic') : undefined) ||
    (env.XDG_CONFIG_HOME ? join(env.XDG_CONFIG_HOME, 'anthropic') : undefined) ||
    (env.HOME ? join(env.HOME, '.config', 'anthropic') : undefined);
  if (!base || !isAbsolute(base)) return undefined;
  const regular = (path: string) => { try { return lstatSync(path).isFile(); } catch { return false; } };
  if (!regular(join(base, 'configs', `${name}.json`)) || !regular(join(base, 'credentials', `${name}.json`))) return undefined;
  return { dir: realpathSync(base), name };
}

export function credentialRoute(env: NodeJS.ProcessEnv, profile = consoleProfile(env)): string | undefined {
  const api = Boolean(env.ANTHROPIC_API_KEY);
  const bedrock = env.CLAUDE_CODE_USE_BEDROCK === '1';
  const vertex = env.CLAUDE_CODE_USE_VERTEX === '1';
  // An explicitly selected profile that is not logged in never falls through to another route.
  if (env.ANTHROPIC_PROFILE?.trim() && !profile) return undefined;
  if (Number(api) + Number(bedrock) + Number(vertex) + Number(Boolean(profile)) !== 1) return undefined;
  const chain = env.EEVEE_CLAUDE_CLOUD_AUTH === 'provider-chain';
  if ((env.EEVEE_CLAUDE_CLOUD_AUTH && !chain) || (chain && (api || profile))) return undefined;
  if (profile) return `Anthropic Console OAuth profile ${profile.name} (ant auth login)`;
  if (!chain && env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES === '1') return undefined;
  if (bedrock) {
    if (chain) return 'Amazon Bedrock (explicit provider credential chain)';
    if (env.AWS_BEARER_TOKEN_BEDROCK) return 'Amazon Bedrock (provider-owned bearer credential)';
    if (env.AWS_PROFILE || env.AWS_DEFAULT_PROFILE || env.AWS_CONFIG_FILE || env.AWS_SHARED_CREDENTIALS_FILE) return undefined;
    if (env.AWS_ACCESS_KEY_ID && env.AWS_SECRET_ACCESS_KEY) return 'Amazon Bedrock (provider-owned AWS credentials)';
    if (env.AWS_WEB_IDENTITY_TOKEN_FILE && env.AWS_ROLE_ARN) return 'Amazon Bedrock (provider-owned web identity)';
    return undefined;
  }
  if (vertex) {
    if (!env.ANTHROPIC_VERTEX_PROJECT_ID) return undefined;
    if (chain) return 'Google Vertex (explicit provider ADC credential chain)';
    return env.GOOGLE_APPLICATION_CREDENTIALS ? 'Google Vertex (provider-owned cloud credentials)' : undefined;
  }
  return 'ANTHROPIC_API_KEY';
}

// The supported profile route reports no token/API-key source; an API key,
// subscription login or missing profile reports one, so the prompt is never released to it.
export function accountMatches(route: string, info: { apiProvider?: string; tokenSource?: string; apiKeySource?: string }, expectedProvider: string): boolean {
  if (info.apiProvider !== expectedProvider) return false;
  if (route.startsWith('Anthropic Console OAuth profile')) return info.tokenSource === undefined && info.apiKeySource === undefined;
  return true;
}

// Interrupt acknowledgment is not a cancelled-result discriminator in this SDK.
// Preserve an authoritative success; an error after interruption remains ambiguous.
export function sdkOutcome(cancelRequested: boolean, isError: boolean, subtype: string): 'completed' | 'failed' | 'unknown' {
  if (!isError && subtype === 'success') return 'completed';
  return cancelRequested ? 'unknown' : 'failed';
}
