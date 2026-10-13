import { z } from 'zod';
import { lstatSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';

export const SDK_VERSION = '0.3.296';
export const CLI_VERSION = '2.1.296';
export const FRAME_LIMIT = 2 * 1024 * 1024;
export const READ_TOOLS = ['Read', 'Glob', 'Grep', 'AskUserQuestion'];
export const DENIED_TOOLS = ['Bash', 'Agent', 'Task', 'TaskOutput', 'TaskStop', 'NotebookEdit', 'Skill', 'WebFetch', 'WebSearch', 'ToolSearch', 'EnterWorktree', 'ExitWorktree', 'EnterPlanMode', 'ExitPlanMode', 'CronCreate', 'CronDelete', 'CronList', 'mcp__*'];

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

export function credentialRoute(env: NodeJS.ProcessEnv): string | undefined {
  const api = Boolean(env.ANTHROPIC_API_KEY);
  const bedrock = env.CLAUDE_CODE_USE_BEDROCK === '1';
  const vertex = env.CLAUDE_CODE_USE_VERTEX === '1';
  if (Number(api) + Number(bedrock) + Number(vertex) !== 1) return undefined;
  const chain = env.EEVEE_CLAUDE_CLOUD_AUTH === 'provider-chain';
  if ((env.EEVEE_CLAUDE_CLOUD_AUTH && !chain) || (chain && api)) return undefined;
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

// Interrupt acknowledgment is not a cancelled-result discriminator in this SDK.
// Preserve an authoritative success; an error after interruption remains ambiguous.
export function sdkOutcome(cancelRequested: boolean, isError: boolean, subtype: string): 'completed' | 'failed' | 'unknown' {
  if (!isError && subtype === 'success') return 'completed';
  return cancelRequested ? 'unknown' : 'failed';
}
