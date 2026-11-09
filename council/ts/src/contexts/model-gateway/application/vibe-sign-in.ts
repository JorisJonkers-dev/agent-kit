import { join } from 'node:path'

import type { ProcessCommand } from '../../../ports/index.js'
import type { ModelGatewayFilesPort, ModelGatewayProcessPort } from './ports.js'

/**
 * Whether a Vibe home is signed in, checked where Vibe itself keeps its key,
 * without reading the key. Vibe stores `MISTRAL_API_KEY` in the OS keyring
 * under the service `ai.mistral.vibe`, and falls back to a line in the home's
 * `.env` when it has no keyring:
 *
 * - on macOS the keyring is the login Keychain, asked with `security
 *   find-generic-password` without `-w`, so the value is never printed;
 * - on Linux Vibe goes through Python's `keyring`, whose Secret Service
 *   backend stores an item with the attributes `service` and `username`. The
 *   check asks the Secret Service for matching item paths over D-Bus
 *   (`SearchItems`), which returns paths only, never a secret;
 * - on every platform, a non-empty `MISTRAL_API_KEY=` line in `<home>/.env`.
 *
 * A keyring that cannot be asked (no `security`, no `gdbus`, no Secret
 * Service on the bus) counts as holding nothing, so the `.env` decides.
 */
export async function vibeSignedIn(ports: VibeSignInPorts, home: string, platform: NodeJS.Platform): Promise<boolean> {
  if (await keyringHoldsKey(ports.process, platform)) return true
  const dotenv = join(home, '.env')
  if (!(await ports.files.exists(dotenv))) return false
  return (await ports.files.readText(dotenv)).split('\n').some((line) => DOTENV_KEY.test(line))
}

/** What the check needs: a process to ask the keyring with, and files to read the `.env` from. */
export interface VibeSignInPorts {
  readonly process: Pick<ModelGatewayProcessPort, 'exec'>
  readonly files: Pick<ModelGatewayFilesPort, 'exists' | 'readText'>
}

/** The keyring service and account Vibe stores its Mistral key under. */
export const VIBE_KEYRING_SERVICE = 'ai.mistral.vibe'
export const VIBE_KEYRING_ACCOUNT = 'MISTRAL_API_KEY'

/** Seconds the Secret Service may take to answer before the keyring counts as holding nothing. */
const SECRET_SERVICE_TIMEOUT_S = 5

/** A `.env` line that sets the key to something, quoted or not. */
const DOTENV_KEY = /^\s*MISTRAL_API_KEY\s*=\s*['"]?[^'"\s]/

/**
 * The command that asks this platform's keyring whether Vibe's item exists;
 * undefined where Vibe uses none. The Keychain may wait for its owner to
 * unlock it, so only the Secret Service lookup is timed.
 */
export function keyringQuery(platform: NodeJS.Platform): ProcessCommand | undefined {
  if (platform === 'darwin') {
    return { args: ['find-generic-password', '-s', VIBE_KEYRING_SERVICE, '-a', VIBE_KEYRING_ACCOUNT], command: 'security' }
  }
  if (platform === 'linux') {
    return {
      args: [
        'call',
        '--session',
        '--timeout',
        String(SECRET_SERVICE_TIMEOUT_S),
        '--dest',
        'org.freedesktop.secrets',
        '--object-path',
        '/org/freedesktop/secrets',
        '--method',
        'org.freedesktop.Secret.Service.SearchItems',
        `{'service': '${VIBE_KEYRING_SERVICE}', 'username': '${VIBE_KEYRING_ACCOUNT}'}`,
      ],
      command: 'gdbus',
      timeoutMs: (SECRET_SERVICE_TIMEOUT_S + 1) * 1000,
    }
  }
  return undefined
}

async function keyringHoldsKey(process: Pick<ModelGatewayProcessPort, 'exec'>, platform: NodeJS.Platform): Promise<boolean> {
  const query = keyringQuery(platform)
  if (query === undefined) return false
  const result = await process.exec(query).catch(() => ({ exitCode: 1, stdout: '' }))
  if (result.exitCode !== 0) return false
  // `security` exits 0 only when the item exists. `SearchItems` answers two
  // lists of item paths, unlocked and locked, such as
  // `([objectpath '/org/freedesktop/secrets/collection/login/3'], @ao [])`;
  // an item in either list exists.
  return platform === 'darwin' || /'\/[^']+'/.test(result.stdout)
}
