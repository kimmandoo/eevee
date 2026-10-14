import { z } from 'zod';
import { lstatSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import type { EffortLevel, ModelInfo } from '@anthropic-ai/claude-agent-sdk';

export const SDK_VERSION = '0.3.296';
export const CLI_VERSION = '2.1.296';
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
  prompt: z.string().max(FRAME_LIMIT / 2), cwd: z.string().min(1).max(4096), state_dir: z.string().min(1).max(4096),
  operation: z.enum(['plan', 'implement']), edit_paths: z.array(z.string().min(1).max(4096)).max(256),
  resume: z.string().uuid().nullish(), inspect_models: z.boolean(), selection: SelectionSchema,
}).strict().superRefine((command, context) => {
  if (command.inspect_models && (command.prompt !== '' || command.resume != null || command.edit_paths.length !== 0 || command.operation !== 'plan')) {
    context.addIssue({ code: 'custom', message: 'Model inspection accepts no prompt, session or write scope' });
  }
});
const ModelSchema = z.object({
  value: boundedText(256), resolvedModel: boundedText(256).optional(),
  displayName: boundedText(512), description: z.string().max(4096).refine(value =>
    !/[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(value)),
  supportsEffort: z.boolean().optional(), supportedEffortLevels: z.array(EffortSchema).max(5).optional(),
});

export class SDKPolicyError extends Error {}

export function modelCatalogue(raw: ModelInfo[], selection: Selection) {
  const parsed = z.array(ModelSchema).max(256).safeParse(raw);
  if (!parsed.success) throw new SDKPolicyError('Live SDK model catalogue unsupported: invalid bounds/schema/display text');
  const models = parsed.data;
  if (new Set(models.map(model => model.value)).size !== models.length) throw new SDKPolicyError('Duplicate SDK model identifiers');
  const selected = selection.model === null ? undefined :
    models.find(model => model.value === selection.model || model.resolvedModel === selection.model);
  if (selection.model !== null && !selected) throw new SDKPolicyError('Selected model is not advertised by the live SDK backend');
  if (selection.effort !== null &&
    (!selected || selected.supportsEffort !== true || !selected.supportedEffortLevels?.includes(selection.effort))) {
    throw new SDKPolicyError('Reasoning effort unsupported: select a model with live SDK-advertised supportedEffortLevels');
  }
  return {
    models: models.map(model => ({
      id: model.value, name: model.displayName, description: model.description,
      efforts: [] as EffortLevel[],
      default_effort: null,
    })),
    current: selected?.value ?? null,
    source: `Claude Agent SDK ${SDK_VERSION} supportedModels()`,
    effort_note: models.length === 0 ? 'Model and reasoning selection unsupported: the live SDK returned no supported models.' :
      'Reasoning selection unsupported: supportedModels() may advertise effort capability, but this SDK has no public pre-prompt acknowledgement of applied effort after environment/session overrides, organization caps and model downgrades. Explicit effort is refused rather than silently changed.',
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

// This control call initializes the real SDK without releasing the prompt iterator.
// The caller may release it only after backend and model policy both succeed.
export async function prepareModels(
  sdk: { accountInfo(): Promise<{ apiProvider?: string; tokenSource?: string; apiKeySource?: string }>;
    initializationResult(): Promise<{ claude_code_version?: string }>; supportedModels(): Promise<ModelInfo[]>;
    setModel(model?: string): Promise<void>; getContextUsage(options: { detail: 'summary' }): Promise<{ model: string }> },
  selection: Selection, route: string, expectedProvider: string, inspectModels = false,
) {
  if (!accountMatches(route, await sdk.accountInfo(), expectedProvider)) throw new Error('Effective SDK authentication backend differs from reviewed route');
  if ((await sdk.initializationResult()).claude_code_version !== CLI_VERSION) throw new Error('Effective SDK executable version differs from reviewed CLI');
  const models = await sdk.supportedModels();
  const catalogue = modelCatalogue(models, selection);
  if (selection.effort !== null) {
    throw new SDKPolicyError('Reasoning selection unsupported: SDK has no public pre-prompt applied-effort acknowledgement; org caps and downgrades cannot be qualified');
  }
  let approvedModel: string | undefined;
  if (!inspectModels && selection.model !== null) {
    const selected = models.find(model => model.value === selection.model || model.resolvedModel === selection.model)!;
    approvedModel = selected.resolvedModel ?? selected.value;
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

// The pinned CLI reports no token/API-key source for a profile credential; an API key,
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
