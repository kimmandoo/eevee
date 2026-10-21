import { z } from 'zod';
import { lstatSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import type { EffortLevel, ModelInfo } from '@anthropic-ai/claude-agent-sdk';

// Versions are observed identity, never a release allowlist.
const VersionSchema = z.string().max(128).regex(/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)*$/).refine(value => value.trim() === value);
export const SDKPackageSchema = z.object({
  name: z.literal('@anthropic-ai/claude-agent-sdk'),
  version: VersionSchema, claudeCodeVersion: VersionSchema,
});
export const BRIDGE_POLICY = {
  tools: 'closed-allowlist', permissions: 'default-host-once', mcp: 'disabled',
  configuration: 'isolated', authentication: 'api-console-cloud-only',
  dispatch: 'correlated-sdk-submission',
  usage: 'correlated-sdk-snapshots',
};
export const FRAME_LIMIT = 2 * 1024 * 1024;
export const READ_TOOLS = ['Read', 'Glob', 'Grep', 'AskUserQuestion'];
export const DENIED_TOOLS = ['Bash', 'Agent', 'Task', 'TaskOutput', 'TaskStop', 'NotebookEdit', 'Skill', 'WebFetch', 'WebSearch', 'ToolSearch', 'EnterWorktree', 'ExitWorktree', 'EnterPlanMode', 'ExitPlanMode', 'CronCreate', 'CronDelete', 'CronList', 'mcp__*'];

const boundedText = (max: number) => z.string().min(1).max(max).refine(value =>
  !/[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(value), 'Display controls are forbidden');
// These are the official Options.effort wire values, not a model support catalogue.
const EffortSchema = z.enum(['low', 'medium', 'high', 'xhigh', 'max']);
export const SelectionSchema = z.object({
  model: boundedText(256).nullable(), effort: EffortSchema.nullable(),
}).strict();
export type Selection = z.infer<typeof SelectionSchema>;
export const RunSchema = z.object({
  protocol: z.literal(1), type: z.literal('run'), generation: z.number().int().positive(), task_id: z.number().int().positive(),
  request_id: boundedText(256).nullish(),
  prompt: z.string().max(FRAME_LIMIT / 2), cwd: z.string().min(1).max(4096), state_dir: z.string().min(1).max(4096),
  operation: z.enum(['plan', 'implement']), edit_paths: z.array(z.string().min(1).max(4096)).max(256),
  resume: z.string().uuid().nullish(), inspect_models: z.boolean(), selection: SelectionSchema,
}).strict().superRefine((command, context) => {
  if (command.inspect_models && (command.request_id != null || command.prompt !== '' || command.resume != null || command.edit_paths.length !== 0 || command.operation !== 'plan')) {
    context.addIssue({ code: 'custom', message: 'Model inspection accepts no prompt, session or write scope' });
  }
});

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
    tools: z.array(z.string()).max(256), mcp_servers: z.array(z.unknown()).max(0),
    plugins: z.array(z.unknown()).max(0), skills: z.array(z.unknown()).max(0),
  }).safeParse(raw);
  if (!parsed.success || parsed.data.claude_code_version !== cliVersion ||
    parsed.data.tools.some(tool => !tools.includes(tool))) {
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
export function selectedWrite(cwd: string, paths: string[], candidate: unknown): boolean {
  if (typeof candidate !== 'string' || candidate.length > 4096) return false;
  const root = realpathSync(cwd);
  const target = resolve(root, candidate);
  const rel = relative(root, target);
  if (!rel || isAbsolute(rel) || rel === '..' || rel.startsWith(`..${sep}`)) return false;
  if (!paths.some(p => !isAbsolute(p) && resolve(root, p) === target)) return false;
  let current = root;
  const parts = rel.split(sep);
  const protectedNames: Record<string, true> = { '.git': true, '.claude': true, '.codex': true, '.gemini': true, '.opencode': true, '.npmrc': true, '.netrc': true, '.pypirc': true };
  if (parts.some(part => {
    const name = part.toLowerCase();
    return protectedNames[name] || name === '.env' || name.startsWith('.env.') || name.startsWith('id_') ||
      name.includes('secret') || name.includes('credential') || name.startsWith('private_key') || name.endsWith('.pem') || name.endsWith('.key');
  })) return false;
  for (let i = 0; i < parts.length; i++) {
    current = resolve(current, parts[i]);
    try {
      const metadata = lstatSync(current);
      if (metadata.isSymbolicLink() || (i === parts.length - 1 && (!metadata.isFile() || metadata.nlink !== 1))) return false;
    }
    catch (error) {
      if (!error || typeof error !== 'object' || !('code' in error) || error.code !== 'ENOENT' || i !== parts.length - 1) return false;
      return realpathSync(dirname(target)) === dirname(target);
    }
  }
  return realpathSync(target) === target;
}

// Recheck after the user has reviewed a write. Metadata identity is a stale-consent
// guard, not an atomic filesystem sandbox; PreToolUse independently checks scope.
export function selectedWriteIdentity(cwd: string, paths: string[], candidate: unknown): string | undefined {
  try {
    if (!selectedWrite(cwd, paths, candidate) || typeof candidate !== 'string') return undefined;
    const target = resolve(realpathSync(cwd), candidate);
    try {
      const stat = lstatSync(target, { bigint: true });
      return `file:${stat.dev}:${stat.ino}:${stat.size}:${stat.mtimeNs}:${stat.ctimeNs}`;
    } catch (error) {
      if (!error || typeof error !== 'object' || !('code' in error) || error.code !== 'ENOENT') return undefined;
      const stat = lstatSync(dirname(target), { bigint: true });
      return `new:${stat.dev}:${stat.ino}:${stat.mtimeNs}:${stat.ctimeNs}`;
    }
  } catch { return undefined; }
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
